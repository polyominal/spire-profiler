use std::collections::HashSet;

use super::*;
use crate::check_catalog::{ParsedClass, candidate_hooks, tracked_regexes};

const SOURCE: &str = "public override void Hook(int amount = 1) {
    Helper(); PowerCmd.Apply(amount, \"a b\");
}
private void Helper() { CreatureCmd.Damage(1); Deep(); }
private void Deep() { ForgeCmd.Forge(1); }";

impl Fingerprints {
    fn fixture(methods: &str) -> Self {
        let source = format!(
            "namespace MegaCrit.Sts2.Core.Models.Powers; public sealed class Power {{ {methods} }}"
        );
        let files = ParsedClass::read(&[("Power.cs", &source)], "MegaCrit.Sts2.Core.Models.Powers")
            .expect("the fixture supplies complete C# declarations");
        let (snapshot, failures) = Self::capture(
            [("Powers", &files), ("Relics", &BTreeMap::new())],
            &BTreeSet::from([("Powers", "Power", "Hook")]),
        );
        assert!(failures.is_empty(), "{failures:?}");
        snapshot
    }
}

#[test]
fn declaration_and_direct_helper_changes_expire_review() {
    let accepted = Fingerprints::fixture(SOURCE);
    let expected = accepted.methods.keys().cloned().collect();
    for (before, after) in [
        ("int amount", "long amount"),
        ("amount = 1", "amount = 2"),
        ("Apply(amount", "Apply(amount + 1"),
        ("Damage(1)", "Damage(2)"),
        ("void Helper()", "void Helper(int other = 0)"),
        ("a b", "a  b"),
    ] {
        let changed = Fingerprints::fixture(&SOURCE.replace(before, after));
        assert_eq!(
            changed.compare(&accepted, &expected),
            ["stale reviewed fingerprint: Powers.Power.Hook"],
            "change {before:?} to {after:?} must expire the body review"
        );
    }
}

#[test]
fn trivia_and_helper_order_do_not_expire_review() {
    let accepted = Fingerprints::fixture(SOURCE);
    let formatted = Fingerprints::fixture(
        "private void Deep() { ForgeCmd.Forge(1); }
        // independent source order
        private void Helper() { CreatureCmd.Damage(1); /* explanation */ Deep(); }
        public override void Hook( int amount = 1 ) {
            Helper( ); PowerCmd.Apply( amount, \"a b\" ); // explanation
        }",
    );
    assert_eq!(formatted.methods, accepted.methods);
    let outside_review = Fingerprints::fixture(&SOURCE.replace("Forge(1)", "Forge(2)"));
    assert_eq!(outside_review.methods, accepted.methods);
}

#[test]
fn helper_overloads_and_cycles_follow_the_scanners_bounded_matching() {
    let source = "public override void Hook() { Helper(); }
        private void Helper() { Hook(); }
        private void Helper(int amount) { PowerCmd.Apply(amount); }";
    let accepted = Fingerprints::fixture(source);
    let changed = Fingerprints::fixture(&source.replace("Apply(amount)", "Apply(amount + 1)"));
    assert_eq!(
        changed.compare(&accepted, &accepted.methods.keys().cloned().collect()),
        ["stale reviewed fingerprint: Powers.Power.Hook"]
    );
}

#[test]
fn stable_candidate_names_do_not_hide_changed_effects() {
    let accepted = Fingerprints::fixture("public override void Hook() { PowerCmd.Apply(1); }");
    let source = "namespace MegaCrit.Sts2.Core.Models.Powers; public sealed class Power {
        public override void Hook() { PowerCmd.Apply(1); CreatureCmd.Damage(2); }
    }";
    let files = ParsedClass::read(&[("Power.cs", source)], "MegaCrit.Sts2.Core.Models.Powers")
        .expect("the fixture supplies complete C# declarations");
    let empty = BTreeMap::new();
    let candidates = candidate_hooks(
        &empty,
        &files,
        &HashSet::from(["Hook".to_owned()]),
        &tracked_regexes(),
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].effects, ["damage", "power"]);
    let (changed, failures) = Fingerprints::capture(
        [("Powers", &files), ("Relics", &empty)],
        &BTreeSet::from([("Powers", "Power", "Hook")]),
    );
    assert!(failures.is_empty());
    assert_eq!(
        changed.compare(&accepted, &accepted.methods.keys().cloned().collect()),
        ["stale reviewed fingerprint: Powers.Power.Hook"]
    );
}

