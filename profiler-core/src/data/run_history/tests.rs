//! Each test seeds the persisted store and asserts the selection and view
//! results.

use std::fs;
use std::path::Path;

use super::*;
use crate::data::state::RunOutcome;
use crate::source_kind::SourceKind;
use crate::test_util::scratch_dir;

fn seed_data(data: &Path, runs_text: &str, combats_text: &str) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.runs_path_full = data.join("runs.jsonl");
        st.runs_dir_full = data.join("runs");
    });
    invalidate();
    clear();
    write_runs_fixture(data, runs_text);
    write_combats_fixture(data, combats_text);
}

fn write_runs_fixture(data: &Path, runs_text: &str) {
    let runs: serde_json::Value = serde_json::from_str(runs_text).expect("runs fixture parses");
    let mut lines = String::new();
    for run in runs.as_array().expect("runs fixture is an array") {
        lines.push_str(&run.to_string());
        lines.push('\n');
    }
    fs::write(data.join("runs.jsonl"), lines).expect("write fixture");
}

fn write_combats_fixture(data: &Path, combats_text: &str) {
    let combats: serde_json::Value =
        serde_json::from_str(combats_text).expect("combats fixture parses");
    for combat in combats.as_array().expect("combats fixture is an array") {
        let id = combat["combat_id"].as_u64().expect("fixture combat_id");
        let run_seq = combat["run"]["seq"].as_u64().expect("fixture run.seq");
        let dir = data.join("runs").join(run_seq.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{id}.json")), combat.to_string()).unwrap();
    }
}

const RUNS: &str = r#"[
        {"run_id":1,"profile":2,"build":"b","character":"SHROUD","ascension":3,"game_mode":"Standard",
         "outcome":"victory","seed":"ALPHA","started_at":1786579200,
         "ended_at":1786579800,"combats":1,"damage_dealt":30,"damage_taken":12},
        {"run_id":2,"profile":2,"build":"b","character":"IRONCLAD","ascension":7,"game_mode":"Standard",
         "outcome":"defeat","seed":"BETA","started_at":1786665600,
         "ended_at":1786699200,"combats":2,"damage_dealt":70,"damage_taken":40,
         "players":[{"slot":0,"net_id":"1","character":"IRONCLAD"},
                    {"slot":1,"net_id":"2","character":"SILENT"}]}
    ]"#;

const COMBATS: &str = r#"[
        {"combat_id":1,"encounter_id":"A","result":"completed","turns":4,"damage_received":12,
         "block_total":0,
         "run":{"seq":1,"character":"SHROUD","ascension":3,"game_mode":"Standard"},
         "cards":[
            {"id":"STRIKE","kind":0,"plays":2,"damage_dealt":20,"block_gained":0,"block_effective":0,"heal":0},
            {"id":"CRACKED_CORE","kind":1,"plays":0,"damage_dealt":10,"block_gained":0,"block_effective":0,"heal":0}
         ]},
        {"combat_id":2,"encounter_id":"B","result":"completed","turns":5,"damage_received":15,
         "block_total":16,
         "run":{"seq":2,"character":"IRONCLAD","ascension":7,"game_mode":"Standard"},
         "cards":[
            {"id":"STRIKE","kind":0,"plays":3,"damage_dealt":40,"block_gained":0,"block_effective":0,"heal":0,
             "dmg_direct":30,"dmg_attributed":10,"dmg_modifier":0},
            {"id":"DEFEND","kind":0,"plays":2,"damage_dealt":0,"block_gained":16,"block_effective":15,"heal":0}
         ]},
        {"combat_id":3,"encounter_id":"C","result":"defeated","turns":3,"damage_received":25,
         "block_total":0,
         "run":{"seq":2,"character":"IRONCLAD","ascension":7,"game_mode":"Standard"},
         "cards":[
            {"id":"STRIKE","kind":0,"plays":1,"damage_dealt":30,"block_gained":0,"block_effective":0,"heal":0,
             "dmg_direct":20,"dmg_attributed":0,"dmg_modifier":5},
            {"id":"NOT_YET","kind":0,"plays":1,"damage_dealt":0,"block_gained":0,"block_effective":0,"heal":4}
         ]}
    ]"#;

