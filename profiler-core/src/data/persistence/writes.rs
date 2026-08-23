//! The on-disk write entry point the events layer calls besides the combat
//! finalizer: the run finalizer, which rewrites `runs.jsonl` atomically.

use super::combats::load_run_combat_docs;
use super::io::{read_file, write_file};
use super::log::append_log;
use crate::data::records;
use crate::data::state::STATE;
use crate::fail;

/// Parses one run's combat docs into combat records.
fn parse_run(seq: u32) -> Vec<records::CombatRec> {
    let mut combats = Vec::new();
    for doc in load_run_combat_docs(seq) {
        match records::parse_combat_doc(&doc) {
            Ok(combat) => combats.push(combat),
            Err(err) => fail(format!("cannot parse a runs/{seq} combat file: {err}")),
        }
    }
    combats
}

/// The record comes from the run context: identity and header facts only.
pub fn write_run_record() {
    let run_ctx = STATE.with(|s| s.borrow().run_ctx.clone());
    if parse_run(run_ctx.seq).is_empty() {
        append_log(format!(
            "run {} ended with no combat records\n",
            run_ctx.seq
        ));
        return;
    }

    let (profile, runs_path) = STATE.with(|s| {
        let st = s.borrow();
        (st.run_profile, st.runs_path_full.clone())
    });
    let line = records::build_run_json(&run_ctx, profile);
    let mut content = read_file(&runs_path).unwrap_or_default();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&line);
    content.push('\n');
    write_file(&runs_path, &content);

    crate::data::run_history::invalidate();

    append_log(format!(
        "run {} ended: {} ({}), {}\n",
        run_ctx.seq,
        run_ctx.character,
        run_ctx.game_mode,
        run_ctx.outcome.name(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::build_combat_json;
    use crate::data::persistence::test_support::*;
    use crate::data::state::RunOutcome;

    #[test]
    fn write_run_record_appends_one_line_per_run() {
        let dir = temp_dir("run-record");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_profile = 3;
            st.run_ctx.active = true;
            st.run_ctx.seq = 42;
            st.run_ctx.character = "SHROUD".to_owned();
            st.run_ctx.ascension = 5;
            st.run_ctx.game_mode = "standard".to_owned();
            st.run_ctx.seed = "SEED123".to_owned();
            st.run_ctx.started_at = 1_786_624_000;
            st.run_ctx.ended_at = 1_786_624_496;
            st.run_ctx.outcome = RunOutcome::Victory;
            st.run_ctx.players = synthetic_roster();
        });
        let c = synthetic_combat(); // run_seq 42, seq 7
        write_store_file(&data, 42, 7, &build_combat_json(&c));

        write_run_record();

        let content = std::fs::read_to_string(data.join("runs.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "one line per run");
        let runs: serde_json::Value = serde_json::from_str(lines[0]).expect("line parses");
        assert_eq!(runs["run_id"], 42);
        assert_eq!(runs["profile"], 3);
        assert_eq!(runs["character"], "SHROUD");
        assert_eq!(runs["ascension"], 5);
        assert_eq!(runs["game_mode"], "standard");
        assert_eq!(runs["outcome"], "victory");
        assert_eq!(runs["seed"], "SEED123");
        assert_eq!(runs["started_at"], 1_786_624_000);
        assert_eq!(runs["ended_at"], 1_786_624_496);
        assert_eq!(runs["players"][0]["slot"], 0);
        assert_eq!(runs["players"][0]["character"], "SHROUD");
        assert!(runs["players"][0].get("net_id").is_none());
        assert!(runs.get("cards").is_none());
        assert!(runs.get("combats").is_none());
        assert!(runs.get("build").is_none());
        let log = std::fs::read_to_string(data.join("profiler.log")).unwrap();
        assert!(log.contains("run 42 ended: SHROUD (standard), victory"));

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.seq = 43;
            st.run_ctx.character = "IRONCLAD".to_owned();
            st.run_ctx.outcome = RunOutcome::Defeat;
        });
        let mut c2 = synthetic_combat();
        c2.run_seq = 43;
        c2.seq = 8;
        write_store_file(&data, 43, 8, &build_combat_json(&c2));
        write_run_record();
        let content = std::fs::read_to_string(data.join("runs.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains(r#""run_id":43"#));
    }

    #[test]
    fn write_run_record_with_no_combats_logs_and_writes_nothing() {
        let dir = temp_dir("run-record-empty");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.seq = 42;
        });
        write_run_record();
        assert!(!data.join("runs.jsonl").exists());
        let log = std::fs::read_to_string(data.join("profiler.log")).unwrap();
        assert!(log.contains("run 42 ended with no combat records"));
    }
}
