//! The on-disk write entry point the events layer calls besides the combat
//! finalizer: the run finalizer, which rewrites `runs.jsonl` atomically.

use super::combats::{load_run_combat_docs, parse_combat_docs};
use super::io::{ReadFile, read_file, write_file};
use crate::data::persistence::event_log;
use crate::data::records;
use crate::data::state::{EndedRun, STATE};

/// The record comes from the run context: identity and header facts only.
pub fn write_run_record(ended: &EndedRun) {
    let run = &ended.context.run;
    if parse_combat_docs(&load_run_combat_docs(run.seq)).is_empty() {
        event_log!("run {} ended with no combat records", run.seq);
        return;
    }

    let (profile, runs_path) = STATE.with(|s| {
        let st = s.borrow();
        (st.run_profile, st.runs_path_full.clone())
    });
    let line = records::build_run_json(ended, profile);
    let mut content = match read_file(&runs_path) {
        ReadFile::Missing => String::new(),
        ReadFile::Content(content) => content,
        ReadFile::Failed => return,
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&line);
    content.push('\n');
    write_file(&runs_path, &content);

    crate::data::run_history::invalidate();

    event_log!(
        "run {} ended: {} ({}), {}",
        run.seq,
        run.character,
        run.game_mode,
        ended.outcome.name(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::build_combat_json;
    use crate::data::persistence::test_support::*;
    use crate::data::state::{RunContext, RunOutcome, RunSnapshot};

    #[test]
    fn write_run_record_appends_one_line_per_run() {
        let dir = unique_dir("run-record");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_profile = 3;
        });
        let ended = EndedRun {
            context: RunContext {
                run: RunSnapshot {
                    seq: 42,
                    character: "SHROUD".to_owned(),
                    ascension: 5,
                    game_mode: "standard".to_owned(),
                    seed: "SEED123".to_owned(),
                },
                started_at: 1_786_624_000,
                players: synthetic_roster(),
            },
            outcome: RunOutcome::Victory,
            ended_at: 1_786_624_496,
        };
        let c = synthetic_combat(); // run 42, seq 7
        write_store_file(&data, 42, 7, &build_combat_json(&c));

        write_run_record(&ended);

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

        let ended = EndedRun {
            context: RunContext {
                run: RunSnapshot {
                    seq: 43,
                    character: "IRONCLAD".to_owned(),
                    ..RunSnapshot::default()
                },
                ..RunContext::default()
            },
            outcome: RunOutcome::Defeat,
            ended_at: 1_786_624_996,
        };
        let mut c2 = synthetic_combat();
        c2.run = Some(synthetic_run(43));
        c2.seq = 8;
        write_store_file(&data, 43, 8, &build_combat_json(&c2));
        write_run_record(&ended);
        let content = std::fs::read_to_string(data.join("runs.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains(r#""run_id":43"#));
    }

    #[test]
    fn write_run_record_with_no_combats_logs_and_writes_nothing() {
        let dir = unique_dir("run-record-empty");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        write_run_record(&EndedRun {
            context: RunContext {
                run: RunSnapshot {
                    seq: 42,
                    ..RunSnapshot::default()
                },
                ..RunContext::default()
            },
            outcome: RunOutcome::Defeat,
            ended_at: 1_786_624_996,
        });
        assert!(!data.join("runs.jsonl").exists());
        let log = std::fs::read_to_string(data.join("profiler.log")).unwrap();
        assert!(log.contains("run 42 ended with no combat records"));
    }

    #[test]
    fn write_run_record_preserves_unreadable_history() {
        let dir = unique_dir("run-record-unreadable");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        let c = synthetic_combat();
        write_store_file(&data, 42, 7, &build_combat_json(&c));
        let runs_path = data.join("runs.jsonl");
        std::fs::write(&runs_path, [0xff]).unwrap();

        write_run_record(&EndedRun {
            context: RunContext {
                run: synthetic_run(42),
                ..RunContext::default()
            },
            outcome: RunOutcome::Victory,
            ended_at: 1_786_624_496,
        });

        assert_eq!(std::fs::read(&runs_path).unwrap(), vec![0xff]);
        assert!(!runs_path.with_extension("jsonl.tmp").exists());
    }
}
