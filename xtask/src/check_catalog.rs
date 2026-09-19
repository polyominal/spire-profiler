//! `cargo xtask check-catalog`: verify the attribution catalog against the
//! decompiled game source. Requires the `cargo xtask decompile` output in
//! `tmp/sts2-decompiled`, with a provenance recording the pinned game
//! version.
//!
//! Fails on entries that no longer resolve or no longer match their review
//! decision: the shim would skip the former and the latter needs a fresh
//! reading of the decompiled body. New uncatalogued hooks also fail until
//! they are catalogued or recorded as reviewed exclusions. Inclusion stays a
//! human judgment: this is a report over decompiled syntax, not a semantic
//! C# analysis or a generator.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

use crate::{catalog, decompile, game_version, workspace_root};

/// Effect statements that require source review, including card generation
/// because generated instances inherit their suppliers. Block loss is absent
/// because it is not a recorded event.
const TRACKED: &[(&str, &str)] = &[
    ("damage", r"CreatureCmd\.Damage|DamageCommand|DealDamage"),
    ("block", r"GainBlock"),
    ("forge", r"ForgeCmd\.Forge"),
    ("orb", r"OrbCmd\.(?:Channel|EvokeLast|Passive)"),
    ("osty", r"OstyCmd"),
    ("power", r"PowerCmd\.Apply(?:<|\()"),
    (
        "cardgen",
        r"AddGeneratedCardToCombat|AddGeneratedCardsToCombat|AddToCombatAndPreview|CreateInHand|CardCmd\.Transform",
    ),
];

/// Files whose public virtual methods form the hook universe.
const BASE_MODELS: [&str; 3] = ["AbstractModel.cs", "RelicModel.cs", "PowerModel.cs"];

/// Every declaration line: access modifier, optional `override`/`async`,
/// return type, then the name right before `(`.
const DECL_RE: &str = r"(?m)^[ \t]*(?:public|protected|private|internal)\s+(?:(override)\s+|async\s+)?[\w<>,.\[\]? ]+?\s+(\w+)\s*\(";

/// Identifiers followed by `(`: the candidate helper calls in a body.
const CALL_RE: &str = r"\b([A-Za-z_]\w*)\s*\(";

const NAMESPACE_RE: &str = r"(?m)^namespace\s+([A-Za-z_][\w.]*)\s*(?:;|\{)";

const HOOK_DECL_RE: &str =
    r"(?m)^[ \t]*public virtual\s+(?:async\s+)?[\w<>,.\[\]? ]+?\s+(\w+)\s*\(";

struct ClassFile {
    /// Every declared method with its brace-matched body: existence checks
    /// and one level of private-helper following.
    methods: Vec<Method>,
}

struct Method {
    name: String,
    body: String,
    is_override: bool,
}

fn tracked_regexes() -> Vec<(&'static str, Regex)> {
    TRACKED
        .iter()
        .map(|(label, pattern)| {
            (
                *label,
                Regex::new(pattern).expect("tracked-effect patterns are static literals"),
            )
        })
        .collect()
}

pub fn run() -> Result<()> {
    let tree = decompile::default_output_dir(workspace_root());
    if !tree.join("project.godot").is_file() {
        bail!(
            "no decompiled source at {} — run `cargo xtask decompile` first",
            tree.display()
        );
    }
    let version = provenance_version(&tree)?;
    if version != game_version::PIN {
        bail!(
            "the decompiled tree at {} is from game {version}, not the pinned {} — re-run \
             `cargo xtask decompile`",
            tree.display(),
            game_version::PIN
        );
    }
    println!("check-catalog: {} (game {version})", tree.display());

    let models = tree.join("src/Core/Models");
    let universe = hook_universe(&models)?;
    let tracked = tracked_regexes();
    let relic_files = class_files(&models.join("Relics"), "MegaCrit.Sts2.Core.Models.Relics")?;
    let power_files = class_files(&models.join("Powers"), "MegaCrit.Sts2.Core.Models.Powers")?;
    let mut review = Review::default();

    review.check_entries(
        &catalog::RELICS,
        &relic_files,
        "Relics",
        &universe,
        &tracked,
    );
    review.check_entries(
        &catalog::POWERS,
        &power_files,
        "Powers",
        &universe,
        &tracked,
    );
    review.check_pattern_health([&relic_files, &power_files], &tracked);
    let candidates = candidate_hooks(&relic_files, &power_files, &universe, &tracked);
    review.compare_candidates(&candidates);
    review.report()
}

