use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use profiler_store::Store;
use serde::Deserialize;
use serde_json::json;

const UNAVAILABLE: &str = r#"{"ok":false,"error":"native storage unavailable"}"#;
const MAX_TRANSPORT_BYTES: usize = 64 * 1024 * 1024;
// The game needs one store; extra handles support independent tools and fixtures.
const MAX_STORES: usize = 16;
static NEXT_STORE: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static STORES: RefCell<Vec<(u64, Entry)>> = const { RefCell::new(Vec::new()) };
}

struct Entry {
    database: Option<Store>,
    response: Box<str>,
    poisoned: bool,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum OpenRequest {
    Open { path: String },
}

impl Entry {
    fn execute(&mut self, request: &str) {
        self.response = UNAVAILABLE.into();
        if self.poisoned {
            return;
        }
        struct PanicGuard<'a>(&'a mut Entry);
        impl Drop for PanicGuard<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.poisoned = true;
                    self.0.database = None;
                }
            }
        }
        let guard = PanicGuard(self);
        let entry = &mut *guard.0;
        #[cfg(test)]
        if request == r#"{"op":"panic_for_test"}"# {
            panic!("injected storage command panic");
        }
        let result = if let Some(database) = &mut entry.database {
            database.execute(request).map_err(|error| error.to_string())
        } else if request.len() > MAX_TRANSPORT_BYTES {
            Err("storage request exceeds size limit".to_owned())
        } else {
            serde_json::from_str::<OpenRequest>(request)
                .map_err(|error| error.to_string())
                .and_then(|OpenRequest::Open { path }| {
                    if path.is_empty() {
                        return Err("database path is empty".to_owned());
                    }
                    let database =
                        Store::open(Path::new(&path)).map_err(|error| error.to_string())?;
                    entry.database = Some(database);
                    Ok(json!(true))
                })
        };
        let response = match result {
            Ok(value) => json!({ "ok": true, "value": value }),
            Err(error) => json!({ "ok": false, "error": error }),
        }
        .to_string();
        entry.response = if response.len() < MAX_TRANSPORT_BYTES {
            response.into_boxed_str()
        } else {
            r#"{"ok":false,"error":"storage response exceeds size limit"}"#.into()
        };
    }
}

pub(super) fn create() -> u64 {
    STORES.with(|cell| {
        let mut entries = cell.borrow_mut();
        if entries.len() == MAX_STORES {
            return 0;
        }
        let Ok(id) =
            NEXT_STORE.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        else {
            return 0;
        };
        entries.push((
            id,
            Entry {
                database: None,
                response: UNAVAILABLE.into(),
                poisoned: false,
            },
        ));
        id
    })
}

pub(super) fn destroy(id: u64) {
    STORES.with(|cell| cell.borrow_mut().retain(|(key, _)| *key != id));
}

pub(super) fn execute(id: u64, request: &str) -> i32 {
    STORES.with(|cell| {
        let Ok(mut entries) = cell.try_borrow_mut() else {
            return 0;
        };
        let Some((_, entry)) = entries.iter_mut().find(|(key, _)| *key == id) else {
            return 0;
        };
        entry.execute(request);
        1
    })
}

