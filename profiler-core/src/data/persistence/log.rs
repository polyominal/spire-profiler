//! The process-lifetime `profiler.log` handle: one held file instead of an
//! open+stat+write+close per game event.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::data::state::STATE;
use crate::fail;

/// Opening the file per line costs ~4 syscalls per event; the handle is
/// opened lazily and reused. A broken directory logs only once.
struct LogSink {
    path: PathBuf,
    file: Option<fs::File>,
    open_failure_logged: bool,
}

thread_local! {
    static LOG_SINK: std::cell::RefCell<Option<LogSink>> = const { std::cell::RefCell::new(None) };
}

/// Appends one line to `<data_dir>/profiler.log` through the held handle.
pub fn append_log(line: String) {
    LOG_SINK.with(|cell| {
        let mut sink = cell.borrow_mut();
        // Clone the path only on an actual switch, never per appended line.
        let switched = match sink.as_ref() {
            Some(held) => STATE.with(|s| held.path != s.borrow().log_path_full),
            None => true,
        };
        if switched {
            let log_path = STATE.with(|s| s.borrow().log_path_full.clone());
            *sink = Some(LogSink {
                path: log_path,
                file: None,
                open_failure_logged: false,
            });
        }
        let sink = sink.as_mut().expect("sink was just ensured");
        if let Some(file) = sink.file.as_mut() {
            if let Err(err) = file.write_all(line.as_bytes()) {
                fail(format!("cannot write log line: {err}"));
                sink.file = None;
            }
            return;
        }
        // No held handle yet: one-shot open+write, kept on success.
        match fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&sink.path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(line.as_bytes()) {
                    fail(format!("cannot write log line: {err}"));
                } else {
                    sink.file = Some(file);
                }
            }
            Err(err) => {
                if !sink.open_failure_logged {
                    sink.open_failure_logged = true;
                    fail(format!("cannot open log file: {err}"));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;

    #[test]
    fn append_log_appends_lines() {
        let dir = temp_dir("append-log");
        let data = dir.join("data");
        fs::create_dir_all(&data).unwrap();
        init_state(&data);
        append_log("first\n".to_owned());
        append_log("second\n".to_owned());
        let content = fs::read_to_string(data.join("profiler.log")).unwrap();
        assert_eq!(content, "first\nsecond\n");
    }

    #[test]
    fn append_log_reuses_the_held_handle_across_calls() {
        // A per-call open would create a fresh profiler.log and leave the
        // renamed file with only the first line; the held handle keeps
        // appending to the renamed inode.
        let dir = temp_dir("append-hold");
        let data = dir.join("data");
        fs::create_dir_all(&data).unwrap();
        init_state(&data);
        append_log("first\n".to_owned());
        fs::rename(data.join("profiler.log"), data.join("profiler.log.moved")).unwrap();
        append_log("second\n".to_owned());
        assert!(!data.join("profiler.log").exists());
        let content = fs::read_to_string(data.join("profiler.log.moved")).unwrap();
        assert_eq!(content, "first\nsecond\n");
    }

    #[test]
    fn append_log_switches_handle_when_the_path_changes() {
        // The sink keys on the log path, so a data-dir change must not
        // keep writing the old file.
        let dir = temp_dir("append-switch");
        let a = dir.join("a");
        let b = dir.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        init_state(&a);
        append_log("to-a\n".to_owned());
        init_state(&b);
        append_log("to-b\n".to_owned());
        assert_eq!(
            fs::read_to_string(a.join("profiler.log")).unwrap(),
            "to-a\n"
        );
        assert_eq!(
            fs::read_to_string(b.join("profiler.log")).unwrap(),
            "to-b\n"
        );
    }
}