#[derive(Default)]
struct Review {
    failures: Vec<String>,
    seen_non_hooks: HashSet<(&'static str, &'static str, &'static str)>,
}

impl Review {
    /// A catalogued entry must resolve to its reviewed method shape in the
    /// namespace's class file; failures are the shim's runtime skips.
    fn check_entries(
        &mut self,
        entries: &[(&'static str, &'static str)],
        files: &BTreeMap<String, ClassFile>,
        ns: &'static str,
        universe: &HashSet<String>,
        tracked: &[(&'static str, Regex)],
    ) {
        for (class, method) in entries {
            let Some(file) = files.get(*class) else {
                self.failures
                    .push(format!("{ns}: {class} no longer exists"));
                continue;
            };
            let mut declarations = file
                .methods
                .iter()
                .filter(|declaration| declaration.name == *method);
            let (Some(declaration), None) = (declarations.next(), declarations.next()) else {
                self.failures.push(format!(
                    "{ns}: {class}.{method} is missing or overloaded — moved, renamed, or \
                     ambiguous?"
                ));
                continue;
            };
            if !declaration.is_override {
                self.check_non_hook(ns, class, method);
            } else if !universe.contains(*method) {
                self.failures.push(format!(
                    "{ns}: {class}.{method} overrides a method outside the hook universe — new \
                     hook type?"
                ));
            } else if declaration.effects(&file.methods, tracked).is_empty() {
                self.failures.push(format!(
                    "{ns}: {class}.{method} shows no tracked effect (body or private helpers) — \
                     bookkeeping wrap or stale entry?"
                ));
            }
        }
    }

    fn check_non_hook(&mut self, ns: &'static str, class: &'static str, method: &'static str) {
        let key = (ns, class, method);
        if catalog::NON_HOOK_ENTRIES.contains(&key) {
            self.seen_non_hooks.insert(key);
        } else {
            self.failures.push(format!(
                "{ns}: {class}.{method} wraps a non-hook method — record the exception or use \
                 the real hook"
            ));
        }
    }

    fn check_pattern_health(
        &mut self,
        files: [&BTreeMap<String, ClassFile>; 2],
        tracked: &[(&'static str, Regex)],
    ) {
        let mut matched = vec![false; tracked.len()];
        for file in files.iter().flat_map(|files| files.values()) {
            for body in file.methods.iter().map(|method| &method.body) {
                for (seen, (_, pattern)) in matched.iter_mut().zip(tracked) {
                    *seen |= pattern.is_match(body);
                }
            }
        }
        for ((label, _), matched) in tracked.iter().zip(matched) {
            if !matched {
                self.failures.push(format!(
                    "tracked-effect pattern `{label}` matches no method body — update TRACKED"
                ));
            }
        }
    }

    fn compare_candidates(&mut self, candidates: &[Candidate<'_>]) {
        let reviewed: HashSet<_> = catalog::REVIEWED_CANDIDATES.iter().copied().collect();
        if reviewed.len() != catalog::REVIEWED_CANDIDATES.len() {
            self.failures
                .push("duplicate reviewed-candidate entries".to_owned());
            return;
        }
        let candidate_keys: HashSet<_> = candidates
            .iter()
            .map(|candidate| (candidate.namespace, candidate.class, candidate.method))
            .collect();
        let new_candidates = candidates.iter().filter(|candidate| {
            !reviewed.contains(&(candidate.namespace, candidate.class, candidate.method))
        });
        for candidate in new_candidates {
            println!("new candidate: {candidate}");
            self.failures.push(format!(
                "new candidate {candidate} — catalog it or record the exclusion"
            ));
        }
        self.failures.extend(
            reviewed
                .difference(&candidate_keys)
                .map(|(ns, class, method)| {
                    format!("reviewed candidate {ns}.{class}.{method} is no longer reported")
                }),
        );
        self.compare_non_hooks();
    }

    fn compare_non_hooks(&mut self) {
        let expected: HashSet<_> = catalog::NON_HOOK_ENTRIES.iter().copied().collect();
        if expected.len() != catalog::NON_HOOK_ENTRIES.len() {
            self.failures.push("duplicate non-hook entries".to_owned());
            return;
        }
        if self.seen_non_hooks == expected {
            return;
        }
        self.failures
            .extend(
                expected
                    .difference(&self.seen_non_hooks)
                    .map(|(ns, class, method)| {
                        format!("non-hook entry {ns}.{class}.{method} is missing or became a hook")
                    }),
            );
        self.failures.extend(
            self.seen_non_hooks
                .difference(&expected)
                .map(|(ns, class, method)| {
                    format!("unexpected non-hook entry {ns}.{class}.{method}")
                }),
        );
    }

    fn report(self) -> Result<()> {
        let total = catalog::RELICS.len() + catalog::POWERS.len();
        println!(
            "catalog: {total} entries, {} reviewed candidates, {} failures",
            catalog::REVIEWED_CANDIDATES.len(),
            self.failures.len()
        );
        if self.failures.is_empty() {
            return Ok(());
        }
        for failure in &self.failures {
            eprintln!("fail: {failure}");
        }
        let count = self.failures.len();
        bail!("{count} catalog checks failed")
    }
}

/// Uncatalogued hooks whose bodies produce tracked effects. The reviewed
/// list decides whether each report is expected drift signal.
fn candidate_hooks<'a>(
    relic_files: &'a BTreeMap<String, ClassFile>,
    power_files: &'a BTreeMap<String, ClassFile>,
    universe: &HashSet<String>,
    tracked: &[(&'static str, Regex)],
) -> Vec<Candidate<'a>> {
    let catalogued: HashSet<(&str, &str)> = catalog::RELICS
        .iter()
        .copied()
        .chain(catalog::POWERS.iter().copied())
        .collect();
    let mut candidates = Vec::new();
    for (label, files) in [("Relics", relic_files), ("Powers", power_files)] {
        for (class, file) in files {
            for method in file.methods.iter().filter(|method| method.is_override) {
                if !universe.contains(&method.name)
                    || catalogued.contains(&(class.as_str(), method.name.as_str()))
                {
                    continue;
                }
                let effects = method.effects(&file.methods, tracked);
                if !effects.is_empty() {
                    candidates.push(Candidate {
                        namespace: label,
                        class,
                        method: &method.name,
                        effects,
                    });
                }
            }
        }
    }
    candidates.sort_by_key(|candidate| (candidate.namespace, candidate.class, candidate.method));
    candidates
}

struct Candidate<'a> {
    namespace: &'static str,
    class: &'a str,
    method: &'a str,
    effects: Vec<&'static str>,
}

impl std::fmt::Display for Candidate<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{} [{}]",
            self.namespace,
            self.class,
            self.method,
            self.effects.join(", ")
        )
    }
}

impl Method {
    /// TRACKED order, including one level of helper calls (Poison's Trigger).
    fn effects(&self, methods: &[Method], tracked: &[(&'static str, Regex)]) -> Vec<&'static str> {
        static CALLS: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(CALL_RE).expect("the call pattern is a static literal"));
        let mut bodies = vec![self.body.as_str()];
        for call in CALLS.captures_iter(&self.body) {
            bodies.extend(
                methods
                    .iter()
                    .filter(|method| method.name == call[1])
                    .map(|method| method.body.as_str()),
            );
        }
        tracked
            .iter()
            .filter(|(_, re)| bodies.iter().any(|body| re.is_match(body)))
            .map(|(label, _)| *label)
            .collect()
    }
}

fn hook_universe(models: &Path) -> Result<HashSet<String>> {
    let re = Regex::new(HOOK_DECL_RE).expect("the hook-universe pattern is a static literal");
    let mut universe = HashSet::new();
    for name in BASE_MODELS {
        let path = models.join(name);
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        for capture in re.captures_iter(&text) {
            universe.insert(capture[1].to_owned());
        }
    }
    Ok(universe)
}

/// Production classes can gain subdirectories; every `Mocks` tree is test
/// support rather than a game hook.
fn class_files(dir: &Path, expected_namespace: &str) -> Result<BTreeMap<String, ClassFile>> {
    let mut files = BTreeMap::new();
    let mut directories = vec![dir.to_path_buf()];
    while let Some(current) = directories.pop() {
        for entry in
            fs::read_dir(&current).with_context(|| format!("listing {}", current.display()))?
        {
            let path = entry
                .with_context(|| format!("reading an entry of {}", current.display()))?
                .path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "Mocks") {
                    continue;
                }
                directories.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "cs") {
                continue;
            }
            let text =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let Some((class, file)) = parse_class_file(&path, &text, expected_namespace)? else {
                continue;
            };
            if files.contains_key(&class) {
                bail!("duplicate class {class}");
            }
            files.insert(class, file);
        }
    }
    Ok(files)
}

/// One decompiled class file: its name, hook overrides, and every method
/// with its brace-matched body; a file with no class declaration is not a
/// class (None). Namespace drift bails: a moved class would parse cleanly
/// and then fail every catalog lookup.
fn parse_class_file(
    path: &Path,
    text: &str,
    expected_namespace: &str,
) -> Result<Option<(String, ClassFile)>> {
    static CLASS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^public (?:sealed |abstract )?class\s+(\w+)")
            .expect("the class pattern is a static literal")
    });
    static NAMESPACE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(NAMESPACE_RE).expect("the namespace pattern is a static literal")
    });
    static DECLARATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(DECL_RE).expect("the declaration pattern is a static literal"));
    let Some(class) = CLASS.captures(text).map(|capture| capture[1].to_owned()) else {
        return Ok(None);
    };
    let namespace = NAMESPACE
        .captures(text)
        .and_then(|capture| capture.get(1))
        .map(|namespace| namespace.as_str())
        .unwrap_or_default();
    if namespace != expected_namespace {
        bail!(
            "{} declares namespace {namespace:?}, expected {expected_namespace:?}",
            path.display()
        );
    }
    let mut methods = Vec::new();
    for capture in DECLARATION.captures_iter(text) {
        let name = capture[2].to_owned();
        let body = brace_body(
            text,
            capture
                .get(0)
                .expect("every regex capture has the whole match")
                .end(),
        )
        .with_context(|| {
            format!(
                "{}: {class}.{name}: no block body — unsupported decompiler output",
                path.display()
            )
        })?
        .to_owned();
        methods.push(Method {
            name,
            body,
            is_override: capture.get(1).is_some(),
        });
    }
    Ok(Some((class, ClassFile { methods })))
}

