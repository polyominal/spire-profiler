use std::fs;

use super::*;
use crate::data::run_history;
use crate::data::state::{CombatResult, RunOutcome};

const CACHE_RECORD: &str = r#"{"run_id":99,"seed":"CACHE","started_at":3000,"profile":2}"#;

fn marker_output(test_name: &str) -> Option<String> {
    if std::env::var_os("SPIRE_WRITE_OUTCOME_CHILD").is_some() {
        return None;
    }
    let test_name = test_name.strip_prefix("profiler_core::").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env("SPIRE_WRITE_OUTCOME_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stderr).unwrap())
}

#[test]
fn failed_combat_writes_finish_once_and_recover() {
    if let Some(stderr) = marker_output(concat!(
        module_path!(),
        "::failed_combat_writes_finish_once_and_recover"
    )) {
        assert!(!stderr.contains("combat 1 summary written"), "{stderr}");
        assert_eq!(stderr.matches("combat 2 summary written").count(), 4);
        return;
    }
    for stage in ["runs", "parent", "temp", "rename"] {
        exercise_combat_write_failure(stage, false);
    }
}

#[test]
fn failed_interrupted_writes_merge_once_and_recover() {
    if let Some(stderr) = marker_output(concat!(
        module_path!(),
        "::failed_interrupted_writes_merge_once_and_recover"
    )) {
        assert!(!stderr.contains("combat 1 summary written"), "{stderr}");
        assert_eq!(stderr.matches("combat 2 summary written").count(), 4);
        return;
    }
    for stage in ["runs", "parent", "temp", "rename"] {
        exercise_combat_write_failure(stage, true);
    }
}

fn exercise_combat_write_failure(stage: &str, interrupted: bool) {
    let base = wiped_dir(&format!("combat-write-{stage}-{interrupted}"));
    test_reset();
    init(&base);
    set_run_meta(2);
    run_started("IRONCLAD", 0, "Standard", "ACTIVE", 0, "", 1000);
    combat_started("FAILED", "test");
    turn_started();
    block_gained(5, "DEFEND", 0, 0);
    assert!(!run_history::select("CACHE", 3000, 2));
    fs::write(base.join("runs.jsonl"), CACHE_RECORD).unwrap();
    let blocker = base.join(match stage {
        "runs" => "runs",
        "parent" => "runs/1",
        "temp" => "runs/1/1.json.tmp",
        _ => "runs/1/1.json",
    });
    if stage == "runs" {
        fs::remove_dir(&blocker).unwrap();
    }
    if matches!(stage, "runs" | "parent") {
        fs::write(&blocker, "preserved").unwrap();
    } else {
        fs::create_dir_all(&blocker).unwrap();
        fs::write(blocker.join("old"), "preserved").unwrap();
    }
    if interrupted {
        combat_started("RECOVERY", "test");
    } else {
        combat_ended();
        combat_ended();
    }
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!((state.run_combats, state.run_turns), (1, 1));
        assert_eq!(state.run_cards[0].block_gained, 5);
        let combat = state.current.as_ref().unwrap();
        assert_eq!(
            combat.result(),
            (!interrupted).then_some(CombatResult::Completed)
        );
    });
    assert!(!run_history::select("CACHE", 3000, 2));
    assert!(!read_test_file(&base, "profiler.log").contains("combat 1 ended:"));
    if blocker.is_dir() {
        assert_eq!(
            fs::read_to_string(blocker.join("old")).unwrap(),
            "preserved"
        );
        fs::remove_dir_all(&blocker).unwrap();
    } else {
        assert_eq!(fs::read_to_string(&blocker).unwrap(), "preserved");
        fs::remove_file(&blocker).unwrap();
    }
    assert!(!base.join("runs/1/1.json.tmp").exists());
    if !interrupted {
        combat_ended();
        combat_started("RECOVERY", "test");
    }
    turn_started();
    block_gained(7, "DEFEND", 0, 0);
    combat_ended();
    combat_ended();
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!((state.run_combats, state.run_turns), (2, 2));
        assert_eq!(state.run_cards[0].block_gained, 12);
    });
    assert_eq!(combat_ids(&base.join("runs")), vec![(1, 2)]);
    assert_eq!(read_combat(&base).0.cards[0].block_gained, 7);
    assert!(run_history::select("CACHE", 3000, 2));
}

#[test]
fn failed_run_writes_close_once_and_recover() {
    if let Some(stderr) = marker_output(concat!(
        module_path!(),
        "::failed_run_writes_close_once_and_recover"
    )) {
        assert!(!stderr.contains("run 1 recorded"), "{stderr}");
        assert_eq!(stderr.matches("run 100 recorded (victory)").count(), 3);
        return;
    }
    for stage in ["empty", "unreadable", "temp"] {
        let base = wiped_dir(&format!("run-write-{stage}"));
        test_reset();
        init(&base);
        set_run_meta(2);
        run_started("IRONCLAD", 0, "Standard", "ACTIVE", 0, "", 1000);
        if stage != "empty" {
            combat_started("SAVED", "test");
            combat_ended();
        }
        assert!(!run_history::select("CACHE", 3000, 2));
        let old = if stage == "unreadable" {
            &[0xff][..]
        } else {
            CACHE_RECORD.as_bytes()
        };
        fs::write(base.join("runs.jsonl"), old).unwrap();
        let tmp = base.join("runs.jsonl.tmp");
        if stage == "temp" {
            fs::create_dir(&tmp).unwrap();
        }
        run_ended(RunOutcome::Defeat);
        run_ended(RunOutcome::Victory);
        assert!(STATE.with(|cell| cell.borrow().run_ctx.is_none()));
        assert_eq!(fs::read(base.join("runs.jsonl")).unwrap(), old);
        assert!(!run_history::select("CACHE", 3000, 2));
        assert!(!read_test_file(&base, "profiler.log").contains("run 1 ended:"));
        if stage == "temp" {
            fs::remove_dir(&tmp).unwrap();
        }
        fs::write(base.join("runs.jsonl"), CACHE_RECORD).unwrap();
        run_ended(RunOutcome::Victory);
        assert_eq!(read_test_file(&base, "runs.jsonl"), CACHE_RECORD);
        run_started("DEFECT", 0, "Standard", "RECOVERY", 0, "", 2000);
        combat_started("RECOVERY", "test");
        combat_ended();
        assert!(run_history::select("RECOVERY", 2000, 2));
        assert_eq!(run_history::selected_view().unwrap().outcome, None);
        run_ended(RunOutcome::Victory);
        run_ended(RunOutcome::Victory);
        let runs = read_all_runs(&base);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0]["run_id"], 99);
        assert_eq!(runs[1]["run_id"], 100);
        assert_eq!(runs[1]["outcome"], "victory");
        assert!(run_history::select("RECOVERY", 2000, 2));
        assert_eq!(
            run_history::selected_view().unwrap().outcome,
            Some(RunOutcome::Victory)
        );
        assert!(!tmp.exists());
    }
}
