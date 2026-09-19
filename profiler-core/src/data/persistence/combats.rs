//! The combat store: one atomic, write-once file per combat under
//! `runs/<run_id>/`, named by its globally-unique id. Paths locate a run's
//! records; persisted header identities validate their membership before
//! history or resume aggregation. Per-run reads scan one directory;
//! combats outside any run land in `runs/0/`.

use std::fs;
use std::path::Path;

use super::MAX_JSON_SIZE;
use super::combat_doc::build_combat_json;
use super::io::{ReadFile, ensure_data_dir, read_dir, read_file, write_file};
use super::runs::merge_into_run;
use crate::data::persistence::event_log;
use crate::data::records;
use crate::data::state::{Combat, STATE};
use crate::fail;

pub(crate) struct StoredCombatDoc {
    path_run_id: u32,
    path_combat_id: u32,
    doc: Box<str>,
}

/// The `<digits>.json` combat-file ids in one directory, sorted; anything
/// else is skipped.
fn scan_combat_ids(dir: &Path) -> Option<Vec<u32>> {
    let entries = read_dir(dir)?;
    let mut ids: Vec<u32> = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if !stem.is_empty()
            && stem.bytes().all(|b| b.is_ascii_digit())
            && let Ok(id) = stem.parse::<u32>()
        {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    Some(ids)
}

/// The highest id in the store; boot seeds `next_combat_id` with this.
pub(crate) fn max_combat_id() -> Option<u32> {
    let base = STATE.with(|s| {
        s.borrow()
            .store_paths
            .as_ref()
            .map(|paths| paths.runs_dir.clone())
    })?;
    let entries = read_dir(&base)?;
    let mut max = 0u32;
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(run_seq) = name.parse::<u32>() else {
            continue;
        };
        for id in scan_combat_ids(&base.join(run_seq.to_string()))? {
            max = max.max(id);
        }
    }
    Some(max)
}

/// One run's documents in id order; unreadable files are skipped.
pub(crate) fn load_run_combat_docs(run_id: u32) -> impl Iterator<Item = StoredCombatDoc> {
    let dir = STATE.with(|s| {
        s.borrow()
            .store_paths
            .as_ref()
            .map(|paths| paths.runs_dir.join(run_id.to_string()))
    });
    dir.into_iter().flat_map(move |dir| {
        scan_combat_ids(&dir)
            .unwrap_or_default()
            .into_iter()
            .filter_map(move |id| {
                let ReadFile::Content(content) = read_file(&dir.join(format!("{id}.json"))) else {
                    return None;
                };
                Some(StoredCombatDoc {
                    path_run_id: run_id,
                    path_combat_id: id,
                    doc: content.into_boxed_str(),
                })
            })
    })
}

/// The whole history, oldest first.
pub(crate) fn load_combat_docs_from(dir: &Path) -> impl Iterator<Item = StoredCombatDoc> + '_ {
    let mut ids: Vec<(u32, u32)> = Vec::new(); // (run id, combat id)
    for entry in fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(run_seq) = name.parse::<u32>() else {
            continue;
        };
        for id in scan_combat_ids(&dir.join(run_seq.to_string())).unwrap_or_default() {
            ids.push((run_seq, id));
        }
    }
    // Global id order, whatever order the run directories come in.
    ids.sort_unstable_by_key(|&(_, id)| id);
    ids.into_iter().filter_map(move |(run_seq, id)| {
        let path = dir.join(run_seq.to_string()).join(format!("{id}.json"));
        let ReadFile::Content(content) = read_file(&path) else {
            return None;
        };
        Some(StoredCombatDoc {
            path_run_id: run_seq,
            path_combat_id: id,
            doc: content.into_boxed_str(),
        })
    })
}

/// Test-only alias for [`load_combat_docs_from`] on the configured store.
#[cfg(test)]
pub(crate) fn load_all_combat_docs() -> Box<[Box<str>]> {
    let Some(dir) = STATE.with(|s| {
        s.borrow()
            .store_paths
            .as_ref()
            .map(|paths| paths.runs_dir.clone())
    }) else {
        return Box::default();
    };
    load_combat_docs_from(&dir)
        .map(|stored| stored.doc)
        .collect()
}