const COMBATS_ONLY: &str = r#"[
        {"combat_id":10,"encounter_id":"A","result":"completed","turns":4,"damage_received":12,
         "block_total":5,
         "run":{"seq":3,"character":"SHROUD","ascension":1,"game_mode":"Standard","seed":"GAMMA"},
         "cards":[
            {"id":"STRIKE","kind":0,"plays":2,"damage_dealt":20,"block_gained":0,"block_effective":0,"heal":0},
            {"id":"DASH","kind":0,"plays":1,"damage_dealt":15,"block_gained":5,"block_effective":5,"heal":0}
         ]},
        {"combat_id":11,"encounter_id":"B","result":"defeated","turns":3,"damage_received":25,
         "block_total":0,
         "run":{"seq":3,"character":"SHROUD","ascension":1,"game_mode":"Standard","seed":"GAMMA"},
         "cards":[
            {"id":"STRIKE","kind":0,"plays":1,"damage_dealt":10,"block_gained":0,"block_effective":0,"heal":0}
         ]}
    ]"#;

const BETA_START: i64 = 1_786_665_600;

#[test]
fn next_run_id_advances_past_runs_file_and_run_dirs() {
    let data = scratch_dir("next-run-id");
    seed_data(
        &data,
        r#"[{"run_id":9,"profile":2,"character":"A","ascension":0,"game_mode":"Standard",
                "outcome":"victory","seed":"X","started_at":1786579200,
                "ended_at":1786579800,"combats":1,"damage_dealt":1,"damage_taken":1}]"#,
        r#"[{"combat_id":12,"encounter_id":"K","result":"completed","turns":1,
                "damage_received":1,"block_total":0,
                "run":{"seq":12,"character":"A","ascension":0,"game_mode":"Standard"},
                "cards":[]}]"#,
    );
    assert_eq!(
        next_run_id(&data.join("runs.jsonl"), &data.join("runs")),
        13
    );
    let empty = scratch_dir("next-run-id-empty");
    assert_eq!(
        next_run_id(&empty.join("runs.jsonl"), &empty.join("runs")),
        1
    );
}

#[test]
fn next_run_id_reserves_abandoned_run_dirs() {
    let data = scratch_dir("next-run-id-abandoned");
    seed_data(&data, "[]", "[]");
    fs::create_dir_all(data.join("runs/12")).unwrap();
    assert_eq!(
        next_run_id(&data.join("runs.jsonl"), &data.join("runs")),
        13
    );
    fs::create_dir_all(data.join("runs/profile-1")).unwrap();
    assert_eq!(
        next_run_id(&data.join("runs.jsonl"), &data.join("runs")),
        13
    );
}

#[test]
fn continued_run_id_rejoins_the_latest_matching_fragment() {
    let data = scratch_dir("continued-run-id");
    seed_data(
        &data,
        "[]",
        r#"[{"combat_id":1,"encounter_id":"A","result":"completed","turns":4,"damage_received":12,
                "block_total":0,
                "run":{"seq":7,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"AAA"},
                "cards":[]},
               {"combat_id":2,"encounter_id":"B","result":"completed","turns":4,"damage_received":12,
                "block_total":0,
                "run":{"seq":9,"character":"IRONCLAD","ascension":1,"game_mode":"Standard","seed":"BBB"},
                "cards":[]},
               {"combat_id":3,"encounter_id":"C","result":"completed","turns":4,"damage_received":12,
                "block_total":0,
                "run":{"seq":7,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"AAA"},
                "cards":[]}]"#,
    );
    assert_eq!(continued_run_id(&data.join("runs"), "AAA"), Some(7));
    assert_eq!(continued_run_id(&data.join("runs"), "BBB"), Some(9));
    assert_eq!(continued_run_id(&data.join("runs"), "CCC"), None);
    assert_eq!(continued_run_id(&data.join("runs"), ""), None);
}

