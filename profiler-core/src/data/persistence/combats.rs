//! The combat store: one atomic, write-once file per combat under
//! `runs/<run_id>/`, named by its globally-unique id. Run membership is
//! structural — the path IS the run — so the per-run paths (run-end
//! summary, save+quit resume rebuild, next-run-id derivation) read one
//! run's directory instead of the whole history. Combats outside any run
//! land in `runs/0/`.

use std::fs;
use std::path::{Path, PathBuf};

use super::combat_doc::build_combat_json;
use super::io::{ReadFile, ensure_data_dir, read_file, write_file};
use super::runs::merge_into_run;
use super::{MAX_JSON_SIZE, RUNS_DIR_NAME};
use crate::data::persistence::event_log;
use crate::data::records;
use crate::data::state::{Combat, STATE};
use crate::fail;

fn combat_path(run_seq: u32, id: u32) -> PathBuf {
    STATE.with(|s| {
        s.borrow()
            .data_dir
            .join(RUNS_DIR_NAME)
            .join(run_seq.to_string())
            .join(format!("{id}.json"))
    })
}

pub(crate) struct StoredCombatDoc {
    path_run_id: u32,
    path_combat_id: u32,
    doc: String,
}

/// The `<digits>.json` combat-file ids in one directory, sorted; anything
/// else is skipped.
fn scan_combat_ids(dir: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<u32> = Vec::new();
    for entry in entries.flatten() {
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
    ids
}

/// The highest id in the store; boot seeds `next_combat_id` with this.
pub(crate) fn max_combat_id() -> u32 {
    let base = STATE.with(|s| s.borrow().data_dir.join(RUNS_DIR_NAME));
    let Ok(entries) = fs::read_dir(&base) else {
        return 0;
    };
    let mut max = 0u32;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(run_seq) = name.parse::<u32>() else {
            continue;
        };
        for id in scan_combat_ids(&base.join(run_seq.to_string())) {
            max = max.max(id);
        }
    }
    max
}

/// One run's documents in id order; unreadable files are skipped.
pub(crate) fn load_run_combat_docs(run_id: u32) -> Vec<StoredCombatDoc> {
    let dir = STATE.with(|s| {
        s.borrow()
            .data_dir
            .join(RUNS_DIR_NAME)
            .join(run_id.to_string())
    });
    let mut docs = Vec::new();
    for id in scan_combat_ids(&dir) {
        if let ReadFile::Content(content) = read_file(&dir.join(format!("{id}.json"))) {
            docs.push(StoredCombatDoc {
                path_run_id: run_id,
                path_combat_id: id,
                doc: content,
            });
        }
    }
    docs
}

/// The whole history, oldest first.
pub(crate) fn load_combat_docs_from(dir: &Path) -> Vec<StoredCombatDoc> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<(u32, u32)> = Vec::new(); // (run id, combat id)
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(run_seq) = name.parse::<u32>() else {
            continue;
        };
        for id in scan_combat_ids(&dir.join(run_seq.to_string())) {
            ids.push((run_seq, id));
        }
    }
    // Global id order, whatever order the run directories come in.
    ids.sort_unstable_by_key(|&(_, id)| id);
    let mut docs = Vec::with_capacity(ids.len());
    for (run_seq, id) in ids {
        let path = dir.join(run_seq.to_string()).join(format!("{id}.json"));
        if let ReadFile::Content(content) = read_file(&path) {
            docs.push(StoredCombatDoc {
                path_run_id: run_seq,
                path_combat_id: id,
                doc: content,
            });
        }
    }
    docs
}

/// Test-only alias for [`load_combat_docs_from`] on the configured store.
#[cfg(test)]
pub(crate) fn load_all_combat_docs() -> Vec<String> {
    let dir = STATE.with(|s| s.borrow().data_dir.join(RUNS_DIR_NAME));
    load_combat_docs_from(&dir)
        .into_iter()
        .map(|stored| stored.doc)
        .collect()
}

