//! The event surface's unit tests: each test drives a scripted event
//! sequence against a fresh core and asserts the persisted JSON. The suite
//! is split by topic into submodules; this file keeps the shared helpers.

use super::*;
use crate::data::records::{CardRec, CombatRec};
use crate::test_util::{SourceFixture, combat_epoch, combat_ids, unique_dir};

mod boundary;
mod ids;
mod run;
mod self_test;
mod writes;

/// A fresh data dir and core with a running combat.
fn combat_fixture(encounter: &str) -> PathBuf {
    let base = unique_dir(&format!("spire-profiler-test-{encounter}"));
    test_reset();
    init(&base);
    combat_started(encounter, "test");
    base
}

fn read_test_file(base: &Path, name: &str) -> String {
    std::fs::read_to_string(base.join(name)).expect("test file missing")
}

fn read_all_combats(base: &Path) -> Vec<(CombatRec, serde_json::Value)> {
    let runs_dir = base.join("runs");
    combat_ids(&runs_dir)
        .iter()
        .map(|&(run_id, id)| {
            let text = std::fs::read_to_string(
                runs_dir.join(run_id.to_string()).join(format!("{id}.json")),
            )
            .expect("combat file readable");
            let rec = crate::data::records::parse_combat_doc(&text).expect("combat doc parses");
            let doc: serde_json::Value = serde_json::from_str(&text).expect("combat doc parses");
            (rec, doc)
        })
        .collect()
}

fn read_all_runs(base: &Path) -> Vec<serde_json::Value> {
    let text = read_test_file(base, "runs.jsonl");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("run line parses"))
        .collect()
}

fn read_combat(base: &Path) -> (CombatRec, serde_json::Value) {
    let mut all = read_all_combats(base);
    assert_eq!(all.len(), 1, "exactly one combat record");
    let (combat, doc) = all.remove(0);
    (combat, doc)
}

fn card_row<'a>(combat: &'a CombatRec, id: &str) -> &'a CardRec {
    combat
        .cards
        .iter()
        .find(|card| card.id == id)
        .unwrap_or_else(|| panic!("no card row for {id} in the self-test combat"))
}

fn current_play_counters() -> (u32, u32, u32) {
    STATE.with(|cell| {
        let st = cell.borrow();
        let combat = st.current.as_ref().expect("combat exists");
        (
            combat.plays,
            combat.generated_plays,
            combat.generation_triggers,
        )
    })
}

fn card_json<'a>(combat: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    combat["cards"]
        .as_array()
        .expect("combat cards array")
        .iter()
        .find(|card| card["id"] == id)
        .unwrap_or_else(|| panic!("no card row for {id} in the combat JSON"))
}

fn assert_no_key(value: &serde_json::Value, key: &str) {
    match value {
        serde_json::Value::Object(map) => {
            assert!(!map.contains_key(key), "unexpected '{key}' field");
            for child in map.values() {
                assert_no_key(child, key);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                assert_no_key(child, key);
            }
        }
        _ => {}
    }
}