#[test]
fn select_by_seed_assembles_the_full_view() {
    let base = scratch_dir("run-history-seed");
    let data = std::path::Path::new(&base);
    seed_data(data, RUNS, COMBATS);

    let RunSelection::Selected(view) = select_run("BETA", BETA_START, 2) else {
        panic!("BETA must match");
    };
    assert_eq!(view.run_id, 2);
    assert_eq!(view.character, "IRONCLAD");
    assert_eq!(view.ascension, 7);
    assert_eq!(view.game_mode, "Standard");
    assert_eq!(view.outcome, Some(RunOutcome::Defeat));
    assert_eq!(view.result, "Defeat");
    assert_eq!(view.seed, "BETA");
    assert_eq!(view.started_at, BETA_START);
    assert_eq!(view.players.len(), 2);
    assert_eq!(view.players[0].slot, 0);
    assert_eq!(view.players[0].character, "IRONCLAD");
    assert_eq!(view.players[1].character, "SILENT");

    assert_eq!(view.combats.len(), 2);
    assert_eq!(view.combats[0].seq, 2);
    assert_eq!(view.combats[0].encounter, "B");
    assert_eq!(view.combats[0].result, "completed");
    assert_eq!(view.combats[0].damage_dealt, 40);
    assert_eq!(view.combats[0].damage_taken, 15);
    assert_eq!(view.combats[0].turns, 5);
    assert_eq!(view.combats[1].seq, 3);
    assert_eq!(view.combats[1].result, "defeated");
    assert_eq!(view.combats[1].damage_dealt, 30);
    assert_eq!(view.combats[1].damage_taken, 25);

    assert_eq!(view.rollup.len(), 3);
    assert_eq!(view.rollup[0].id, "STRIKE");
    assert_eq!(view.rollup[0].kind, SourceKind::Card);
    assert_eq!(view.rollup[0].damage_dealt, 70);
    assert_eq!(view.rollup[0].plays, 4);
    assert_eq!(view.rollup[0].dmg_direct, 50);
    assert_eq!(view.rollup[0].dmg_attributed, 10);
    assert_eq!(view.rollup[0].dmg_modifier, 5);
    assert_eq!(view.rollup[1].id, "DEFEND");
    assert_eq!(view.rollup[1].damage_dealt, 0);
    assert_eq!(view.rollup[1].block_effective, 15);
    assert_eq!(view.rollup[2].id, "NOT_YET");
    assert_eq!(view.rollup[2].damage_dealt, 0);
}

#[test]
fn combat_only_runs_fall_back_to_a_synthesized_view() {
    let base = scratch_dir("run-history-combats-only");
    let data = std::path::Path::new(&base);
    seed_data(data, "[]", COMBATS_ONLY);

    let RunSelection::Selected(view) = select_run("GAMMA", 0, 4) else {
        panic!("GAMMA must fall back to its combats");
    };
    assert_eq!(view.run_id, 3);
    assert_eq!(view.character, "SHROUD");
    assert_eq!(view.ascension, 1);
    assert_eq!(view.game_mode, "Standard");
    assert_eq!(view.seed, "GAMMA");
    assert_eq!(
        view.profile, 4,
        "the select's profile carries onto the view"
    );
    assert!(view.outcome.is_none());
    assert_eq!(
        view.result, "Unfinished",
        "the run never closed, so its terminal state is unknown"
    );
    assert_eq!(view.started_at, 0);
    assert_eq!(view.ended_at, 0);
    assert!(view.players.is_empty());
    assert_eq!(view.combats.len(), 2);
    assert_eq!(view.combats[0].seq, 10);
    assert_eq!(view.combats[0].damage_dealt, 35);
    assert_eq!(view.combats[1].seq, 11);
    assert_eq!(view.rollup.len(), 2);
    assert_eq!(view.rollup[0].id, "STRIKE");
    assert_eq!(view.rollup[0].damage_dealt, 30);
    assert_eq!(view.rollup[0].plays, 3);
    assert_eq!(view.rollup[1].id, "DASH");
    assert_eq!(view.rollup[1].damage_dealt, 15);
    assert_eq!(view.rollup[1].block_effective, 5);
}