/// The brace-matched block opening after `from`; decompiled methods always
/// use block bodies, so the first `{` is the body's and a `;` before it
/// means a bodyless declaration. The count is not literal-aware: a brace
/// inside a string or char literal would unbalance it.
fn brace_body(text: &str, from: usize) -> Option<&str> {
    let brace = text[from..].find('{')? + from;
    if text[from..brace].contains(';') {
        return None;
    }
    let mut depth = 0usize;
    for (index, character) in text[brace..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[brace..brace + index + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The pinned game version the tree was decompiled from; a tree produced
/// before the field existed is rejected with the remedy.
fn provenance_version(tree: &Path) -> Result<String> {
    let path = tree.join(".provenance.json");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    value
        .get("game_version")
        .and_then(|version| version.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no \"game_version\" — re-run `cargo xtask decompile`",
                path.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_universe_accepts_non_task_return_types() {
        let shell = xshell::Shell::new().expect("cargo test runs with a working directory");
        let temp = shell
            .create_temp_dir()
            .expect("the test model tree needs an isolated temporary directory");
        let models = temp.path().join("models");
        std::fs::create_dir_all(&models).expect("creating the test model tree");
        for name in BASE_MODELS {
            std::fs::write(
                models.join(name),
                "public virtual ValueTask<int> NewHook();\n",
            )
            .expect("writing the test model");
        }

        let universe = hook_universe(&models).expect("reading the test models");
        assert!(universe.contains("NewHook"));
    }

    #[test]
    fn class_files_reject_namespace_drift() {
        let shell = xshell::Shell::new().expect("cargo test runs with a working directory");
        let temp = shell
            .create_temp_dir()
            .expect("the test class tree needs an isolated temporary directory");
        let root = temp.path();
        std::fs::write(
            root.join("Moved.cs"),
            "namespace MegaCrit.Sts2.Core.Moved;\npublic sealed class Moved {}\n",
        )
        .expect("writing the test class");

        let error = match class_files(root, "MegaCrit.Sts2.Core.Models.Relics") {
            Ok(files) => panic!("namespace drift passed with {} classes", files.len()),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("expected \"MegaCrit.Sts2.Core.Models.Relics\"")
        );
    }

    #[test]
    fn card_generation_follows_the_game_hooks_not_object_creation() {
        let file = ClassFile {
            methods: vec![
                Method {
                    name: "AfterPlayerTurnStart".to_owned(),
                    body: r#"CardCmd.TransformToRandom(card, rng);"#.to_owned(),
                    is_override: true,
                },
                Method {
                    name: "AfterObtained".to_owned(),
                    body: r#"RunState.CreateCard<Apotheosis>(owner); CardPileCmd.Add(card, PileType.Deck);"#
                        .to_owned(),
                    is_override: false,
                },
            ],
        };
        let tracked = tracked_regexes();

        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked),
            ["cardgen"]
        );
        assert!(file.methods[1].effects(&file.methods, &tracked).is_empty());
    }

    #[test]
    fn power_application_covers_generic_and_plain_apply() {
        let file = ClassFile {
            methods: vec![
                Method {
                    name: "Generic".to_owned(),
                    body: "await PowerCmd.Apply<StrengthPower>(ctx, target, 1);".to_owned(),
                    is_override: false,
                },
                Method {
                    name: "Plain".to_owned(),
                    body: "await PowerCmd.Apply(ctx, power, target, 1);".to_owned(),
                    is_override: false,
                },
            ],
        };
        let tracked = tracked_regexes();

        assert_eq!(file.methods[0].effects(&file.methods, &tracked), ["power"]);
        assert_eq!(file.methods[1].effects(&file.methods, &tracked), ["power"]);
    }

    #[test]
    fn effect_detection_follows_one_helper_level() {
        let file = ClassFile {
            methods: vec![
                Method {
                    name: "AfterSideTurnStart".to_owned(),
                    body: "await Trigger();".to_owned(),
                    is_override: true,
                },
                Method {
                    name: "Trigger".to_owned(),
                    body: "await CreatureCmd.Damage(ctx, target, 1);".to_owned(),
                    is_override: false,
                },
            ],
        };

        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked_regexes()),
            ["damage"]
        );
    }

    #[test]
    fn candidate_review_fails_on_an_unreviewed_hook() {
        let mut review = Review::default();
        review.check_non_hook(
            catalog::NON_HOOK_ENTRIES[0].0,
            catalog::NON_HOOK_ENTRIES[0].1,
            catalog::NON_HOOK_ENTRIES[0].2,
        );
        let mut candidates: Vec<Candidate> = catalog::REVIEWED_CANDIDATES
            .iter()
            .map(|(namespace, class, method)| Candidate {
                namespace,
                class,
                method,
                effects: vec!["damage"],
            })
            .collect();
        candidates.push(Candidate {
            namespace: "Powers",
            class: "NewPower",
            method: "AfterSideTurnStart",
            effects: vec!["power"],
        });

        review.compare_candidates(&candidates);

        assert_eq!(review.failures.len(), 1);
        assert!(review.failures[0].contains("NewPower.AfterSideTurnStart"));
    }

    #[test]
    fn parsed_overloads_keep_their_own_bodies_and_catalog_entries_require_one_method() {
        let source = "namespace MegaCrit.Sts2.Core.Models.Powers;\n\
                      public sealed class NewPower\n{\n\
                          public override void AfterSideTurnStart() {}\n\
                          public override void AfterSideTurnStart(int amount) { PowerCmd.Apply(amount); }\n\
                          private void Helper() { CreatureCmd.Damage(); }\n}\n";
        let (name, file) = parse_class_file(
            Path::new("NewPower.cs"),
            source,
            "MegaCrit.Sts2.Core.Models.Powers",
        )
        .expect("fixture is a complete class")
        .expect("fixture declares NewPower");
        let files = BTreeMap::from([(name, file)]);
        let universe = HashSet::from(["AfterSideTurnStart".to_owned()]);
        let tracked = tracked_regexes();
        let relics = BTreeMap::new();
        let candidates = candidate_hooks(&relics, &files, &universe, &tracked);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].method, "AfterSideTurnStart");
        assert_eq!(candidates[0].effects, ["power"]);

        let mut review = Review::default();
        review.check_entries(
            &[("NewPower", "AfterSideTurnStart"), ("NewPower", "Missing")],
            &files,
            "Powers",
            &universe,
            &tracked,
        );
        assert_eq!(review.failures.len(), 2);
        assert!(review.failures[0].contains("AfterSideTurnStart is missing or overloaded"));
        assert!(review.failures[1].contains("Missing is missing or overloaded"));
    }

    #[test]
    fn effect_detection_unions_direct_and_helper_effects() {
        let file = ClassFile {
            methods: vec![
                Method {
                    name: "AfterSideTurnStart".to_owned(),
                    body: "await CreatureCmd.Damage(ctx, target, 1); await Buff();".to_owned(),
                    is_override: true,
                },
                Method {
                    name: "Buff".to_owned(),
                    body: "await PowerCmd.Apply<StrengthPower>(ctx, target, 1);".to_owned(),
                    is_override: false,
                },
            ],
        };

        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked_regexes()),
            ["damage", "power"]
        );
    }

    #[test]
    fn class_files_reject_bodyless_declarations() {
        let shell = xshell::Shell::new().expect("cargo test runs with a working directory");
        let temp = shell
            .create_temp_dir()
            .expect("the test class tree needs an isolated temporary directory");
        let root = temp.path();
        std::fs::write(
            root.join("Empty.cs"),
            "namespace MegaCrit.Sts2.Core.Models.Relics;\npublic sealed class Empty\n{\n    public void Hook();\n}\n",
        )
        .expect("writing the test class");

        let error = match class_files(root, "MegaCrit.Sts2.Core.Models.Relics") {
            Ok(files) => panic!("a bodyless declaration passed with {} classes", files.len()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("no block body"));
    }

    #[test]
    fn brace_body_rejects_bodyless_declarations() {
        let source = "void Hook();\nvoid Next() { Damage(); }";
        let declaration_end =
            source.find("Hook()").expect("the test declares Hook") + "Hook()".len();

        assert_eq!(brace_body(source, declaration_end), None);
    }

    #[test]
    fn brace_body_includes_nested_blocks() {
        let source = "void Hook() { if (live) { Damage(); } } void Next() {}";
        let declaration_end =
            source.find("Hook()").expect("the test declares Hook") + "Hook()".len();

        assert_eq!(
            brace_body(source, declaration_end),
            Some("{ if (live) { Damage(); } }")
        );
    }
}
