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
//! # Identifiers
//!
//! Both ids are u32s derived from the store itself, never a process
//! counter. The combat id is seeded at boot to the highest id among the
//! store's file names and incremented at each combat start, so the first
//! new combat takes max+1, the file name IS the id, and same-seed replays
//! never collide. The run id is
//! max+1 over `runs.jsonl` records and the store's run directory names (an
//! abandoned run leaves its directory but no record line); a continued run
//! rejoins only a unique exact (profile, seed, original StartTime) identity.
//! Both allocate through `u32::MAX`, then fail-log and refuse fresh starts
//! without wrapping or reusing an ID; exact run continuation still works.
//! Missing storage is empty; failed reads or directory scans cannot seed IDs.
//! A failed boot scan disables combat starts for this State owner; fresh run
//! starts retry their ID scan each time.
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
//! Source kinds encode Card=0, Relic=1, Power=2, Potion=3, Osty=4, and
//! Unknown=5. Unknown rows use `UNATTRIBUTED` with an explicit creditor slot;
//! their reserved capacity preserves totals when ordinary rows cannot fit.
//!
//! Combat record:
//!
//! ```text
//! combat_id, started_at, encounter_id,
//! result ("completed" | "defeat" | "interrupted"),
//! turns, damage_received,
//! run: {seq, character, ascension, game_mode, seed, profile, started_at}
//!     // absent for out-of-run combats
//! cards: [{id, kind, plays, damage_dealt, damage_blocked,
//!          block_gained, block_effective, forge, dmg_direct, dmg_attributed,
//!          dmg_modifier, blk_modifier,
//!          mitigate_debuff, mitigate_buff, mitigate_str, self_damage, player}]
//! ```
//!
//! It carries only what the read side consumes — the run-history view and
//! the resume rebuild: no roster (`runs.jsonl` is its home)
//! and no headline counters (`plays`, `potions_used`, ...). The `run`
//! header is complete so the combats-only fallback for unclosed runs can
//! synthesize a view and rejoin a resumed run's fragments. Its `started_at`
//! is the original game StartTime, distinct from the combat's timestamp.
//! Profile is captured at run start, never taken from later metadata. An
//! empty seed, profile < 0 (unknown defaults to -1), or start time <= 0
//! (unknown defaults to 0) cannot match; run ID 0 is never a real run.
//! Unknown identities remain explicit and never use the session clock.
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
//! run-history matching uses the same full identity as combat headers. The record is
//! identity + header only: the run-history panel recomputes roll-ups from
//! the combat store. The roster carries slot + character; the net id stays
//! in-memory (nothing reads it back).
//!
//! The schema changes with the structs: a breaking change lands directly,
//! and incompatible old data is deleted by hand, never migrated. The parse
//! structs stay lenient (missing fields default, unknown fields ignored) so
//! a partial or stale record still reads; that leniency is boundary
//! robustness, not a compatibility contract.
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
//! Combat/run writers return true, report success, and invalidate history
//! only after rename succeeds. Finished combats merge into their active
//! run's live totals even on write failure; lifecycle events never retry.
//!
//! [`read_file`] keeps “missing” separate from “unreadable” (an empty file
//! is a state, not an error) and validates UTF-8, so the runs.jsonl rewrite
//! never mistakes corrupt history for an empty store. Every store scan
//! skips names that are not `<digits>.json`, and every parse failure is
//! fail-logged and skipped — one bad file never takes the rest of the
//! store down. The store grows without bound by design.
//!
//! # Allocation boundary
//!
//! Filesystem and codec work is an explicit allocating boundary. The
//! inventory is `ensure_data_dir`, `read_file`, `read_dir`, `write_file`,
//! `build_combat_json`, the record builders and parsers in `records`,
//! `max_combat_id`, `load_run_combat_docs`, `load_combat_docs_from`,
//! `parse_combat_docs`, `write_combat_file`, and `write_run_record`.
//! `bind_log_path` and `append_log` are also adapters: the line buffer is
//! fixed, but path ownership, opening the sink, and the operating system may
//! allocate. `reset_log_sink` is their owner reset.
//!
//! These names identify allocation boundaries, not exemptions for callers.
//! `write_combat_file` calls `merge_into_run` before any store or codec work;
//! that merge is computation and stays measured. `rebuild_run_accumulator`
//! combines store loading, parsing, and run merging in one mixed operation.
//! Its whole call remains measured; a pure merge sample is separate evidence,
//! not a claim that the resume path is physically split. Cache invalidation
//! after a successful write is a lifecycle reset and remains observable by the
//! computation accounting. A mixed event cannot be wrapped in one ignored
//! region merely because it eventually writes a file.
//!
//! Bounded identity fields in [`super::state::caps`] also bound emitted JSON.
//! The budgets below enumerate every emitted field, including quotes, keys,
//! separators, full-width numbers, and worst-case text escaping. Arrays use
//! their maximum row and player counts. Schema-envelope tests pin the resulting
//! lengths, excluding the JSONL newline; the 64 MiB input file limit remains
//! independent of these smaller output bounds.