#[test]
fn seeds_without_combats_or_entries_stay_empty() {
    let base = scratch_dir("run-history-fallback-empty");
    let data = std::path::Path::new(&base);
    seed_data(
        data,
        "[]",
        r#"[{"combat_id":1,"encounter_id":"X","result":"completed","turns":1,
                 "damage_received":0,"block_total":0,
                 "run":{"seq":1,"character":"A","ascension":0,"game_mode":"Standard","seed":"OTHER"},
                 "cards":[]}]"#,
    );
    assert!(matches!(
        select_run("NOPE", 1_786_624_800, 2),
        RunSelection::Empty
    ));
    // An empty seed declines the fallback too — matching on it would merge
    // unrelated legacy runs.
    assert!(matches!(
        select_run("", 1_786_624_800, 2),
        RunSelection::Empty
    ));
    let fresh = scratch_dir("run-history-fallback-fresh");
    let fresh_data = std::path::Path::new(&fresh);
    seed_data(fresh_data, "[]", "[]");
    assert!(matches!(
        select_run("NOPE", 1_786_624_800, 2),
        RunSelection::Empty
    ));
}

#[test]
fn closed_runs_never_take_the_fallback() {
    let base = scratch_dir("run-history-no-fallback");
    let data = std::path::Path::new(&base);
    seed_data(data, RUNS, COMBATS);

    let RunSelection::Selected(defeat) = select_run("BETA", BETA_START, 2) else {
        panic!("BETA must match");
    };
    assert_eq!(defeat.result, "Defeat");
    let RunSelection::Selected(won) = select_run("ALPHA", 1_786_579_200, 2) else {
        panic!("ALPHA must match");
    };
    assert_eq!(won.outcome, Some(RunOutcome::Victory));
    assert_eq!(won.result, "Victory");
}

#[test]
fn abandoned_runs_render_the_abandoned_label() {
    let base = scratch_dir("run-history-abandoned");
    let data = std::path::Path::new(&base);
    let runs = r#"[
            {"run_id":5,"profile":2,"character":"DEFECT","ascension":2,"game_mode":"Standard",
             "outcome":"abandoned","seed":"GAMMA",
             "started_at":1786579200,"ended_at":1786579800,
             "combats":1,"damage_dealt":10,"damage_taken":5}
        ]"#;
    seed_data(data, runs, "[]");

    let RunSelection::Selected(view) = select_run("GAMMA", 1_786_579_200, 2) else {
        panic!("GAMMA must match");
    };
    assert_eq!(view.outcome, Some(RunOutcome::Abandoned));
    assert_eq!(view.result, "Abandoned");
    assert_eq!(
        view.ended_at, 1_786_579_800,
        "ended_at is the abandon moment"
    );
}

#[test]
fn fallback_views_flow_through_the_selection_plumbing() {
    let base = scratch_dir("run-history-fallback-plumbing");
    let data = std::path::Path::new(&base);
    seed_data(data, "[]", COMBATS_ONLY);

    assert!(select("GAMMA", 0, 4));
    assert!(screen_open(), "select marks the screen open");
    let RunSelection::Selected(expected) = select_run("GAMMA", 0, 4) else {
        panic!("GAMMA must fall back");
    };
    let stored = selected_view().expect("fallback selection stored");
    assert_eq!(&*expected, &stored);
    assert_eq!(stored.result, "Unfinished");
    let fp = selected_view_fingerprint().expect("fingerprint present");
    assert!(select("GAMMA", 0, 4));
    assert_eq!(selected_view_fingerprint(), Some(fp));
    clear();
    assert!(selected_view().is_none());
    assert_eq!(selected_view_fingerprint(), None);
}

