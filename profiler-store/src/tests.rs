use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::Store;

thread_local! {
    static BUSY_CALLS: Cell<usize> = const { Cell::new(0) };
}

fn count_busy(_: i32) -> bool {
    BUSY_CALLS.with(|calls| calls.set(calls.get() + 1));
    false
}

fn database() -> (TempDir, PathBuf, Store) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the store crate has a workspace parent")
        .join("tmp");
    std::fs::create_dir_all(&scratch).expect("workspace scratch is writable");
    let directory = tempfile::tempdir_in(scratch).expect("fixture owns a scratch directory");
    let path = directory.path().join("statistics-v3/statistics.sqlite3");
    let store = Store::open(&path).expect("fixture database opens");
    (directory, path, store)
}

fn run(id: &str, seed: &str, started_at: i64) -> Value {
    json!({
        "schema_version": 2, "game_version": "game", "mod_version": "mod",
        "run_id": id, "preserved_run_ids": [], "profile": 0, "seed": seed,
        "started_at": started_at, "ended_at": 0, "character": "IRONCLAD",
        "ascension": 0, "game_mode": "normal", "outcome": "active",
        "players": [{"slot": 0, "character": "IRONCLAD"}]
    })
}

fn record(owner: &Value, combat_id: u32, ordinal: u32, damage: i64, policy: i32) -> Value {
    json!({
        "schema_version": 2, "game_version": "game", "mod_version": "mod",
        "run_id": owner["run_id"], "ordinal": ordinal, "run": owner,
        "combat": {
            "policy_version": policy, "combat_id": combat_id,
            "encounter_id": "TEST", "encounter_type": "normal", "started_at": 100,
            "result": "completed", "turns": 1, "plays": 1, "potions_used": 0,
            "damage_received": 0, "block_total": 0,
            "cards": [{
                "id": "STRIKE", "kind": 0, "player": 0, "plays": 1,
                "damage_dealt": damage, "damage_blocked": 0,
                "block_gained": 0, "block_effective": 0, "forge": 0,
                "dmg_direct": if policy == 0 { 0 } else { damage },
                "dmg_attributed": 0, "dmg_modifier": 0,
                "blk_modifier": 0, "mitigate_debuff": 0, "mitigate_buff": 0,
                "mitigate_str": 0, "self_damage": 0
            }],
            "coverage": {
                "quality": if policy == 0 { "unknown" } else { "complete" },
                "failures": 0, "reasons": []
            }
        }
    })
}

fn execute(store: &mut Store, request: Value) -> Value {
    store
        .execute(&request.to_string())
        .expect("fixture request succeeds")
}

fn open_run(store: &mut Store, seed: &str, started_at: i64) -> Value {
    execute(
        store,
        json!({"op": "open_run", "run": run("", seed, started_at), "continued": false}),
    )
}

fn save_combat(store: &mut Store, record: Value) -> Value {
    execute(store, json!({"op": "save_combat", "record": record}))
}

fn load_run(store: &mut Store, id: &str) -> Value {
    execute(store, json!({"op": "load_run", "run_id": id}))
}

fn select(store: &mut Store, seed: &str, started_at: i64) -> Value {
    execute(
        store,
        json!({"op": "select", "profile": 0, "seed": seed, "started_at": started_at}),
    )
}

fn combat_ids(view: &Value) -> Vec<u32> {
    view["combats"]
        .as_array()
        .expect("run has combats")
        .iter()
        .map(|record| {
            record["combat"]["combat_id"]
                .as_u64()
                .expect("numeric combat ID") as u32
        })
        .collect()
}

#[test]
fn a_failed_payload_commit_leaves_a_durable_missing_intent_after_reopen() {
    let (_directory, path, mut store) = database();
    let owner = open_run(&mut store, "INTERRUPTED", 100);
    store
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_payload BEFORE UPDATE OF record_json ON combats
         BEGIN SELECT RAISE(FAIL, 'injected payload failure'); END;",
        )
        .expect("fixture installs a connection-local write failure");
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":record(&owner, 1, 1, 9, 3)}).to_string())
            .is_err()
    );
    assert_eq!(execute(&mut store, json!({"op":"max_combat_id"})), json!(1));
    let pending = select(&mut store, "INTERRUPTED", 100);
    assert_eq!(pending["last_ordinal"], 1);
    assert_eq!(pending["reasons"], json!(["statistics-record-missing"]));
    assert!(combat_ids(&pending).is_empty());

    drop(store);
    let mut reopened = Store::open(&path).expect("committed intent survives reopen");
    let missing = select(&mut reopened, "INTERRUPTED", 100);
    assert_eq!(missing["reasons"], json!(["statistics-record-missing"]));
    assert_eq!(
        save_combat(&mut reopened, record(&owner, 2, 2, 4, 3)),
        json!(true)
    );
    let later = select(&mut reopened, "INTERRUPTED", 100);
    assert_eq!(combat_ids(&later), [2]);
    assert_eq!(later["reasons"], json!(["statistics-record-missing"]));
    let healthy = open_run(&mut reopened, "HEALTHY", 101);
    assert_eq!(
        save_combat(&mut reopened, record(&healthy, 3, 3, 7, 3)),
        json!(true)
    );
    assert_eq!(select(&mut reopened, "HEALTHY", 101)["reasons"], json!([]));
}

