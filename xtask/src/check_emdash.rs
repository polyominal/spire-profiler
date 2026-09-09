//! The em-dash ratchet. An em dash can stand in for a comma, colon,
//! semicolon, or parentheses, so the reader must infer the clause relation
//! a specific mark would state; heavy use also reads as machine-generated.
//! In-house text therefore pins per-file U+2014 counts at hand-curated
//! ceilings that only descend: a file leaves [`PINS`] when it reaches
//! zero, and an empty table is a plain ban.
//!
//! Scope: the fmt-md doc set and xtask/src count every em dash (their
//! strings are all developer-facing), while profiler-core counts comments
//! only, because its string and char literals hold player-visible
//! typography and the renderer's glyph allowlist. Comment classification
//! reuses the per-line scanner from [`crate::scan`], so a trailing
//! comment on a code line is out of scope.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::scan::{LineKind, LineScanner};
use crate::{md, workspace_root};

const EM_DASH: char = '\u{2014}';

/// Hand-curated ceilings, one per file with a nonzero count. A pin only
/// descends: reword instead of raising, and delete the entry at zero.
const PINS: &[(&str, usize)] = &[
    ("profiler-core/src/abi.rs", 9),
    ("profiler-core/src/data.rs", 4),
    ("profiler-core/src/data/events/card.rs", 2),
    ("profiler-core/src/data/events/potion.rs", 2),
    ("profiler-core/src/data/events/run.rs", 1),
    ("profiler-core/src/data/events/tests/card.rs", 2),
    ("profiler-core/src/data/events/tests/orb_potion.rs", 1),
    ("profiler-core/src/data/ledger.rs", 4),
    ("profiler-core/src/data/persistence.rs", 6),
    ("profiler-core/src/data/persistence/time.rs", 1),
    ("profiler-core/src/data/run_history.rs", 1),
    ("profiler-core/src/data/run_history/tests.rs", 1),
    ("profiler-core/src/data/state.rs", 6),
    ("profiler-core/src/engine.rs", 4),
    ("profiler-core/src/engine/gdext.rs", 12),
    ("profiler-core/src/engine/object.rs", 2),
    ("profiler-core/src/lib.rs", 5),
    ("profiler-core/src/registration.rs", 2),
    ("profiler-core/src/ui.rs", 2),
    ("profiler-core/src/ui/chart_layout.rs", 7),
    ("profiler-core/src/ui/palette.rs", 2),
    ("profiler-core/src/ui/panel.rs", 2),
    ("profiler-core/src/ui/panel_body.rs", 3),
    ("profiler-core/src/ui/panel_common.rs", 4),
    ("profiler-core/src/ui/panel_replay.rs", 2),
    ("profiler-core/src/ui/run_panel.rs", 7),
    ("profiler-core/src/ui/theme.rs", 3),
    ("profiler-core/src/ui/tooltip.rs", 4),
    ("profiler-core/src/ui/ui_model.rs", 1),
    ("xtask/src/build.rs", 1),
    ("xtask/src/bundle.rs", 2),
    ("xtask/src/check_abi.rs", 3),
    ("xtask/src/check_catalog.rs", 10),
    ("xtask/src/check_citations.rs", 1),
    ("xtask/src/decompile.rs", 2),
    ("xtask/src/discover.rs", 3),
    ("xtask/src/game_version.rs", 1),
];

const COMMENT_ROOTS: &[&str] = &["profiler-core/src", "profiler-core/tests"];

pub fn run() -> Result<()> {
    let root = workspace_root();
    let mut counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for doc in md::DOCS {
        let path = root.join(doc);
        let content =
            fs::read_to_string(&path).with_context(|| format!("while attempting to read {doc}"))?;
        counts.insert(path, content.matches(EM_DASH).count());
    }
    for rel in COMMENT_ROOTS {
        collect(&root.join(rel), true, &mut counts)?;
    }
    collect(&root.join("xtask/src"), false, &mut counts)?;

    let counts: BTreeMap<String, usize> = counts
        .into_iter()
        .map(|(path, count)| {
            let rel = path
                .strip_prefix(root)
                .expect("every counted path was built below the workspace root")
                .to_str()
                .expect("in-scope files have UTF-8 names");
            (rel.to_owned(), count)
        })
        .collect();
    let total: usize = counts.values().sum();

    let offenses = evaluate(&counts, PINS);
    let mut failures = 0;
    for offense in &offenses {
        if offense.is_failure() {
            eprintln!("check-emdash: {}", offense.line());
            failures += 1;
        } else {
            println!("check-emdash: {}", offense.line());
        }
    }
    if failures > 0 {
        bail!("check-emdash: {failures} file(s) breach the em-dash pins");
    }
    println!(
        "check-emdash: {} files pinned, {total} em dashes",
        PINS.len()
    );
    Ok(())
}

/// Adds every `.rs` file under `dir`, keyed by absolute path.
fn collect(dir: &Path, comments_only: bool, counts: &mut BTreeMap<PathBuf, usize>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("while attempting to list {}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("while attempting to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect(&path, comments_only, counts)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("while attempting to read {}", path.display()))?;
            let count = if comments_only {
                comment_dashes(&content)
            } else {
                content.matches(EM_DASH).count()
            };
            counts.insert(path, count);
        }
    }
    Ok(())
}