use super::state::caps;

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
pub(crate) use io::{ReadFile, read_dir, read_file};
pub use io::{ensure_data_dir, write_file};
pub(crate) use log::{append_log, bind_log_path, event_log, reset_log_sink};
pub(crate) use runs::CardStatKey;
pub use runs::{merge_into_run, rebuild_run_accumulator};
pub use time::now_seconds;
pub use writes::write_run_record;

/// Hard cap on one JSON document (read and write).
const MAX_JSON_SIZE: usize = 64 * 1024 * 1024;

const fn json_unsigned_bytes(value: u64) -> usize {
    if value == 0 {
        1
    } else {
        value.ilog10() as usize + 1
    }
}

const fn json_text_bytes(input_bytes: usize) -> usize {
    // An ASCII control byte expands to six bytes, for example \u0001.
    "\"\"".len() + "\\u0001".len() * input_bytes
}

const fn json_object_bytes(fields: &[(&str, usize)]) -> usize {
    let mut bytes = "{}".len();
    let mut index = 0;
    while index < fields.len() {
        if index > 0 {
            bytes += ",".len();
        }
        bytes += "\"\":".len() + fields[index].0.len() + fields[index].1;
        index += 1;
    }
    bytes
}

const fn json_array_bytes(element_bytes: usize, count: usize) -> usize {
    "[]".len() + count * element_bytes + count.saturating_sub(1) * ",".len()
}

const JSON_U8_BYTES: usize = json_unsigned_bytes(u8::MAX as u64);
const JSON_U32_BYTES: usize = json_unsigned_bytes(u32::MAX as u64);
const JSON_I32_BYTES: usize = "-".len() + json_unsigned_bytes(i32::MIN.unsigned_abs() as u64);
const JSON_I64_BYTES: usize = "-".len() + json_unsigned_bytes(i64::MIN.unsigned_abs());

const MAX_CARD_JSON_BYTES: usize = json_object_bytes(&[
    ("id", json_text_bytes(caps::MODEL_ID_BYTES)),
    (
        "kind",
        json_unsigned_bytes(crate::source_kind::SourceKind::Unknown as u64),
    ),
    ("plays", JSON_U32_BYTES),
    ("damage_dealt", JSON_I64_BYTES),
    ("damage_blocked", JSON_I64_BYTES),
    ("block_gained", JSON_I64_BYTES),
    ("block_effective", JSON_I64_BYTES),
    ("forge", JSON_I64_BYTES),
    ("dmg_direct", JSON_I64_BYTES),
    ("dmg_attributed", JSON_I64_BYTES),
    ("dmg_modifier", JSON_I64_BYTES),
    ("blk_modifier", JSON_I64_BYTES),
    ("mitigate_debuff", JSON_I64_BYTES),
    ("mitigate_buff", JSON_I64_BYTES),
    ("mitigate_str", JSON_I64_BYTES),
    ("self_damage", JSON_I64_BYTES),
    ("player", JSON_U8_BYTES),
]);

const MAX_RUN_SNAPSHOT_JSON_BYTES: usize = json_object_bytes(&[
    ("seq", JSON_U32_BYTES),
    ("character", json_text_bytes(caps::RUN_CHARACTER_BYTES)),
    ("ascension", JSON_I32_BYTES),
    ("game_mode", json_text_bytes(caps::LABEL_BYTES)),
    ("seed", json_text_bytes(caps::SEED_BYTES)),
    ("profile", JSON_I32_BYTES),
    ("started_at", JSON_I64_BYTES),
]);