#[test]
fn fallback_disambiguates_same_seed_replays_by_start_time() {
    let base = scratch_dir("run-history-fallback-seq");
    let data = std::path::Path::new(&base);
    seed_data(
        data,
        "[]",
        r#"[{"combat_id":1,"started_at":1000,"encounter_id":"A","result":"completed","turns":1,
                 "damage_received":0,"block_total":0,
                 "run":{"seq":4,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"OMEGA"},
                 "cards":[]},
                {"combat_id":2,"started_at":2000,"encounter_id":"B","result":"completed","turns":1,
                 "damage_received":0,"block_total":7,
                 "run":{"seq":6,"character":"IRONCLAD","ascension":2,"game_mode":"Standard","seed":"OMEGA"},
                 "cards":[]}]"#,
    );
    let RunSelection::Selected(view) = select_run("OMEGA", 1050, 2) else {
        panic!("OMEGA must fall back");
    };
    assert_eq!(view.run_id, 4, "the closest group wins, not the latest");
    assert_eq!(view.character, "DEFECT");
    assert_eq!(view.combats.len(), 1, "replays never merge");
    assert_eq!(view.combats[0].seq, 1);
    let RunSelection::Selected(view) = select_run("OMEGA", 2050, 2) else {
        panic!("OMEGA must fall back");
    };
    assert_eq!(view.run_id, 6);
    assert!(matches!(
        select_run("OMEGA", 10_000, 2),
        RunSelection::Empty
    ));
}

#[test]
fn same_seed_without_the_exact_time_selects_empty() {
    let base = scratch_dir("run-history-tiebreak");
    let data = std::path::Path::new(&base);
    let runs = r#"[
            {"run_id":1,"profile":2,"character":"A","ascension":0,"game_mode":"Daily","outcome":"victory",
             "seed":"DAILY","started_at":1786579200,"ended_at":1786579800,"combats":0},
            {"run_id":2,"profile":2,"character":"B","ascension":0,"game_mode":"Daily","outcome":"defeat",
             "seed":"DAILY","started_at":1786752000,"ended_at":1786752600,"combats":0}
        ]"#;
    seed_data(data, runs, "[]");

    assert!(matches!(
        select_run("DAILY", 1_786_708_800, 2),
        RunSelection::Empty
    ));
}

#[test]
fn seed_match_with_exact_start_time_wins() {
    let base = scratch_dir("run-history-exact-seed-time");
    let data = std::path::Path::new(&base);
    let runs = r#"[
            {"run_id":1,"profile":2,"character":"A","ascension":0,"game_mode":"Daily","outcome":"victory",
             "seed":"DAILY","started_at":1786579200,"ended_at":1786579800,"combats":0},
            {"run_id":2,"profile":2,"character":"B","ascension":0,"game_mode":"Daily","outcome":"defeat",
             "seed":"DAILY","started_at":1786752000,"ended_at":1786752600,"combats":0}
        ]"#;
    seed_data(data, runs, "[]");

    let RunSelection::Selected(view) = select_run("DAILY", 1_786_579_200, 2) else {
        panic!("DAILY must match");
    };
    assert_eq!(view.run_id, 1);
    assert_eq!(view.character, "A");
}

#[test]
fn a_wrong_seed_never_matches_even_at_the_exact_time() {
    let base = scratch_dir("run-history-exact-no-seed");
    let data = std::path::Path::new(&base);
    let runs = r#"[
            {"run_id":1,"profile":2,"character":"A","ascension":0,"game_mode":"Standard","outcome":"defeat",
             "seed":"REAL_SEED","started_at":1786579200,"ended_at":1786579800,"combats":0}
        ]"#;
    seed_data(data, runs, "[]");

    assert!(matches!(
        select_run("MANGLED", 1_786_579_200, 2),
        RunSelection::Empty
    ));
}