#[test]
fn locked_intent_backlog_attempts_one_write_and_recovers_on_explicit_save() {
    let (_directory, path, mut store) = database();
    let owner = open_run(&mut store, "LOCKED", 200);
    let other = open_run(&mut store, "OTHER", 201);
    let locker = Connection::open(path).expect("second connection opens");
    locker
        .execute_batch("BEGIN")
        .expect("reader transaction begins");
    let _: i64 = locker
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .expect("fixture holds a shared lock through commit");
    for id in 1..=8 {
        assert!(
            store
                .execute(&json!({"op":"save_combat","record":record(&owner,id,id,1,3)}).to_string())
                .is_err()
        );
    }
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":record(&other,1,1,1,3)}).to_string())
            .is_err()
    );
    assert!(select(&mut store, "OTHER", 201).is_null());
    store
        .connection
        .busy_handler(Some(count_busy))
        .expect("fixture counts busy writes");
    BUSY_CALLS.with(|calls| calls.set(0));
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":record(&owner,9,9,1,3)}).to_string())
            .is_err()
    );
    assert_eq!(
        BUSY_CALLS.with(Cell::get),
        1,
        "the backlog attempts one transaction per save"
    );
    let pending = select(&mut store, "LOCKED", 200);
    assert_eq!(pending["last_ordinal"], 9);
    assert_eq!(
        pending["reasons"],
        json!(["statistics-intent-write-failed"])
    );
    locker
        .execute_batch("ROLLBACK")
        .expect("fixture releases its reader lock");
    store
        .connection
        .busy_timeout(Duration::from_millis(50))
        .expect("fixture restores the busy limit");

    let mut finalized = owner.clone();
    finalized["outcome"] = json!("victory");
    finalized["ended_at"] = json!(300);
    assert_eq!(
        execute(&mut store, json!({"op":"save_run","run":finalized})),
        json!(true)
    );
    let view = select(&mut store, "LOCKED", 200);
    assert_eq!(view["last_ordinal"], 9);
    assert_eq!(view["reasons"], json!(["statistics-record-missing"]));
}

#[test]
fn live_records_sort_by_combat_id_and_duplicate_payloads_remain_immutable() {
    let (_directory, path, mut store) = database();
    let first = open_run(&mut store, "FIRST", 300);
    let second = open_run(&mut store, "SECOND", 301);
    assert!(select(&mut store, "FIRST", 300).is_null());
    let mut wrong_identity = record(&first, 4, 4, 1, 3);
    wrong_identity["run"]["seed"] = json!("SECOND");
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":wrong_identity}).to_string())
            .is_err()
    );
    assert_eq!(execute(&mut store, json!({"op":"max_combat_id"})), json!(0));
    assert_eq!(
        save_combat(&mut store, record(&first, 3, 3, 3, 3)),
        json!(true)
    );
    assert_eq!(
        save_combat(&mut store, record(&first, 1, 1, 9, 3)),
        json!(true)
    );
    assert_eq!(
        save_combat(&mut store, record(&second, 2, 2, 5, 3)),
        json!(true)
    );
    assert_eq!(
        combat_ids(&load_run(
            &mut store,
            first["run_id"].as_str().expect("run ID")
        )),
        [1, 3]
    );
    assert_eq!(execute(&mut store, json!({"op":"max_combat_id"})), json!(3));
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":record(&first,1,1,99,3)}).to_string())
            .is_err()
    );
    assert!(
        store
            .execute(&json!({"op":"save_combat","record":record(&second,1,1,99,3)}).to_string())
            .is_err()
    );

    drop(store);
    let mut reopened = Store::open(&path).expect("database reopens");
    let retained = select(&mut reopened, "FIRST", 300);
    assert_eq!(combat_ids(&retained), [1, 3]);
    assert_eq!(
        retained["combats"][0]["combat"]["cards"][0]["damage_dealt"],
        9
    );
    assert_eq!(select(&mut reopened, "SECOND", 301)["reasons"], json!([]));
}

#[test]
fn corrupt_payloads_degrade_only_their_owner_and_keep_valid_records() {
    let (_directory, _path, mut store) = database();
    let first = open_run(&mut store, "CORRUPT", 400);
    let second = open_run(&mut store, "HEALTHY", 401);
    for id in [1, 3, 4] {
        save_combat(&mut store, record(&first, id, id, i64::from(id), 3));
    }
    save_combat(&mut store, record(&second, 2, 2, 7, 3));
    let mut wrong_version = record(&first, 1, 1, 1, 3);
    wrong_version["schema_version"] = json!(99);
    store
        .connection
        .execute(
            "UPDATE combats SET record_json=?1 WHERE native_id=1",
            [wrong_version.to_string()],
        )
        .expect("fixture corrupts one stored version");
    let mut wrong_owner = record(&first, 3, 3, 3, 3);
    wrong_owner["run_id"] = second["run_id"].clone();
    store
        .connection
        .execute(
            "UPDATE combats SET record_json=?1 WHERE native_id=3",
            [wrong_owner.to_string()],
        )
        .expect("fixture corrupts one stored identity");

    let damaged = select(&mut store, "CORRUPT", 400);
    assert_eq!(combat_ids(&damaged), [4]);
    assert_eq!(damaged["reasons"], json!(["statistics-read-failed"]));
    let healthy = select(&mut store, "HEALTHY", 401);
    assert_eq!(combat_ids(&healthy), [2]);
    assert_eq!(healthy["reasons"], json!([]));
}