/// Parses store documents in order; a bad document is fail-logged and
/// skipped, never fatal to the rest of the store.
pub(crate) fn parse_combat_docs(docs: &[StoredCombatDoc]) -> Vec<records::CombatRec> {
    let mut combats = Vec::new();
    for stored in docs {
        let combat = match records::parse_combat_doc(&stored.doc) {
            Ok(combat) => combat,
            Err(err) => {
                fail!("cannot parse a runs/ combat file: {err}");
                continue;
            }
        };
        if combat_identity_matches_path(&combat, stored) {
            combats.push(combat);
        }
    }
    combats
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
pub fn write_combat_file(c: &Combat) {
    if !ensure_data_dir() {
        return;
    }
    merge_into_run(c);
    let combat_json = build_combat_json(c);
    if combat_json.len() > MAX_JSON_SIZE {
        fail!("combat {} JSON overflow; combat not written", c.seq);
        return;
    }
    let path = combat_path(c.run.as_ref().map_or(0, |run| run.seq), c.seq);
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
        return;
    }
    write_file(&path, &combat_json);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;
    use crate::test_util::combat_ids;

    fn store_ids(data: &std::path::Path) -> Vec<u32> {
        combat_ids(&data.join("runs"))
            .into_iter()
            .map(|(_, id)| id)
            .collect()
    }

    #[test]
    fn write_combat_file_lands_one_file_per_combat() {
        let dir = unique_dir("store-write");
        let data = dir.join("data");
        init_state(&data);
        let c = synthetic_combat(); // seq 7, run 42, as combat_started would assign
        write_combat_file(&c);
        let path = data.join("runs/42/7.json");
        let content = fs::read_to_string(&path).expect("store file written");
        assert_eq!(content, build_combat_json(&c));
        assert_eq!(store_ids(&data), vec![7]);
        let log = fs::read_to_string(data.join("profiler.log")).unwrap();
        assert!(log.contains("combat 7 ended: BYGONE_EFFIGY (completed)"));
        assert!(log.contains("stored at "));
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
        c2.encounter_id = "FROZEN_COUNCIL".to_owned();
        write_combat_file(&c2);
        assert_eq!(store_ids(&data), vec![412, 418]);
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
        assert_eq!(store_ids(&data), vec![7]);
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
        assert_eq!(max_combat_id(), 7);
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
            2,
            "the counter re-seeds to the store's highest id"
        );
        let mut b1 = synthetic_combat();
        b1.seq = 3;
        b1.run = Some(synthetic_run(43));
        let mut b2 = synthetic_combat();
        b2.seq = 4;
        b2.run = Some(synthetic_run(43));
        b2.encounter_id = "FROZEN_COUNCIL".to_owned();
        write_combat_file(&b1);
        write_combat_file(&b2);
        assert_eq!(
            store_ids(&data),
            vec![1, 2, 3, 4],
            "no attempt-B file overwrote an attempt-A file"
        );
        assert_eq!(max_combat_id(), 4);
        let mut a_files: Vec<String> = fs::read_dir(data.join("runs/42"))
            .expect("run 42 dir exists")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        a_files.sort();
        assert_eq!(a_files, vec!["1.json".to_owned(), "2.json".to_owned()]);
        let mut b_files: Vec<String> = fs::read_dir(data.join("runs/43"))
            .expect("run 43 dir exists")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        b_files.sort();
        assert_eq!(b_files, vec!["3.json".to_owned(), "4.json".to_owned()]);
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
        let docs = load_run_combat_docs(7);
        assert_eq!(docs.len(), 2);
        assert!(docs[0].doc.contains(r#""combat_id":1"#));
        assert!(docs[1].doc.contains(r#""combat_id":2"#));
        assert_eq!(load_run_combat_docs(9).len(), 1);
        assert!(
            load_run_combat_docs(5).is_empty(),
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

        let combats = parse_combat_docs(&load_combat_docs_from(&data.join("runs")));
        assert_eq!(combats.len(), 1);
        assert_eq!(combats[0].combat_id, 7);
        assert!(combats[0].run.is_none());
    }
}
