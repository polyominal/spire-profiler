//! The event surface's unit tests: each test drives a scripted event
//! sequence against a fresh core and asserts the persisted JSON. The suite
//! is split by topic into submodules; this file keeps the shared helpers.

use super::*;
use crate::data::records::{CardRec, CombatRec};
use crate::data::state::CardStat;

mod card;
mod combat;
mod orb_potion;
mod power;
mod run;
mod self_test;

fn read_test_file(base: &str, name: &str) -> String {
    std::fs::read_to_string(format!("{base}/{name}")).expect("test file missing")
}

fn read_all_combats(base: &str) -> Vec<(CombatRec, serde_json::Value)> {
    let runs_dir = std::path::Path::new(base).join("runs");
    let mut ids: Vec<(u32, u32)> = Vec::new(); // (run id, combat id)
    for run in std::fs::read_dir(&runs_dir)
        .expect("runs dir exists")
        .flatten()
    {
        let run_dir = run.path();
        if !run_dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&run_dir)
            .expect("run dir exists")
            .flatten()
        {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = stem.parse::<u32>() else {
                continue;
            };
            let run_id = run_dir
                .file_name()
                .expect("run dir has a name")
                .to_string_lossy()
                .parse::<u32>()
                .expect("run dirs are numeric");
            ids.push((run_id, id));
        }
    }
    ids.sort_unstable_by_key(|&(_, id)| id);
    ids.iter()
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

fn read_all_runs(base: &str) -> Vec<serde_json::Value> {
    let text = read_test_file(base, "runs.jsonl");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("run line parses"))
        .collect()
}

fn read_combat(base: &str) -> (CombatRec, serde_json::Value) {
    let mut all = read_all_combats(base);
    assert_eq!(all.len(), 1, "exactly one combat record");
    let (combat, doc) = all.remove(0);
    (combat, serde_json::json!([doc]))
}

fn assert_no_card(combat: &CombatRec, id: &str) {
    assert!(
        combat.cards.iter().all(|card| card.id != id),
        "no ledger row for {id}"
    );
}

fn card_row<'a>(combat: &'a CombatRec, id: &str) -> &'a CardRec {
    combat
        .cards
        .iter()
        .find(|card| card.id == id)
        .unwrap_or_else(|| panic!("no card row for {id} in the self-test combat"))
}

fn current_rows() -> Vec<CardStat> {
    STATE.with(|cell| {
        let st = cell.borrow();
        st.current.as_ref().expect("combat exists").cards.clone()
    })
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
    combat[0]["cards"]
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