#[test]
fn unknown_runs_select_empty() {
    let base = scratch_dir("run-history-empty");
    let data = std::path::Path::new(&base);
    seed_data(data, RUNS, COMBATS);

    let far = BETA_START + 1_000_000;
    assert!(matches!(select_run("NOPE", far, 2), RunSelection::Empty));
    assert!(matches!(
        select_run("ALPHA", BETA_START, 2),
        RunSelection::Empty
    ));
    assert!(matches!(
        select_run("ALPHA", 1_786_579_200, 9),
        RunSelection::Empty
    ));
    assert!(matches!(
        select_run("ALPHA", 1_786_579_200, -1),
        RunSelection::Selected(_)
    ));

    let fresh = scratch_dir("run-history-fresh");
    let fresh_data = std::path::Path::new(&fresh);
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.runs_path_full = fresh_data.join("runs.jsonl");
        st.runs_dir_full = fresh_data.join("runs");
    });
    invalidate();
    assert!(matches!(
        select_run("ALPHA", BETA_START, 2),
        RunSelection::Empty
    ));

    let bare = scratch_dir("run-history-bare");
    let bare_data = std::path::Path::new(&bare);
    seed_data(
        bare_data,
        r#"[{"run_id":9,"profile":2,"character":"A","ascension":0,"game_mode":"Standard",
                "outcome":"defeat","seed":"BARE","started_at":1786579200,
                "ended_at":1786579800,"combats":0}]"#,
        "[]",
    );
    let RunSelection::Selected(view) = select_run("BARE", 1_786_579_200, 2) else {
        panic!("BARE must match");
    };
    assert!(view.combats.is_empty());
    assert!(view.rollup.is_empty());
}

#[test]
fn cache_reuses_until_invalidated() {
    let base = scratch_dir("run-history-cache");
    let data = std::path::Path::new(&base);
    seed_data(data, RUNS, COMBATS);

    let RunSelection::Selected(first) = select_run("BETA", BETA_START, 2) else {
        panic!("BETA must match");
    };
    assert_eq!(first.rollup[0].damage_dealt, 70);

    let tweaked = COMBATS.replace("\"damage_dealt\":40", "\"damage_dealt\":400");
    write_combats_fixture(data, &tweaked);
    let RunSelection::Selected(second) = select_run("BETA", BETA_START, 2) else {
        panic!("BETA must match");
    };
    assert_eq!(second.rollup[0].damage_dealt, 70);

    invalidate();
    let RunSelection::Selected(third) = select_run("BETA", BETA_START, 2) else {
        panic!("BETA must match");
    };
    assert_eq!(third.rollup[0].damage_dealt, 430);
}

#[test]
fn rollup_keys_on_id_and_kind_and_teams_merge() {
    let base = scratch_dir("run-history-kinds");
    let data = std::path::Path::new(&base);
    let runs = r#"[{"run_id":1,"profile":2,"character":"A","ascension":0,"game_mode":"Standard",
                        "outcome":"victory","seed":"K","started_at":1786579200,
                        "ended_at":1786579800,"combats":1}]"#;
    let combats = r#"[{"combat_id":1,"encounter_id":"K","result":"completed","turns":1,
                           "damage_received":0,"run":{"seq":1,"character":"A","ascension":0,"game_mode":"Standard"},
                           "cards":[
                            {"id":"DUPE","kind":0,"player":0,"plays":1,"damage_dealt":5,"block_gained":0,"block_effective":0,"heal":0},
                            {"id":"ZERO_A","kind":0,"player":0,"plays":1,"damage_dealt":0,"block_gained":1,"block_effective":0,"heal":0},
                            {"id":"DUPE","kind":2,"player":0,"plays":1,"damage_dealt":3,"block_gained":0,"block_effective":0,"heal":0},
                            {"id":"ZERO_B","kind":1,"player":0,"plays":1,"damage_dealt":0,"block_gained":2,"block_effective":0,"heal":0},
                            {"id":"DUPE","kind":0,"player":1,"plays":2,"damage_dealt":7,"block_gained":0,"block_effective":0,"heal":0}
                           ]}]"#;
    seed_data(data, runs, combats);

    let RunSelection::Selected(view) = select_run("K", 1_786_579_200, 2) else {
        panic!("K must match");
    };
    let ids: Vec<(String, SourceKind)> =
        view.rollup.iter().map(|r| (r.id.clone(), r.kind)).collect();
    assert_eq!(
        ids,
        vec![
            ("DUPE".to_owned(), SourceKind::Card),
            ("ZERO_A".to_owned(), SourceKind::Card),
            ("DUPE".to_owned(), SourceKind::Power),
            ("ZERO_B".to_owned(), SourceKind::Relic),
        ]
    );
    assert_eq!(view.rollup[0].plays, 3);
    assert_eq!(view.rollup[0].damage_dealt, 12);
    assert_eq!(view.rollup[0].player, TEAM_SLOT);
    assert_eq!(view.rollup[2].plays, 1);
    assert_eq!(view.rollup[2].damage_dealt, 3);
}

