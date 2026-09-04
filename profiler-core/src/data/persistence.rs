//! Persistence: the JSON files under `<data_dir>` and the write protocol.
//! This module doc is the on-disk schema's single home; nothing else
//! restates it.
//!
//! ```text
//! <data_dir>/                            <game dir>/mod_data/spire-profiler/ by default
//! ├── profiler.log                       append-only gameplay/event trace
//! ├── runs.jsonl                         one run record per line, rewritten atomically
//! └── runs/<run_id>/<combat_id>.json     one write-once file per finished combat
//! ```
//!
//! Split topically: [`time`] (the epoch-seconds clock), [`log`] (the held
//! log handle), [`io`] (tmp+rename writes, whole-file reads), [`combats`]
//! (the combat store), [`combat_doc`] (the combat record's serializer),
//! [`runs`] (the run accumulator and its fold), [`writes`] (the run
//! finalizer).
//!
//! # Identifiers
//!
//! Both ids are u32s derived from the store itself, never a process
//! counter. The combat id is seeded at boot to the highest id among the
//! store's file names and incremented at each combat start, so the first
//! new combat takes max+1, the file name IS the id, and same-seed replays
//! never collide. The run id is
//! max+1 over `runs.jsonl` records and the store's run directory names (an
//! abandoned run leaves its directory but no record line); a continued run
//! rejoins its fragments by seed instead.
//!
//! # On-disk formats
//!
//! All JSON, with two rules everywhere: identity fields stay explicit, and
//! every other numeric is omitted when zero — absent reads as zero
//! (`#[serde(default)]` on every parse struct). Timestamps are i64 epoch
//! seconds serialized straight from the in-memory value. There is no
//! `damage_unblocked`: `blocked + unblocked == dealt` is sim-pinned, so
//! readers derive it.
//!
//! Combat record:
//!
//! ```text
//! combat_id, started_at, encounter_id,
//! result ("completed" | "defeat" | "interrupted"),
//! turns, damage_received,
//! run: {seq, character, ascension, game_mode, seed}   // absent for out-of-run combats
//! cards: [{id, kind, plays, damage_dealt, damage_blocked,
//!          block_gained, block_effective, forge, dmg_direct, dmg_attributed,
//!          dmg_modifier, blk_modifier,
//!          mitigate_debuff, mitigate_buff, mitigate_str, self_damage, player}]
//! ```
//!
//! It carries only what the read side consumes — the run-history view and
//! the resume rebuild: no roster or profile (`runs.jsonl` is their home)
//! and no headline counters (`plays`, `potions_used`, ...). The `run`
//! header is complete so the combats-only fallback for unclosed runs can
//! synthesize a view, and its seed rejoins a resumed run's fragments.
//!
//! Run record:
//!
//! ```text
//! run_id, profile, character, ascension, game_mode,
//! outcome ("victory" | "defeat" | "abandoned"), seed,
//! started_at, ended_at,
//! players: [{slot, character}]   // absent when empty
//! ```
//!
//! `outcome` maps the ABI's `run_ended` code (0/1/2); there is no
//! `abandoned_at` because the abandon force-kill ends the run — `ended_at`
//! IS the abandon moment. `started_at` is the game's own `StartTime`, so
//! run-history matching is exact equality on seed + time. The record is
//! identity + header only: the run-history panel recomputes roll-ups from
//! the combat store. The roster carries slot + character; the net id stays
//! in-memory (nothing reads it back).
//!
//! Schema evolution is additive-only, enforced by the parse structs: new
//! fields arrive optional-on-read, existing fields never change type or
//! meaning, and every parser ignores unknown fields — a reader of any age
//! parses any record. There is no migration machinery and no schema
//! version.
//!
//! # Write protocol
//!
//! [`write_file`] writes a sibling `.tmp` file, then renames it into place
//! (atomic on POSIX), so a crash mid-write leaves the previous complete
//! file or none — never a torn one. Combat records are write-once;
//! `runs.jsonl` is rewritten whole (one line per run). Writes never fsync:
//! the guarantee is crash-consistency (complete-or-absent), not
//! crash-durability. [`MAX_JSON_SIZE`] (64 MiB) caps one document in both
//! directions — the writer refuses a too-large record and [`read_file`]
//! refuses one.
//!
//! [`read_file`] treats a missing file as "no data yet" (an empty file is
//! a state, not an error) and validates UTF-8. Every store scan skips
//! names that are not `<digits>.json`, and every parse failure is
//! fail-logged and skipped — one bad file never takes the rest of the
//! store down. The store grows without bound by design.