pub(crate) const MAX_COMBAT_JSON_BYTES: usize = json_object_bytes(&[
    ("combat_id", JSON_U32_BYTES),
    ("started_at", JSON_I64_BYTES),
    ("encounter_id", json_text_bytes(caps::MODEL_ID_BYTES)),
    ("result", "\"interrupted\"".len()),
    ("turns", JSON_U32_BYTES),
    ("damage_received", JSON_I64_BYTES),
    ("run", MAX_RUN_SNAPSHOT_JSON_BYTES),
    (
        "cards",
        json_array_bytes(MAX_CARD_JSON_BYTES, caps::COMBAT_CARDS),
    ),
]);

const MAX_PLAYER_JSON_BYTES: usize = json_object_bytes(&[
    ("slot", JSON_U8_BYTES),
    ("character", json_text_bytes(caps::MODEL_ID_BYTES)),
]);

pub(crate) const MAX_RUN_JSON_BYTES: usize = json_object_bytes(&[
    ("run_id", JSON_U32_BYTES),
    ("profile", JSON_I32_BYTES),
    ("character", json_text_bytes(caps::RUN_CHARACTER_BYTES)),
    ("ascension", JSON_I32_BYTES),
    ("game_mode", json_text_bytes(caps::LABEL_BYTES)),
    ("outcome", "\"abandoned\"".len()),
    ("seed", json_text_bytes(caps::SEED_BYTES)),
    ("started_at", JSON_I64_BYTES),
    ("ended_at", JSON_I64_BYTES),
    (
        "players",
        json_array_bytes(MAX_PLAYER_JSON_BYTES, caps::MAX_PLAYERS),
    ),
]);

const _: () = assert!(MAX_COMBAT_JSON_BYTES < MAX_JSON_SIZE);
const _: () = assert!(MAX_RUN_JSON_BYTES + 1 < MAX_JSON_SIZE);

#[cfg(test)]
pub(crate) mod test_support {
    // The shared fixtures for the persistence submodule suites: the STATE
    // re-pointing, the synthetic combat, and the store-file helper every
    // suite reuses, so each suite's `tests` module stays small.
    use super::bind_log_path;
    use crate::data::state::{
        CardStat, Combat, CombatPhase, CombatResult, RunPlayer, RunSnapshot, STATE,
    };
    use crate::source_kind::SourceKind;
    use crate::test_util::text;
    pub(crate) use crate::test_util::unique_dir;

    /// Configures the store and log paths without creating their directories.
    pub(crate) fn init_state(data: &std::path::Path) {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.store_paths = Some(crate::data::state::StorePaths::new(data));
        });
        bind_log_path(&data.join("profiler.log"));
    }

    /// The single-player roster every fixture combat carries: one slot-0
    /// entry, the shape the shim stamps for solo runs.
    pub(crate) fn synthetic_roster() -> Vec<RunPlayer> {
        vec![RunPlayer {
            slot: 0,
            net_id: text("1"),
            character: text("SHROUD"),
        }]
    }

    pub(crate) fn synthetic_run(seq: u32) -> RunSnapshot {
        RunSnapshot {
            seq,
            character: text("SHROUD"),
            ascension: 5,
            game_mode: text("standard"),
            seed: text("S"),
            profile: 2,
            started_at: 1000,
        }
    }

    /// Every serialized field; OMNI_CARD is fictional because no real card
    /// populates every field.
    pub(crate) fn synthetic_combat() -> Combat {
        Combat {
            seq: 7,
            encounter_id: text("BYGONE_EFFIGY"),
            encounter_type: text("Elite"),
            started_at: 1_786_624_000,
            phase: CombatPhase::Finished(CombatResult::Completed),
            turns: 5,
            damage_received: 33,
            run: Some(synthetic_run(42)),
            players: synthetic_roster(),
            cards: vec![
                CardStat {
                    id: text("OMNI_CARD"),
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
                    id: text("ANCHOR"),
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