#[test]
fn unknown_database_schema_is_rejected_without_overwriting_its_contents() {
    let (_directory, path, store) = database();
    store
        .connection
        .execute_batch("CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES ('keep');")
        .expect("fixture adds a sentinel");
    store
        .connection
        .pragma_update(None, "user_version", 99)
        .expect("fixture assigns an unknown schema");
    drop(store);
    assert!(Store::open(&path).is_err());
    let connection = Connection::open(path).expect("unknown database remains readable directly");
    let value: String = connection
        .query_row("SELECT value FROM sentinel", [], |row| row.get(0))
        .expect("sentinel survives rejection");
    let version: i32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("unknown version survives rejection");
    assert_eq!((value.as_str(), version), ("keep", 99));
}

const ARCHIVE_GUID: &str = "abcdefabcdefabcdefabcdefabcdefab";

fn archive_request() -> Value {
    let mut first = run("40", "ARCHIVE", 500);
    first["outcome"] = json!("victory");
    first["ended_at"] = json!(600);
    let second = run(ARCHIVE_GUID, "PRESERVED", 501);
    json!({
        "op": "import", "max_run_id": 40, "max_combat_id": 50,
        "runs": [
            {"source_key":"archive-a", "run":first, "finalized":true, "reasons":[],
             "combats":[record(&first,50,50,9,3),record(&first,4,4,4,3),record(&first,4,4,5,3)]},
            {"source_key":"archive-b", "run":second, "finalized":false, "reasons":[],
             "combats":[record(&second,6,1,6,3),record(&second,7,7,8,0)]}
        ],
        "ambiguous":[{"profile":0,"seed":"BLOCKED","started_at":502}],
        "complete":true, "reasons":[]
    })
}

#[test]
fn archive_import_rolls_back_all_runs_and_reserved_ids_on_failure() {
    let (_directory, _path, mut store) = database();
    store
        .connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_second_import BEFORE INSERT ON runs
         WHEN NEW.source_key='archive-b' BEGIN SELECT RAISE(FAIL, 'injected import failure'); END;",
        )
        .expect("fixture installs a transaction failure");
    assert!(store.execute(&archive_request().to_string()).is_err());
    assert_eq!(
        execute(&mut store, json!({"op":"import_status"})),
        json!(false)
    );
    assert_eq!(execute(&mut store, json!({"op":"max_combat_id"})), json!(0));
    assert!(select(&mut store, "ARCHIVE", 500).is_null());
}

#[test]
fn archive_import_preserves_order_duplicates_and_ids_after_idempotent_retry() {
    let (_directory, _path, mut store) = database();
    let request = archive_request();
    assert_eq!(execute(&mut store, request.clone()), json!(true));
    assert_eq!(execute(&mut store, request), json!(true));
    assert_eq!(
        execute(&mut store, json!({"op":"import_status"})),
        json!(true)
    );
    assert_eq!(
        execute(&mut store, json!({"op":"max_combat_id"})),
        json!(50)
    );
    assert_eq!(combat_ids(&load_run(&mut store, "40")), [50, 4, 4]);
    let imported = select(&mut store, "PRESERVED", 501);
    assert_eq!(combat_ids(&imported), [6, 7]);
    assert_eq!(imported["run"]["run_id"], "41");
    assert_eq!(imported["run"]["preserved_run_ids"], json!([ARCHIVE_GUID]));
    assert_eq!(
        imported["combats"][1]["combat"]["cards"][0]["damage_dealt"],
        8
    );
    assert_eq!(
        imported["combats"][1]["combat"]["cards"][0]["dmg_direct"],
        0
    );
    assert!(select(&mut store, "BLOCKED", 502).is_null());
    assert!(
        execute(
            &mut store,
            json!({"op":"open_run","run":run("","BLOCKED",502),"continued":true})
        )
        .is_null()
    );

    let owner = load_run(&mut store, "40")["run"].clone();
    assert_eq!(
        save_combat(&mut store, record(&owner, 51, 51, 3, 3)),
        json!(true)
    );
    assert_eq!(combat_ids(&load_run(&mut store, "40")), [50, 4, 4, 51]);
    let mut later_header = owner;
    later_header["outcome"] = json!("defeat");
    assert_eq!(
        execute(&mut store, json!({"op":"save_run","run":later_header})),
        json!(true)
    );
    assert_eq!(
        select(&mut store, "ARCHIVE", 500)["run"]["outcome"],
        "victory"
    );
    assert_eq!(open_run(&mut store, "NEW", 503)["run_id"], "42");
}