/// Holds one raw document at a time; bad records are fail-logged and skipped.
pub(crate) fn parse_combat_docs(
    docs: impl IntoIterator<Item = StoredCombatDoc>,
) -> impl Iterator<Item = records::CombatRec> {
    docs.into_iter().filter_map(|stored| {
        let combat = match records::parse_combat_doc(&stored.doc) {
            Ok(combat) => combat,
            Err(err) => {
                fail!("cannot parse a runs/ combat file: {err}");
                return None;
            }
        };
        combat_identity_matches_path(&combat, &stored).then_some(combat)
    })
}

fn combat_identity_matches_path(combat: &records::CombatRec, stored: &StoredCombatDoc) -> bool {
    if combat.combat_id != stored.path_combat_id {
        fail!(
            "combat {} is stamped {} but filed as {}",
            stored.path_combat_id,
            combat.combat_id,
            stored.path_combat_id
        );
        return false;
    }
    let stamped_run = combat.run.as_ref().map(|run| run.seq);
    let path_run = (stored.path_run_id != 0).then_some(stored.path_run_id);
    if stamped_run != path_run {
        let stamped = stamped_run
            .map(|seq| seq.to_string())
            .unwrap_or_else(|| "no run".to_owned());
        fail!(
            "combat {} is stamped run {stamped} but filed under run {}",
            stored.path_combat_id,
            stored.path_run_id
        );
        return false;
    }
    true
}