mod combat_doc;
mod combats;
mod io;
mod log;
mod runs;
mod time;
mod writes;

pub use combat_doc::build_combat_json;
pub(crate) use combat_doc::card_stat_from_rec;
pub use combats::write_combat_file;
pub(crate) use combats::{load_combat_docs_from, max_combat_id, parse_combat_docs};
pub use io::{ensure_data_dir, read_file, write_file};
pub(crate) use log::{append_log, bind_log_path, event_log, reset_log_sink};
pub(crate) use runs::{CardStatKey, upsert_card_stat};
pub use runs::{merge_into_run, rebuild_run_accumulator};
pub use time::now_seconds;
pub use writes::write_run_record;

/// Hard cap on one JSON document (read and write).
const MAX_JSON_SIZE: usize = 64 * 1024 * 1024;

const RUNS_DIR_NAME: &str = "runs";

#[cfg(test)]
pub(crate) mod test_support {
    // The shared fixtures for the persistence submodule suites: the STATE
    // re-pointing, the synthetic combat, and the store-file helper every
    // suite reuses, so each suite's `tests` module stays small.
    use super::bind_log_path;
    use crate::data::state::{CardStat, Combat, RunPlayer, RunSnapshot, STATE};
    use crate::source_kind::SourceKind;
    pub(crate) use crate::test_util::unique_dir;

    /// Points the test process's STATE at `data` (creating it), with the
    /// file paths derived the way init derives them.
    pub(crate) fn init_state(data: &std::path::Path) {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.data_dir = data.to_path_buf();
            st.runs_dir_full = data.join("runs");
            st.runs_path_full = data.join("runs.jsonl");
        });
        bind_log_path(&data.join("profiler.log"));
    }

    /// The single-player roster every fixture combat carries: one slot-0
    /// entry, the shape the shim stamps for solo runs.
    pub(crate) fn synthetic_roster() -> Vec<RunPlayer> {
        vec![RunPlayer {
            slot: 0,
            net_id: "1".to_owned(),
            character: "SHROUD".to_owned(),
        }]
    }

    pub(crate) fn synthetic_run(seq: u32) -> RunSnapshot {
        RunSnapshot {
            seq,
            character: "SHROUD".to_owned(),
            ascension: 5,
            game_mode: "standard".to_owned(),
            seed: String::new(),
        }
    }

    /// Every serialized field; OMNI_CARD is fictional because no real card
    /// populates every field.
    pub(crate) fn synthetic_combat() -> Combat {
        Combat {
            seq: 7,
            encounter_id: "BYGONE_EFFIGY".to_owned(),
            encounter_type: "Elite".to_owned(),
            started_at: 1_786_624_000,
            result: "completed".to_owned(),
            turns: 5,
            damage_received: 33,
            run: Some(synthetic_run(42)),
            players: synthetic_roster(),
            cards: vec![
                CardStat {
                    id: "OMNI_CARD".to_owned(),
                    kind: SourceKind::Card,
                    player: 0,
                    plays: 4,
                    damage_dealt: 21,
                    damage_blocked: 2,
                    block_gained: 5,
                    block_effective: 4,
                    forge: 1,
                    dmg_direct: 11,
                    dmg_attributed: 5,
                    dmg_modifier: 3,
                    blk_modifier: 1,
                    mitigate_debuff: 4,
                    mitigate_buff: 2,
                    mitigate_str: 1,
                    self_damage: 3,
                },
                CardStat {
                    id: "ANCHOR".to_owned(),
                    kind: SourceKind::Relic,
                    block_gained: 10,
                    ..CardStat::default()
                },
            ],
            ..Combat::default()
        }
    }

    /// Writes one store file the way the store lays files out.
    pub(crate) fn write_store_file(data: &std::path::Path, run_id: u32, combat_id: u32, doc: &str) {
        let dir = data.join("runs").join(run_id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{combat_id}.json")), doc).unwrap();
    }
}