fn comment_dashes(content: &str) -> usize {
    let mut scanner = LineScanner::default();
    let mut count = 0;
    for line in content.lines() {
        if matches!(scanner.classify(line), LineKind::Comment) {
            count += line.matches(EM_DASH).count();
        }
    }
    count
}

/// The pin comparison, in path order. `Lowerable` is a nudge; the rest fail.
fn evaluate(counts: &BTreeMap<String, usize>, pins: &[(&str, usize)]) -> Vec<Offense> {
    let mut offenses = Vec::new();
    for (path, &actual) in counts {
        match pins.iter().find(|(pinned, _)| pinned == path) {
            Some(&(_, pin)) if actual > pin => offenses.push(Offense::OverPin {
                path: path.clone(),
                pin,
                actual,
            }),
            Some(&(_, pin)) if actual < pin => offenses.push(Offense::Lowerable {
                path: path.clone(),
                pin,
                actual,
            }),
            Some(_) => {}
            None if actual > 0 => offenses.push(Offense::Unpinned {
                path: path.clone(),
                actual,
            }),
            None => {}
        }
    }
    for &(path, _) in pins {
        if !counts.contains_key(path) {
            offenses.push(Offense::StalePin {
                path: path.to_owned(),
            });
        }
    }
    offenses
}

#[derive(Debug, PartialEq, Eq)]
enum Offense {
    OverPin {
        path: String,
        pin: usize,
        actual: usize,
    },
    Lowerable {
        path: String,
        pin: usize,
        actual: usize,
    },
    Unpinned {
        path: String,
        actual: usize,
    },
    StalePin {
        path: String,
    },
}

impl Offense {
    fn is_failure(&self) -> bool {
        !matches!(self, Offense::Lowerable { .. })
    }

    fn line(&self) -> String {
        match self {
            Offense::OverPin { path, pin, actual } => format!(
                "ERROR: {path}: {actual} em dashes exceed the pin of {pin}: reword without \
                 the em dash"
            ),
            Offense::Unpinned { path, actual } => format!(
                "ERROR: {path}: {actual} em dashes with no pin: reword without the em dash, \
                 or add a pin"
            ),
            Offense::StalePin { path } => {
                format!("ERROR: {path}: pinned but not in scope (renamed or deleted): drop it")
            }
            Offense::Lowerable { path, pin, actual } => {
                format!("{path}: pin can be lowered from {pin} to {actual}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures escape the dash as \u{2014} because xtask/src is scanned
    // whole-file: a literal here would feed the ratchet.
    const D: &str = "\u{2014}";

    fn counts(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
        pairs
            .iter()
            .map(|&(path, n)| (path.to_owned(), n))
            .collect()
    }

    #[test]
    fn comments_count_strings_and_char_literals_do_not() {
        let src = format!(
            "//! doc {D}\n\
             /// item {D}\n\
             // line {D}\n\
             /* open {D}\n\
             inside {D} */\n\
             let s = \"{D}\";\n\
             let c = '{D}';\n\
             let t = \"// {D}\";\n\
             let u = 1; // trailing {D}\n"
        );
        assert_eq!(comment_dashes(&src), 5);
        assert_eq!(src.matches(EM_DASH).count(), 9);
    }

    #[test]
    fn above_pin_fails() {
        let offenses = evaluate(&counts(&[("docs/a.md", 3)]), &[("docs/a.md", 2)]);
        assert_eq!(
            offenses.as_slice(),
            &[Offense::OverPin {
                path: "docs/a.md".into(),
                pin: 2,
                actual: 3
            }]
        );
        assert!(offenses[0].is_failure());
    }

    #[test]
    fn unpinned_dash_fails() {
        let offenses = evaluate(&counts(&[("src/b.rs", 1)]), &[]);
        assert_eq!(
            offenses.as_slice(),
            &[Offense::Unpinned {
                path: "src/b.rs".into(),
                actual: 1
            }]
        );
    }

    #[test]
    fn at_pin_passes_quietly() {
        assert!(evaluate(&counts(&[("e.rs", 2)]), &[("e.rs", 2)]).is_empty());
    }

    #[test]
    fn below_pin_only_nudges() {
        let offenses = evaluate(&counts(&[("c.rs", 1)]), &[("c.rs", 3)]);
        assert_eq!(
            offenses.as_slice(),
            &[Offense::Lowerable {
                path: "c.rs".into(),
                pin: 3,
                actual: 1
            }]
        );
        assert!(!offenses[0].is_failure());
    }

    #[test]
    fn zero_needs_no_pin_and_stale_pin_fails() {
        assert!(evaluate(&counts(&[("d.rs", 0)]), &[]).is_empty());
        let offenses = evaluate(&counts(&[]), &[("gone.rs", 2)]);
        assert_eq!(
            offenses.as_slice(),
            &[Offense::StalePin {
                path: "gone.rs".into()
            }]
        );
    }
}