pub(super) fn response(id: u64, copy: impl FnOnce(&str) -> i32) -> i32 {
    STORES.with(|cell| {
        let Ok(entries) = cell.try_borrow() else {
            return 0;
        };
        let Some((_, entry)) = entries.iter().find(|(key, _)| *key == id) else {
            return 0;
        };
        copy(&entry.response)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn response_copy_respects_caller_capacity_and_terminates_json() {
        let handle = create();
        execute(handle, "malformed");
        // SAFETY: null buffers request sizing only, regardless of capacity.
        let required =
            unsafe { crate::abi::spire_profiler_store_response(handle, std::ptr::null_mut(), 0) };
        assert!(required > 1);
        let mut buffer = vec![0xa5; required as usize + 1];
        for capacity in [-1, 0, required - 1] {
            // SAFETY: the allocation covers every nonnegative advertised capacity.
            let size = unsafe {
                crate::abi::spire_profiler_store_response(handle, buffer.as_mut_ptr(), capacity)
            };
            assert_eq!(size, required);
            assert!(buffer.iter().all(|&byte| byte == 0xa5));
        }
        // SAFETY: the owned allocation covers the requested capacity.
        let copied = unsafe {
            crate::abi::spire_profiler_store_response(handle, buffer.as_mut_ptr(), required)
        };
        assert_eq!(copied, required);
        assert_eq!(buffer[required as usize - 1], 0);
        assert_eq!(
            buffer[required as usize], 0xa5,
            "copy stays within capacity"
        );
        let reply: Value = serde_json::from_slice(&buffer[..required as usize - 1])
            .expect("the copied response is complete JSON");
        assert_eq!(reply["ok"], false);
        destroy(handle);
    }

    #[test]
    fn database_open_accepts_json_escaped_paths_and_outlives_an_engine() {
        let handle = create();
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tmp")
            .join(format!("store-abi-{}-{handle}", std::process::id()));
        assert!(
            !directory.exists(),
            "the fixture owns a fresh scratch directory"
        );
        let path = directory.join("測試.sqlite3");
        let request = json!({"op": "open", "path": path})
            .to_string()
            .replace('測', "\\u6e2c");
        let engine = crate::abi::spire_profiler_engine_create();
        assert_eq!(execute(handle, &request), 1);
        response(handle, |json| {
            let reply: Value = serde_json::from_str(json).expect("the store returns JSON");
            assert_eq!(reply["ok"], true, "{json}");
            1
        });
        crate::abi::spire_profiler_engine_destroy(engine);
        assert_eq!(execute(handle, r#"{"op":"import_status"}"#), 1);
        response(handle, |json| {
            let reply: Value = serde_json::from_str(json).expect("the store returns JSON");
            assert_eq!(
                reply["value"], false,
                "an engine does not own its sibling store"
            );
            1
        });
        destroy(handle);
        assert!(path.is_file());
        std::fs::remove_dir_all(directory).expect("the closed fixture owns its database");
    }

    #[test]
    fn requests_are_thread_owned_and_response_reads_do_not_execute_them() {
        let first = create();
        let second = create();
        assert_ne!(first, 0);
        assert_ne!(second, 0);
        assert_eq!(execute(first, "malformed"), 1);
        let mut original = String::new();
        response(first, |json| {
            original = json.to_owned();
            1
        });
        assert!(original.contains("error"));
        assert_ne!(original, UNAVAILABLE);
        for _ in 0..3 {
            assert_eq!(
                response(first, |json| {
                    assert_eq!(json, original);
                    1
                }),
                1
            );
        }
        response(second, |json| {
            assert_eq!(json, UNAVAILABLE);
            1
        });
        std::thread::spawn(move || {
            assert_eq!(execute(first, "malformed"), 0);
            assert_eq!(
                response(first, |_| panic!("another thread cannot find this store")),
                0
            );
            destroy(first);
        })
        .join()
        .expect("wrong-thread calls do not unwind");
        assert_eq!(response(first, |_| 1), 1);
        destroy(first);
        assert_eq!(
            response(first, |_| panic!(
                "a closed handle cannot expose a stale response"
            )),
            0
        );
        assert_eq!(execute(first, "malformed"), 0);
        assert_eq!(response(second, |_| 1), 1);
        destroy(second);
    }

    #[test]
    fn oversized_abi_command_aborts_an_open_import() {
        let handle = create();
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tmp")
            .join(format!("store-oversized-{}-{handle}", std::process::id()));
        assert!(
            !directory.exists(),
            "fixture owns a fresh scratch directory"
        );
        let path = directory.join("statistics.sqlite3");
        let reply = |handle| {
            let mut value = Value::Null;
            response(handle, |json| {
                value = serde_json::from_str(json).expect("ABI returns a JSON envelope");
                1
            });
            value
        };
        assert_eq!(
            execute(handle, &json!({"op":"open","path":path}).to_string()),
            1
        );
        assert_eq!(reply(handle)["ok"], true);
        assert_eq!(
            execute(
                handle,
                r#"{"op":"import_begin","max_run_id":40,"max_combat_id":50}"#
            ),
            1
        );
        assert_eq!(reply(handle)["ok"], true);
        assert_eq!(execute(handle, &"x".repeat(MAX_TRANSPORT_BYTES + 1)), 1);
        assert_eq!(reply(handle)["ok"], false);
        assert_eq!(execute(handle, r#"{"op":"import_end","complete":true}"#), 1);
        assert_eq!(reply(handle)["ok"], false);
        assert_eq!(execute(handle, r#"{"op":"import_status"}"#), 1);
        assert_eq!(reply(handle)["value"], false);
        assert_eq!(execute(handle, r#"{"op":"max_combat_id"}"#), 1);
        assert_eq!(reply(handle)["value"], 0);
        destroy(handle);
        std::fs::remove_dir_all(directory).expect("closed fixture owns its database");
    }

    #[test]
    fn panic_drops_an_active_read_transaction_before_another_store_writes() {
        let reader = create();
        let writer = create();
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tmp")
            .join(format!("store-panic-{}-{reader}", std::process::id()));
        assert!(
            !directory.exists(),
            "fixture owns a fresh scratch directory"
        );
        let path = directory.join("statistics.sqlite3");
        let open = json!({"op":"open","path":path}).to_string();
        assert_eq!(execute(reader, &open), 1);
        assert_eq!(execute(writer, &open), 1);
        let run = |seed| {
            json!({"schema_version":2,"game_version":"g","mod_version":"m",
            "run_id":"","preserved_run_ids":[],"profile":0,"seed":seed,"started_at":100,
            "ended_at":0,"character":"IRONCLAD","ascension":0,"game_mode":"normal",
            "outcome":"active","players":[]})
        };
        assert_eq!(
            execute(
                writer,
                &json!({"op":"open_run","run":run("FIRST"),"continued":false}).to_string()
            ),
            1
        );
        assert_eq!(
            execute(reader, r#"{"op":"read_begin_load","run_id":"1"}"#),
            1
        );
        response(reader, |json| {
            let reply: Value = serde_json::from_str(json).expect("read header is JSON");
            assert_eq!(reply["ok"], true);
            1
        });
        let injected = std::ffi::CString::new(r#"{"op":"panic_for_test"}"#)
            .expect("fixture command contains no NUL");
        // SAFETY: CString owns a terminated request for the duration of the ABI call.
        let result = unsafe { crate::abi::spire_profiler_store_execute(reader, injected.as_ptr()) };
        assert_eq!(result, 0);
        assert_eq!(
            execute(
                writer,
                &json!({"op":"open_run","run":run("SECOND"),"continued":false}).to_string()
            ),
            1
        );
        response(writer, |json| {
            let reply: Value = serde_json::from_str(json).expect("writer response is JSON");
            assert_eq!(
                reply["ok"], true,
                "read lock must be released after panic: {json}"
            );
            1
        });
        destroy(reader);
        destroy(writer);
        std::fs::remove_dir_all(directory).expect("closed fixtures own their database");
    }
}