/// Atomic and write-once; a form crossing [`MAX_JSON_SIZE`] is refused.
pub fn write_combat_file(c: &Combat) -> bool {
    merge_into_run(c);
    if !ensure_data_dir() {
        return false;
    }
    let combat_json = build_combat_json(c);
    if combat_json.len() > MAX_JSON_SIZE {
        fail!("combat {} JSON overflow; combat not written", c.seq);
        return false;
    }
    let Some(path) = STATE.with(|s| {
        s.borrow().store_paths.as_ref().map(|paths| {
            paths
                .runs_dir
                .join(c.run.as_ref().map_or(0, |run| run.seq).to_string())
                .join(format!("{}.json", c.seq))
        })
    }) else {
        return false;
    };
    let parent = path
        .parent()
        .expect("a store path always has a parent directory");
    if let Err(err) = fs::create_dir_all(parent) {
        fail!(
            "cannot create combat store directory '{}': {} (os error {})",
            parent.display(),
            err.kind(),
            err.raw_os_error().unwrap_or(-1)
        );
        return false;
    }
    if !write_file(&path, &combat_json) {
        return false;
    }
    crate::data::run_history::invalidate();
    event_log!(
        "combat {} ended: {} ({}), {} plays, {} cards tracked; stored at {}",
        c.seq,
        c.encounter_id,
        c.result()
            .expect("only a finished combat record reaches the writer")
            .name(),
        c.plays,
        c.cards.len(),
        path.display()
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;
    use crate::data::state::{EndedRun, RunContext, RunOutcome};
    use crate::test_util::{combat_ids, unique_dir};

    fn store_ids(data: &std::path::Path) -> Vec<u32> {
        combat_ids(&data.join("runs"))
            .into_iter()
            .map(|(_, id)| id)
            .collect()
    }

    #[test]
    fn reset_disables_store_reads_and_writes() {
        let data = unique_dir("store-reset");
        init_state(&data);
        let combat = synthetic_combat();
        assert!(write_combat_file(&combat));
        let path = data.join("runs/42/7.json");
        let before = fs::read(&path).expect("combat was written");

        crate::data::events::test_reset();

        assert_eq!(max_combat_id(), None);
        assert!(load_run_combat_docs(42).next().is_none());
        assert!(load_all_combat_docs().is_empty());
        assert!(!ensure_data_dir());
        let mut replacement = synthetic_combat();
        replacement.encounter_id = "REPLACEMENT".into();
        assert!(!write_combat_file(&replacement));
        assert!(!crate::data::persistence::write_run_record(&EndedRun {
            context: RunContext {
                run: synthetic_run(42),
                players: synthetic_roster(),
            },
            outcome: RunOutcome::Victory,
            ended_at: 2000,
        }));
        assert_eq!(fs::read(path).expect("original combat remains"), before);
        assert!(!data.join("runs.jsonl").exists());
    }

    #[test]
    fn write_combat_file_lands_one_file_per_combat() {
        let dir = unique_dir("store-write");
        let data = dir.join("data");
        init_state(&data);
        let c = synthetic_combat(); // seq 7, run 42, as combat_started would assign
        assert!(write_combat_file(&c));
        let path = data.join("runs/42/7.json");
        let content = fs::read_to_string(&path).expect("store file written");
        assert_eq!(content, build_combat_json(&c));
        assert_eq!(store_ids(&data), [7]);
        let log = fs::read_to_string(data.join("profiler.log")).unwrap();
        assert!(log.contains("combat 7 ended: BYGONE_EFFIGY (completed)"));
        assert!(log.contains("stored at "));
    }

    #[test]
    fn write_combat_file_refuses_oversized_json() {
        let data = unique_dir("combat-write-overflow");
        init_state(&data);
        let mut combat = synthetic_combat();
        combat.encounter_id = "x".repeat(MAX_JSON_SIZE).into();
        assert!(!write_combat_file(&combat));
        assert!(!data.join("runs/42").exists());
        assert!(!data.join("profiler.log").exists());
    }

    #[test]
    fn write_combat_file_uses_the_assigned_global_id() {
        let dir = unique_dir("store-id");
        let data = dir.join("data");
        init_state(&data);
        let mut c = synthetic_combat();
        c.seq = 412;
        write_combat_file(&c);
        let mut c2 = synthetic_combat();
        c2.seq = 418;
        c2.encounter_id = "FROZEN_COUNCIL".into();
        write_combat_file(&c2);
        assert_eq!(store_ids(&data), [412, 418]);
        assert!(data.join("runs/42/412.json").exists());
        assert!(data.join("runs/42/418.json").exists());
    }

    #[test]
    fn write_combat_outside_a_run_lands_in_runs_zero() {
        let dir = unique_dir("store-run-zero");
        let data = dir.join("data");
        init_state(&data);
        let mut c = synthetic_combat();
        c.run = None;
        write_combat_file(&c);
        let path = data.join("runs/0/7.json");
        let content = fs::read_to_string(&path).expect("store file written");
        assert_eq!(content, build_combat_json(&c));
        assert_eq!(store_ids(&data), [7]);
    }

    #[test]
    fn max_combat_id_reads_filenames_only() {
        let dir = unique_dir("store-max");
        let data = dir.join("data");
        init_state(&data);
        fs::create_dir_all(data.join("runs/1")).unwrap();
        fs::create_dir_all(data.join("runs/3")).unwrap();
        fs::write(data.join("runs/1/5.json"), "{}").unwrap();
        fs::write(data.join("runs/1/garbage.json"), "{}").unwrap();
        fs::write(data.join("runs/1/12.json.tmp"), "{}").unwrap();
        fs::write(data.join("runs/3/7.json"), "{}").unwrap();
        fs::create_dir_all(data.join("runs/profile-1")).unwrap();
        assert_eq!(max_combat_id(), Some(7));
    }

    #[test]
    fn same_seed_replays_never_collide() {
        let dir = unique_dir("store-replay");
        let data = dir.join("data");
        init_state(&data);
        let mut a1 = synthetic_combat();
        a1.seq = 1;
        let mut a2 = synthetic_combat();
        a2.seq = 2;
        write_combat_file(&a1);
        write_combat_file(&a2);
        STATE.with(|s| s.borrow_mut().next_combat_id = max_combat_id());
        assert_eq!(
            STATE.with(|s| s.borrow().next_combat_id),
            Some(2),
            "the counter re-seeds to the store's highest id"
        );
        let mut b1 = synthetic_combat();
        b1.seq = 3;
        b1.run = Some(synthetic_run(43));
        let mut b2 = synthetic_combat();
        b2.seq = 4;
        b2.run = Some(synthetic_run(43));
        b2.encounter_id = "FROZEN_COUNCIL".into();
        write_combat_file(&b1);
        write_combat_file(&b2);
        assert_eq!(
            store_ids(&data),
            [1, 2, 3, 4],
            "no attempt-B file overwrote an attempt-A file"
        );
        assert_eq!(max_combat_id(), Some(4));
        let mut a_files: Vec<String> = fs::read_dir(data.join("runs/42"))
            .expect("run 42 dir exists")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        a_files.sort();
        assert!(a_files.iter().map(String::as_str).eq(["1.json", "2.json"]));
        let mut b_files: Vec<String> = fs::read_dir(data.join("runs/43"))
            .expect("run 43 dir exists")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        b_files.sort();
        assert!(b_files.iter().map(String::as_str).eq(["3.json", "4.json"]));
    }

    #[test]
    fn load_all_combat_docs_sorts_by_id_and_skips_garbage() {
        let dir = unique_dir("store-load");
        let data = dir.join("data");
        init_state(&data);
        fs::create_dir_all(data.join("runs/2")).unwrap();
        fs::create_dir_all(data.join("runs/1")).unwrap();
        fs::write(data.join("runs/2/10.json"), r#"{"combat_id":10}"#).unwrap();
        fs::write(data.join("runs/1/2.json"), r#"{"combat_id":2}"#).unwrap();
        fs::write(data.join("runs/1/1.json"), r#"{"combat_id":1}"#).unwrap();
        fs::write(data.join("runs/1/junk.json"), "{not json").unwrap();
        let docs = load_all_combat_docs();
        assert_eq!(docs.len(), 3);
        assert!(docs[0].contains(r#""combat_id":1"#));
        assert!(docs[1].contains(r#""combat_id":2"#));
        assert!(docs[2].contains(r#""combat_id":10"#));
    }

    #[test]
    fn load_run_combat_docs_reads_only_that_run() {
        let dir = unique_dir("store-run-load");
        let data = dir.join("data");
        init_state(&data);
        fs::create_dir_all(data.join("runs/7")).unwrap();
        fs::create_dir_all(data.join("runs/9")).unwrap();
        fs::write(data.join("runs/7/2.json"), r#"{"combat_id":2}"#).unwrap();
        fs::write(data.join("runs/7/1.json"), r#"{"combat_id":1}"#).unwrap();
        fs::write(data.join("runs/9/99.json"), r#"{"combat_id":99}"#).unwrap();
        let docs: Vec<_> = load_run_combat_docs(7).collect();
        assert_eq!(docs.len(), 2);
        assert!(docs[0].doc.contains(r#""combat_id":1"#));
        assert!(docs[1].doc.contains(r#""combat_id":2"#));
        assert_eq!(load_run_combat_docs(9).count(), 1);
        assert!(
            load_run_combat_docs(5).next().is_none(),
            "absent run dir is empty"
        );
    }

    #[test]
    fn parse_combat_docs_rejects_path_identity_mismatches() {
        let dir = unique_dir("store-identity");
        let data = dir.join("data");
        init_state(&data);
        fs::create_dir_all(data.join("runs/5")).unwrap();
        fs::create_dir_all(data.join("runs/0")).unwrap();
        fs::write(
            data.join("runs/5/4.json"),
            r#"{"combat_id":4,"run":{"seq":6}}"#,
        )
        .unwrap();
        fs::write(
            data.join("runs/5/5.json"),
            r#"{"combat_id":6,"run":{"seq":5}}"#,
        )
        .unwrap();
        fs::write(data.join("runs/0/7.json"), r#"{"combat_id":7}"#).unwrap();

        let combats: Vec<_> =
            parse_combat_docs(load_combat_docs_from(&data.join("runs"))).collect();
        assert_eq!(combats.len(), 1);
        assert_eq!(combats[0].combat_id, 7);
        assert!(combats[0].run.is_none());
    }

    #[test]
    fn combat_reads_are_deferred_and_skip_bad_records_in_id_order() {
        let data = unique_dir("store-stream");
        init_state(&data);
        write_store_file(&data, 0, 1, r#"{"combat_id":1}"#);
        write_store_file(&data, 0, 2, "invalid JSON");
        write_store_file(&data, 0, 3, r#"{"combat_id":3,"turns":1}"#);
        let runs = data.join("runs");
        let mut combats = parse_combat_docs(load_combat_docs_from(&runs));
        assert_eq!(combats.next().expect("first record is valid").combat_id, 1);

        write_store_file(&data, 0, 3, r#"{"combat_id":3,"turns":7}"#);
        let last = combats.next().expect("bad record cannot hide the next one");
        assert_eq!((last.combat_id, last.turns), (3, 7));
        assert!(combats.next().is_none());

        let mut docs = load_run_combat_docs(0);
        assert_eq!(
            docs.next().expect("first document exists").path_combat_id,
            1
        );
        fs::remove_file(runs.join("0/2.json")).expect("second file exists");
        assert_eq!(
            docs.next().expect("third document exists").path_combat_id,
            3
        );
        assert!(docs.next().is_none());
    }
}
