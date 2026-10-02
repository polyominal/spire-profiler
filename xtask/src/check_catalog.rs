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
use serde::Deserialize;

use crate::{catalog, csharp, decompile, game_version, workspace_root};

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

struct ClassFile {
    methods: Vec<Method>,
}

#[derive(Deserialize)]
struct ParsedClass {
    source: String,
    name: String,
    ns: String,
    methods: Vec<Method>,
}

#[derive(Deserialize)]
struct Method {
    name: String,
    body: Option<String>,
    is_override: bool,
    is_virtual: bool,
    calls: Vec<String>,
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
            for body in file
                .methods
                .iter()
                .filter_map(|method| method.body.as_deref())
            {
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
        let mut bodies: Vec<_> = self.body.as_deref().into_iter().collect();
        for call in &self.calls {
            bodies.extend(
                methods
                    .iter()
                    .filter(|method| method.name == *call)
                    .filter_map(|method| method.body.as_deref()),
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
    let sources: Vec<_> = BASE_MODELS
        .iter()
        .map(|name| {
            let path = models.join(name);
            Ok((
                path.to_string_lossy().into_owned(),
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
            ))
        })
        .collect::<Result<_>>()?;
    let sources: Vec<_> = sources
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let classes: Vec<ParsedClass> = csharp::parse("catalog", &sources)?;
    Ok(classes
        .into_iter()
        .flat_map(|class| class.methods)
        .filter(|method| method.is_virtual)
        .map(|method| method.name)
        .collect())
}

/// Every Mocks tree is test support rather than a game hook.
fn class_files(dir: &Path, expected_namespace: &str) -> Result<BTreeMap<String, ClassFile>> {
    let mut sources = Vec::new();
    let mut directories = vec![dir.to_path_buf()];
    while let Some(current) = directories.pop() {
        for entry in
            fs::read_dir(&current).with_context(|| format!("listing {}", current.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_none_or(|name| name != "Mocks") {
                    directories.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "cs") {
                sources.push((
                    path.to_string_lossy().into_owned(),
                    fs::read_to_string(&path)?,
                ));
            }
        }
    }
    sources.sort_by(|first, second| first.0.cmp(&second.0));
    let sources: Vec<_> = sources
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    ParsedClass::read(&sources, expected_namespace)
}

impl ParsedClass {
    fn read(
        sources: &[(&str, &str)],
        expected_namespace: &str,
    ) -> Result<BTreeMap<String, ClassFile>> {
        let classes: Vec<Self> = csharp::parse("catalog", sources)?;
        let mut files = BTreeMap::new();
        for class in classes {
            if class.ns != expected_namespace {
                bail!(
                    "{} declares namespace {:?}, expected {:?}",
                    class.source,
                    class.ns,
                    expected_namespace
                );
            }
            for method in &class.methods {
                if method.body.is_none() {
                    bail!(
                        "{}: {}.{}: no block or expression body",
                        class.source,
                        class.name,
                        method.name
                    );
                }
            }
            if files
                .insert(
                    class.name.clone(),
                    ClassFile {
                        methods: class.methods,
                    },
                )
                .is_some()
            {
                bail!("duplicate class {}", class.name);
            }
        }
        Ok(files)
    }
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

    fn fixture(methods: &str) -> ClassFile {
        let source = format!(
            "namespace MegaCrit.Sts2.Core.Models.Powers; public sealed class Power {{ {methods} }}"
        );
        ParsedClass::read(&[("Power.cs", &source)], "MegaCrit.Sts2.Core.Models.Powers")
            .expect("fixture declares a complete class")
            .remove("Power")
            .expect("fixture declares Power")
    }

    #[test]
    fn hook_universe_accepts_non_task_return_types() -> Result<()> {
        let shell = xshell::Shell::new()?;
        let temp = shell.create_temp_dir()?;
        for name in BASE_MODELS {
            std::fs::write(
                temp.path().join(name),
                "public class Model { public virtual ValueTask<int> NewHook(); private void Helper() {} }",
            )?;
        }
        assert_eq!(
            hook_universe(temp.path())?,
            HashSet::from(["NewHook".to_owned()])
        );
        Ok(())
    }

    #[test]
    fn class_files_reject_namespace_drift_and_bodyless_methods() -> Result<()> {
        let shell = xshell::Shell::new()?;
        let temp = shell.create_temp_dir()?;
        let path = temp.path().join("Moved.cs");
        for (source, expected) in [
            (
                "namespace MegaCrit.Sts2.Core.Moved; public sealed class Moved {}",
                "expected",
            ),
            (
                "namespace MegaCrit.Sts2.Core.Models.Relics; public sealed class Empty { public void Hook(); }",
                "no block or expression body",
            ),
            (
                "namespace MegaCrit.Sts2.Core.Models.Relics; public sealed class Broken { public void Hook( }",
                "error CS",
            ),
        ] {
            fs::write(&path, source)?;
            let error = class_files(temp.path(), "MegaCrit.Sts2.Core.Models.Relics")
                .err()
                .expect("invalid class fails");
            assert!(error.to_string().contains(expected), "{error}");
        }
        Ok(())
    }

    #[test]
    fn card_generation_follows_game_commands_not_object_creation() {
        let file = fixture("public override void AfterPlayerTurnStart() { CardCmd.TransformToRandom(card, rng); }
            public void AfterObtained() { RunState.CreateCard<Apotheosis>(owner); CardPileCmd.Add(card, PileType.Deck); }");
        let tracked = tracked_regexes();
        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked),
            ["cardgen"]
        );
        assert!(file.methods[1].effects(&file.methods, &tracked).is_empty());
    }

    #[test]
    fn power_application_covers_generic_and_plain_apply() {
        let file = fixture(
            "private void Generic() => PowerCmd.Apply<StrengthPower>(ctx, target, 1);
            private void Plain() { PowerCmd.Apply(ctx, power, target, 1); }",
        );
        for method in &file.methods {
            assert_eq!(method.effects(&file.methods, &tracked_regexes()), ["power"]);
        }
    }

    #[test]
    fn effects_follow_one_helper_level_and_union_direct_calls() {
        let file = fixture("public override void AfterSideTurnStart() { CreatureCmd.Damage(ctx, target, 1); Buff(); }
            private void Buff() { PowerCmd.Apply<StrengthPower>(ctx, target, 1); TooDeep(); }
            private void TooDeep() { ForgeCmd.Forge(); }");
        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked_regexes()),
            ["damage", "power"]
        );
    }

    #[test]
    fn syntax_boundaries_ignore_braces_and_fake_helpers_in_literals_and_comments() {
        let file = fixture(
            r#"public override async Task Hook() {
                var text = "} Buff() {"; var brace = '}';
                /* } Buff(); */ if (live) { await CreatureCmd.Damage(); }
            }
            private void Buff() => PowerCmd.Apply<StrengthPower>();"#,
        );
        assert_eq!(
            file.methods[0].effects(&file.methods, &tracked_regexes()),
            ["damage"]
        );
        assert_eq!(file.methods.len(), 2);
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
    fn overloads_keep_their_bodies_and_catalog_entries_require_one_method() {
        let file = fixture(
            "public override void AfterSideTurnStart() {}
            public override void AfterSideTurnStart(int amount) { PowerCmd.Apply(amount); }
            private void Helper() { CreatureCmd.Damage(); }",
        );
        let files = BTreeMap::from([("Power".to_owned(), file)]);
        let universe = HashSet::from(["AfterSideTurnStart".to_owned()]);
        let tracked = tracked_regexes();
        let relics = BTreeMap::new();
        let candidates = candidate_hooks(&relics, &files, &universe, &tracked);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].effects, ["power"]);
        let mut review = Review::default();
        review.check_entries(
            &[("Power", "AfterSideTurnStart"), ("Power", "Missing")],
            &files,
            "Powers",
            &universe,
            &tracked,
        );
        assert_eq!(review.failures.len(), 2);
        assert!(review.failures[0].contains("AfterSideTurnStart is missing or overloaded"));
        assert!(review.failures[1].contains("Missing is missing or overloaded"));
    }
}