#[test]
fn ambiguous_or_missing_reviewed_roots_have_no_fingerprint() {
    let source = "namespace MegaCrit.Sts2.Core.Models.Powers; public sealed class Power {
        public override void Hook() { PowerCmd.Apply(1); }
        public override void Hook(int amount) { PowerCmd.Apply(amount); }
    }";
    let files = ParsedClass::read(&[("Power.cs", source)], "MegaCrit.Sts2.Core.Models.Powers")
        .expect("the fixture supplies complete C# declarations");
    let (snapshot, failures) = Fingerprints::capture(
        [("Powers", &files), ("Relics", &BTreeMap::new())],
        &BTreeSet::from([
            ("Powers", "Power", "Hook"),
            ("Powers", "Power", "Missing"),
            ("Powers", "Missing", "Hook"),
        ]),
    );
    assert!(snapshot.methods.is_empty());
    assert_eq!(failures.len(), 3);
    assert!(failures[0].contains("class is missing"));
    assert!(failures[1].contains("method is missing or overloaded"));
    assert!(failures[2].contains("method is missing or overloaded"));
}

#[test]
fn incompatible_missing_stale_and_orphaned_baselines_fail_with_review_artifacts() -> Result<()> {
    let scratch_root = workspace_root().join("tmp/catalog-review-tests");
    fs::create_dir_all(&scratch_root)?;
    let scratch = tempfile::tempdir_in(scratch_root)?;
    let accepted_path = scratch.path().join("accepted.json");
    let output = scratch.path().join("review");
    let candidate = Fingerprints::fixture(SOURCE);
    let expected = candidate.methods.keys().cloned().collect();
    for baseline in [
        None,
        Some("malformed".to_owned()),
        Some(r#"{"format_version":0,"game_version":"old","methods":{"Powers.Power.Hook":"old","Powers.Old.Hook":"old"}}"#.to_owned()),
        Some(format!(
            r#"{{"format_version":1,"game_version":"{}","methods":{{}}}}"#,
            game_version::PIN
        )),
    ] {
        if let Some(baseline) = &baseline {
            fs::write(&accepted_path, baseline)?;
        }
        let mut failures = Vec::new();
        candidate.write_review(&accepted_path, &output, &expected, &mut failures)?;
        assert!(!failures.is_empty());
        assert_eq!(fs::read_to_string(&accepted_path).ok(), baseline);
        let proposed: Fingerprints = serde_json::from_str(&fs::read_to_string(output.join("candidate.json"))?)?;
        assert_eq!(proposed.methods, candidate.methods);
        let report = fs::read_to_string(output.join("changes.txt"))?;
        for failure in &failures {
            assert!(report.contains(failure));
        }
        if baseline.as_ref().is_some_and(|text| text.contains("Powers.Old")) {
            assert_eq!(failures.len(), 4);
            for kind in ["format", "game", "stale", "orphaned"] {
                assert!(failures.iter().any(|failure| failure.contains(kind)));
            }
        }
    }
    fs::copy(output.join("candidate.json"), &accepted_path)?;
    let accepted = fs::read_to_string(&accepted_path)?;
    let mut failures = Vec::new();
    candidate.write_review(&accepted_path, &output, &expected, &mut failures)?;
    assert!(failures.is_empty());
    assert_eq!(fs::read_to_string(&accepted_path)?, accepted);
    assert!(fs::read_to_string(output.join("changes.txt"))?.contains("No review changes."));
    Ok(())
}

#[test]
fn duplicate_review_identities_and_conditional_compilation_are_rejected() {
    let duplicate = r#"{"format_version":1,"game_version":"v0.111.0","methods":{"Powers.Power.Hook":"first","Powers.Power.Hook":"second"}}"#;
    assert!(
        serde_json::from_str::<Fingerprints>(duplicate)
            .err()
            .expect("duplicate keys must not overwrite accepted review evidence")
            .to_string()
            .contains("duplicate method identity")
    );
    let source = "namespace MegaCrit.Sts2.Core.Models.Powers; public sealed class Power {
        public override void Hook() {
        #if DEBUG
            PowerCmd.Apply(1);
        #endif
        }
    }";
    let error = ParsedClass::read(&[("Power.cs", source)], "MegaCrit.Sts2.Core.Models.Powers")
        .err()
        .expect("unconfigured conditional compilation is not a complete review input");
    assert!(error.to_string().contains("conditional compilation"));
}