#[test]
fn select_stores_and_clear_drops_the_panel_view() {
    let base = scratch_dir("run-history-selection");
    let data = std::path::Path::new(&base);
    seed_data(data, RUNS, COMBATS);

    assert!(select("BETA", BETA_START, 2));
    assert!(screen_open(), "select marks the screen open");
    let view = selected_view().expect("selection stored");
    assert_eq!(view.seed, "BETA");
    assert_eq!(view.combats.len(), 2);

    clear();
    assert!(selected_view().is_none());
    assert!(!screen_open(), "clear marks the screen closed");
    assert!(select("BETA", BETA_START, 2));
    assert!(!select("NOPE", BETA_START + 1_000_000, 2));
    assert!(selected_view().is_none());
    assert!(screen_open(), "an empty match keeps the screen open");
}

#[test]
fn view_fingerprint_tracks_every_view_field() {
    let synthetic = || RunSummaryView {
        run_id: 2,
        character: "IRONCLAD".to_owned(),
        ascension: 7,
        game_mode: "Standard".to_owned(),
        outcome: Some(RunOutcome::Defeat),
        result: "Defeat".to_owned(),
        seed: "BETA".to_owned(),
        combats: vec![CombatView {
            seq: 1,
            encounter: "ENC0".to_owned(),
            result: "completed".to_owned(),
            damage_dealt: 30,
            damage_taken: 10,
            turns: 3,
        }],
        rollup: vec![
            CardStat {
                id: "STRIKE".to_owned(),
                kind: SourceKind::Card,
                player: TEAM_SLOT,
                plays: 4,
                damage_dealt: 70,
                ..CardStat::default()
            },
            CardStat {
                id: "DEMON_FORM".to_owned(),
                kind: SourceKind::Power,
                player: TEAM_SLOT,
                plays: 1,
                damage_dealt: 35,
                dmg_direct: 35,
                ..CardStat::default()
            },
        ],
        ..RunSummaryView::default()
    };
    let base = view_fingerprint(&synthetic());
    assert_eq!(view_fingerprint(&synthetic()), base);
    let mut changed = synthetic();
    changed.character = "DEFECT".to_owned();
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.result = "Unfinished".to_owned();
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.seed = "OTHER".to_owned();
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.players.push(PlayerRec {
        slot: 1,
        character: "SILENT".to_owned(),
    });
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.combats[0].turns += 1;
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.rollup[1].plays += 1;
    assert_ne!(view_fingerprint(&changed), base);
    let mut changed = synthetic();
    changed.rollup[1].dmg_direct += 1;
    assert_ne!(view_fingerprint(&changed), base);
}

