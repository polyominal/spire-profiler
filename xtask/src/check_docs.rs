//! The documentation gate: `cargo doc` must be warning-free, and the
//! in-house Rust comment density must stay at or below the AGENTS.md
//! budget.
//!
//! The density metric is comment lines / (comment lines + code lines)
//! over the three in-house Rust roots. A line is a comment when its first
//! non-blank characters begin `//` (doc comments `///` and `//!`
//! included) or `/*`, or when it sits inside a `/* */` block; string
//! literal contents never start a comment, and a line where a block
//! closes and code follows, or that carries a trailing comment, is code.
//! Blank lines are not counted. Exceeding the 15% budget fails the gate;
//! the budget is a crate-wide ceiling, not a per-file one.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use xshell::{Shell, cmd};

use crate::scan;

/// The AGENTS.md budget: comment+code density, not code-only.
const DENSITY_LIMIT_PERCENT: f64 = 15.0;

/// Tests included.
const RUST_ROOTS: &[&str] = &["profiler-core/src", "profiler-core/tests", "xtask/src"];

/// Doc-only files hold preserved spec content, not slashable offenders.
const MIN_CODE_LINES: usize = 20;

/// Default length of the offender report when `--top` is absent.
const DEFAULT_TOP_OFFENDERS: usize = 10;

pub fn check_docs(shell: &Shell, top_offenders: Option<usize>) -> Result<()> {
    // A rustdoc warning is a doc bug; -D warnings turns every one (broken
    // links, HTML tags, unresolved names) into a hard failure.
    cmd!(
        shell,
        "cargo doc --workspace --no-deps --document-private-items --locked"
    )
    .env("RUSTDOCFLAGS", "-D warnings")
    .run()
    .context("while attempting to run the cargo doc gate")?;

    let mut files = Vec::new();
    for root in RUST_ROOTS {
        let root = crate::workspace_root().join(root);
        collect_files(&root, &mut files)?;
    }
    let comments: usize = files.iter().map(|file| file.comments).sum();
    let code: usize = files.iter().map(|file| file.code).sum();
    let total = comments + code;
    let density = if total == 0 {
        0.0
    } else {
        density_percent(comments, code)
    };
    println!(
        "in-house Rust density: {density:.1}% ({comments} comments / {total} comment+code lines)"
    );
    // Offenders print before the failure so over-budget drift has a name.
    report_offenders(&files, top_offenders.unwrap_or(DEFAULT_TOP_OFFENDERS));
    if density > DENSITY_LIMIT_PERCENT {
        bail!("check-docs: density exceeds the {DENSITY_LIMIT_PERCENT:.0}% budget");
    }
    Ok(())
}

struct FileCount {
    path: PathBuf,
    comments: usize,
    code: usize,
}

fn collect_files(dir: &Path, out: &mut Vec<FileCount>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("while attempting to list {}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("while attempting to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("while attempting to read {}", path.display()))?;
            let (comments, code) = scan::count_lines(&content);
            out.push(FileCount {
                path,
                comments,
                code,
            });
        }
    }
    Ok(())
}

fn density_percent(comments: usize, code: usize) -> f64 {
    let total = comments + code;
    debug_assert!(total > 0);
    100.0 * comments as f64 / total as f64
}

/// Biggest comment contributors first, so drift has a name.
fn report_offenders(files: &[FileCount], top_offenders: usize) {
    let mut files: Vec<&FileCount> = files
        .iter()
        .filter(|file| file.code >= MIN_CODE_LINES)
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.comments));
    for file in files.into_iter().take(top_offenders) {
        println!(
            "  {:>4} comments / {:>4} code lines  {:>5.1}%  {}",
            file.comments,
            file.code,
            density_percent(file.comments, file.code),
            file.path.display()
        );
    }
}
