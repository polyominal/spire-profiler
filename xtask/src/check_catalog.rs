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

use anyhow::{Context, Result, bail};
use regex::Regex;

use crate::{catalog, decompile, game_version, workspace_root};

/// Effect statements the core tracks; the catalog exists to attribute them.
/// Card generation counts: a generated card's plays credit its generator,
/// so generation hooks are wrapped like any other effect. Block loss is
/// absent on purpose: the shim never forwards it.
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
    /// Hook overrides only: the candidate scan walks these.
    overrides: Vec<String>,
    /// Every declared method with its brace-matched body: existence checks
    /// and one level of private-helper following.
    methods: Vec<(String, String)>,
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
    seen_non_hooks: HashSet<String>,
}

impl Review {
    /// A catalogued entry must resolve to its reviewed method shape in the
    /// namespace's class file; failures are the shim's runtime skips.
    fn check_entries(
        &mut self,
        entries: &[(&str, &str)],
        files: &BTreeMap<String, ClassFile>,
        ns: &str,
        universe: &HashSet<String>,
        tracked: &[(&'static str, Regex)],
    ) {
        for (class, method) in entries {
            let Some(file) = files.get(*class) else {
                self.failures
                    .push(format!("{ns}: {class} no longer exists"));
                continue;
            };
            if file
                .methods
                .iter()
                .filter(|(name, _)| name == method)
                .count()
                != 1
            {
                self.failures.push(format!(
                    "{ns}: {class}.{method} is missing or overloaded — moved, renamed, or \
                     ambiguous?"
                ));
                continue;
            }
            if !file.overrides.iter().any(|name| name == method) {
                self.check_non_hook(ns, class, method);
            } else if !universe.contains(*method) {
                self.failures.push(format!(
                    "{ns}: {class}.{method} overrides a method outside the hook universe — new \
                     hook type?"
                ));
            } else if effects_in(file, method, tracked).is_empty() {
                self.failures.push(format!(
                    "{ns}: {class}.{method} shows no tracked effect (body or private helpers) — \
                     bookkeeping wrap or stale entry?"
                ));
            }
        }
    }

    fn check_non_hook(&mut self, ns: &str, class: &str, method: &str) {
        let key = format!("{ns}.{class}.{method}");
        if catalog::NON_HOOK_ENTRIES
            .iter()
            .any(|entry| format!("{}.{}.{}", entry.0, entry.1, entry.2) == key)
        {
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
            for body in file.methods.iter().map(|(_, body)| body) {
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

    fn compare_candidates(&mut self, candidates: &[Candidate]) {
        let reviewed: HashSet<String> = catalog::REVIEWED_CANDIDATES
            .iter()
            .map(|(namespace, class, method)| format!("{namespace}.{class}.{method}"))
            .collect();
        if reviewed.len() != catalog::REVIEWED_CANDIDATES.len() {
            self.failures
                .push("duplicate reviewed-candidate entries".to_owned());
            return;
        }
        let candidate_keys: HashSet<String> = candidates.iter().map(Candidate::key).collect();
        let new_candidates: Vec<&Candidate> = candidates
            .iter()
            .filter(|candidate| !reviewed.contains(&candidate.key()))
            .collect();
        for candidate in &new_candidates {
            println!("new candidate: {candidate}");
            self.failures.push(format!(
                "new candidate {candidate} — catalog it or record the exclusion"
            ));
        }
        self.failures.extend(
            reviewed
                .difference(&candidate_keys)
                .map(|review| format!("reviewed candidate {review} is no longer reported")),
        );
        self.compare_non_hooks();
    }

    fn compare_non_hooks(&mut self) {
        let expected: HashSet<String> = catalog::NON_HOOK_ENTRIES
            .iter()
            .map(|(namespace, class, method)| format!("{namespace}.{class}.{method}"))
            .collect();
        if expected.len() != catalog::NON_HOOK_ENTRIES.len() {
            self.failures.push("duplicate non-hook entries".to_owned());
            return;
        }
        if self.seen_non_hooks == expected {
            return;
        }
        self.failures.extend(
            expected
                .difference(&self.seen_non_hooks)
                .map(|entry| format!("non-hook entry {entry} is missing or became a hook")),
        );
        self.failures.extend(
            self.seen_non_hooks
                .difference(&expected)
                .map(|entry| format!("unexpected non-hook entry {entry}")),
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
fn candidate_hooks(
    relic_files: &BTreeMap<String, ClassFile>,
    power_files: &BTreeMap<String, ClassFile>,
    universe: &HashSet<String>,
    tracked: &[(&'static str, Regex)],
) -> Vec<Candidate> {
    let catalogued: HashSet<(&str, &str)> = catalog::RELICS
        .iter()
        .copied()
        .chain(catalog::POWERS.iter().copied())
        .collect();
    let mut candidates = Vec::new();
    for (label, files) in [("Relics", relic_files), ("Powers", power_files)] {
        for (class, file) in files {
            for method in &file.overrides {
                if !universe.contains(method)
                    || catalogued.contains(&(class.as_str(), method.as_str()))
                {
                    continue;
                }
                let effects = effects_in(file, method, tracked);
                if !effects.is_empty() {
                    candidates.push(Candidate {
                        namespace: label,
                        class: class.clone(),
                        method: method.clone(),
                        effects,
                    });
                }
            }
        }
    }
    candidates.sort_by_key(Candidate::key);
    candidates
}

struct Candidate {
    namespace: &'static str,
    class: String,
    method: String,
    effects: Vec<&'static str>,
}

impl Candidate {
    fn key(&self) -> String {
        format!("{}.{}.{}", self.namespace, self.class, self.method)
    }
}

impl std::fmt::Display for Candidate {
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

/// The tracked-effect buckets a hook can produce, unioned in TRACKED order
/// over its body and one level of private-helper indirection
/// (PoisonPower's Trigger, Bound Phylactery's SummonPet, ...).
fn effects_in(
    file: &ClassFile,
    method: &str,
    tracked: &[(&'static str, Regex)],
) -> Vec<&'static str> {
    let Some((_, body)) = file.methods.iter().find(|(name, _)| name == method) else {
        return Vec::new();
    };
    let mut bodies = vec![body.as_str()];
    let calls = Regex::new(CALL_RE).expect("the call pattern is a static literal");
    for call in calls.captures_iter(body) {
        bodies.extend(
            file.methods
                .iter()
                .filter(|(name, _)| name == &call[1])
                .map(|(_, helper)| helper.as_str()),
        );
    }
    tracked
        .iter()
        .filter(|(_, re)| bodies.iter().any(|body| re.is_match(body)))
        .map(|(label, _)| *label)
        .collect()
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
    let class_re = Regex::new(r"(?m)^public (?:sealed |abstract )?class\s+(\w+)")
        .expect("the class pattern is a static literal");
    let namespace_re = Regex::new(NAMESPACE_RE).expect("the namespace pattern is a static literal");
    let decl_re = Regex::new(DECL_RE).expect("the declaration pattern is a static literal");
    let Some(class) = class_re.captures(text).map(|capture| capture[1].to_owned()) else {
        return Ok(None);
    };
    let namespace = namespace_re
        .captures(text)
        .map(|capture| capture[1].to_owned())
        .unwrap_or_default();
    if namespace != expected_namespace {
        bail!(
            "{} declares namespace {namespace:?}, expected {expected_namespace:?}",
            path.display()
        );
    }
    let mut overrides = Vec::new();
    let mut methods = Vec::new();
    for capture in decl_re.captures_iter(text) {
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
        methods.push((name, body));
        if capture.get(1).is_some() {
            overrides.push(capture[2].to_owned());
        }
    }
    Ok(Some((class, ClassFile { overrides, methods })))
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
        let root = std::env::temp_dir().join("spire-catalog-hook-universe");
        let models = root.join("models");
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
        std::fs::remove_dir_all(root).expect("removing the test model tree");
    }

    #[test]
    fn class_files_reject_namespace_drift() {
        let root = std::env::temp_dir().join("spire-catalog-namespace");
        std::fs::create_dir_all(&root).expect("creating the test class tree");
        std::fs::write(
            root.join("Moved.cs"),
            "namespace MegaCrit.Sts2.Core.Moved;\npublic sealed class Moved {}\n",
        )
        .expect("writing the test class");

        let error = match class_files(&root, "MegaCrit.Sts2.Core.Models.Relics") {
            Ok(files) => panic!("namespace drift passed with {} classes", files.len()),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("expected \"MegaCrit.Sts2.Core.Models.Relics\"")
        );
        std::fs::remove_dir_all(root).expect("removing the test class tree");
    }

    #[test]
    fn card_generation_follows_the_game_hooks_not_object_creation() {
        let file = ClassFile {
            overrides: vec!["AfterPlayerTurnStart".to_owned()],
            methods: vec![
                (
                    "AfterPlayerTurnStart".to_owned(),
                    r#"CardCmd.TransformToRandom(card, rng);"#.to_owned(),
                ),
                (
                    "AfterObtained".to_owned(),
                    r#"RunState.CreateCard<Apotheosis>(owner); CardPileCmd.Add(card, PileType.Deck);"#
                        .to_owned(),
                ),
            ],
        };
        let tracked = tracked_regexes();

        assert_eq!(
            effects_in(&file, "AfterPlayerTurnStart", &tracked),
            vec!["cardgen"]
        );
        assert_eq!(
            effects_in(&file, "AfterObtained", &tracked),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn power_application_covers_generic_and_plain_apply() {
        let file = ClassFile {
            overrides: Vec::new(),
            methods: vec![
                (
                    "Generic".to_owned(),
                    "await PowerCmd.Apply<StrengthPower>(ctx, target, 1);".to_owned(),
                ),
                (
                    "Plain".to_owned(),
                    "await PowerCmd.Apply(ctx, power, target, 1);".to_owned(),
                ),
            ],
        };
        let tracked = tracked_regexes();

        assert_eq!(effects_in(&file, "Generic", &tracked), vec!["power"]);
        assert_eq!(effects_in(&file, "Plain", &tracked), vec!["power"]);
    }

    #[test]
    fn effect_detection_follows_one_helper_level() {
        let file = ClassFile {
            overrides: vec!["AfterSideTurnStart".to_owned()],
            methods: vec![
                (
                    "AfterSideTurnStart".to_owned(),
                    "await Trigger();".to_owned(),
                ),
                (
                    "Trigger".to_owned(),
                    "await CreatureCmd.Damage(ctx, target, 1);".to_owned(),
                ),
            ],
        };

        assert_eq!(
            effects_in(&file, "AfterSideTurnStart", &tracked_regexes()),
            vec!["damage"]
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
                class: class.to_string(),
                method: method.to_string(),
                effects: vec!["damage"],
            })
            .collect();
        candidates.push(Candidate {
            namespace: "Powers",
            class: "NewPower".to_owned(),
            method: "AfterSideTurnStart".to_owned(),
            effects: vec!["power"],
        });

        review.compare_candidates(&candidates);

        assert_eq!(review.failures.len(), 1);
        assert!(review.failures[0].contains("NewPower.AfterSideTurnStart"));
    }

    #[test]
    fn effect_detection_unions_direct_and_helper_effects() {
        let file = ClassFile {
            overrides: vec!["AfterSideTurnStart".to_owned()],
            methods: vec![
                (
                    "AfterSideTurnStart".to_owned(),
                    "await CreatureCmd.Damage(ctx, target, 1); await Buff();".to_owned(),
                ),
                (
                    "Buff".to_owned(),
                    "await PowerCmd.Apply<StrengthPower>(ctx, target, 1);".to_owned(),
                ),
            ],
        };

        assert_eq!(
            effects_in(&file, "AfterSideTurnStart", &tracked_regexes()),
            vec!["damage", "power"]
        );
    }

    #[test]
    fn class_files_reject_bodyless_declarations() {
        let root = std::env::temp_dir().join("spire-catalog-bodyless");
        std::fs::create_dir_all(&root).expect("creating the test class tree");
        std::fs::write(
            root.join("Empty.cs"),
            "namespace MegaCrit.Sts2.Core.Models.Relics;\npublic sealed class Empty\n{\n    public void Hook();\n}\n",
        )
        .expect("writing the test class");

        let error = match class_files(&root, "MegaCrit.Sts2.Core.Models.Relics") {
            Ok(files) => panic!("a bodyless declaration passed with {} classes", files.len()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("no block body"));
        std::fs::remove_dir_all(root).expect("removing the test class tree");
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