#[test]
fn per_player_rollups_split_the_run() {
    let base = scratch_dir("run-history-phase3-rollups");
    let data = std::path::Path::new(&base);
    let runs = r#"[{"run_id":7,"profile":2,"character":"IRONCLAD,SILENT","ascension":0,"game_mode":"Standard",
                        "outcome":"victory","seed":"P3","started_at":1786579200,
                        "ended_at":1786587000,"combats":1,
                        "players":[{"slot":0,"net_id":"1","character":"IRONCLAD"},
                                   {"slot":1,"net_id":"2","character":"SILENT"}]}]"#;
    let combats = r#"[{"combat_id":1,"encounter_id":"P3","result":"completed","turns":1,
                           "damage_received":0,"started_at":1786579200,
                           "run":{"seq":7,"character":"IRONCLAD,SILENT","ascension":0,"game_mode":"Standard"},
                           "cards":[
                            {"id":"STRIKE","kind":0,"player":0,"plays":1,"damage_dealt":5,"block_gained":0,"block_effective":0,"heal":0},
                            {"id":"STRIKE","kind":0,"player":1,"plays":2,"damage_dealt":7,"block_gained":0,"block_effective":0,"heal":0},
                            {"id":"THORNS_POWER","kind":2,"player":4,"plays":0,"damage_dealt":3,"block_gained":0,"block_effective":0,"heal":0}
                           ]}]"#;
    seed_data(data, runs, combats);

    let RunSelection::Selected(view) = select_run("P3", 1_786_579_200, 2) else {
        panic!("P3 must match");
    };
    assert_eq!(view.player_rollups.len(), 2);
    assert_eq!(view.player_rollups[0].slot, 0);
    assert_eq!(view.player_rollups[0].cards.len(), 1);
    assert_eq!(view.player_rollups[0].cards[0].id, "STRIKE");
    assert_eq!(view.player_rollups[0].cards[0].damage_dealt, 5);
    assert_eq!(view.player_rollups[1].slot, 1);
    assert_eq!(view.player_rollups[1].cards[0].damage_dealt, 7);
    // The team-merged rollup folds both players' STRIKEs and carries the
    // ownerless THORNS_POWER; there is no separate TEAM-scope rollup.
    assert_eq!(view.rollup.len(), 2);
    assert_eq!(view.rollup[0].id, "STRIKE");
    assert_eq!(view.rollup[0].player, TEAM_SLOT);
    assert_eq!(view.rollup[0].damage_dealt, 12);
}

#[test]
fn run_filter_toggle_selects_and_deselects_players() {
    let base = scratch_dir("run-history-phase3-filter");
    let data = std::path::Path::new(&base);
    let runs = r#"[{"run_id":8,"profile":2,"character":"A,B","ascension":0,"game_mode":"Standard",
                        "outcome":"victory","seed":"F","started_at":1786579200,
                        "ended_at":1786587000,"combats":0,
                        "players":[{"slot":0,"net_id":"1","character":"A"},
                                   {"slot":1,"net_id":"2","character":"B"}]}]"#;
    seed_data(data, runs, "[]");

    clear();
    assert!(select("F", 1_786_579_200, 2));
    let view = selected_view().expect("the selection stores the view");
    assert_eq!(run_filter(), PlayerFilter::All);
    toggle_run_filter(0);
    assert_eq!(run_filter(), PlayerFilter::Player(0));
    assert_eq!(filtered_rollup(&view), &view.player_rollups[0].cards[..]);
    toggle_run_filter(0);
    assert_eq!(run_filter(), PlayerFilter::All);
    assert_eq!(filtered_rollup(&view), &view.rollup[..]);
    toggle_run_filter(1);
    assert_eq!(run_filter(), PlayerFilter::Player(1));
    toggle_run_filter(0);
    assert_eq!(run_filter(), PlayerFilter::Player(0), "presses switch");
}

#[test]
fn run_filter_heals_when_the_roster_lacks_the_selected_slot() {
    let base = scratch_dir("run-history-phase3-heal");
    let data = std::path::Path::new(&base);
    let runs = r#"[{"run_id":9,"profile":2,"character":"A","ascension":0,"game_mode":"Standard",
                        "outcome":"victory","seed":"H","started_at":1786579200,
                        "ended_at":1786587000,"combats":0,
                        "players":[{"slot":0,"net_id":"1","character":"A"}]}]"#;
    seed_data(data, runs, "[]");

    clear();
    assert!(select("H", 1_786_579_200, 2));
    toggle_run_filter(1);
    assert_eq!(run_filter(), PlayerFilter::Player(1));
    heal_run_filter();
    assert_eq!(
        run_filter(),
        PlayerFilter::All,
        "a slot the view cannot render heals to All"
    );
    // The All filter is untouched by the heal.
    heal_run_filter();
    assert_eq!(run_filter(), PlayerFilter::All);
}
