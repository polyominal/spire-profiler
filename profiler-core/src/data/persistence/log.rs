//! Transparent event-log rendering and the process-lifetime `profiler.log`
//! handle. Lines render immediately into fixed storage, then drain through
//! one held file; the sink never re-enters profiler state. Each line has a
//! 2048-byte buffer: source ids and data-dir paths fit comfortably while a
//! logging call's stack use stays bounded. Overlong lines end with `…`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fmt, fs};

use crate::fail;

const EVENT_LOG_LINE_CAP: usize = 2048;
const TRUNCATION_SUFFIX: &str = "…\n";

struct LineBuffer {
    bytes: [u8; EVENT_LOG_LINE_CAP],
    len: usize,
    truncated: bool,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; EVENT_LOG_LINE_CAP],
            len: 0,
            truncated: false,
        }
    }

    fn finish(&mut self) -> &[u8] {
        let suffix = if self.truncated {
            TRUNCATION_SUFFIX.as_bytes()
        } else {
            b"\n"
        };
        self.bytes[self.len..self.len + suffix.len()].copy_from_slice(suffix);
        self.len += suffix.len();
        &self.bytes[..self.len]
    }
}

impl fmt::Write for LineBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }

        let budget = EVENT_LOG_LINE_CAP - TRUNCATION_SUFFIX.len();
        let start = self.len;
        if s.len() <= budget - start {
            self.bytes[start..start + s.len()].copy_from_slice(s.as_bytes());
            self.len = start + s.len();
            return Ok(());
        }

        let mut take = budget - start;
        while take > 0 && !s.is_char_boundary(take) {
            take -= 1;
        }
        self.bytes[start..start + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len = start + take;
        self.truncated = true;
        Ok(())
    }
}

struct LogSink {
    path: Option<PathBuf>,
    file: Option<fs::File>,
    open_failure_logged: bool,
}

thread_local! {
    static LOG_SINK: std::cell::RefCell<LogSink> = const {
        std::cell::RefCell::new(LogSink {
            path: None,
            file: None,
            open_failure_logged: false,
        })
    };
}

pub(crate) fn bind_log_path(path: &Path) {
    LOG_SINK.with(|cell| {
        let mut sink = cell.borrow_mut();
        if sink.path.as_deref() == Some(path) {
            return;
        }
        sink.path = Some(path.to_path_buf());
        sink.file = None;
        sink.open_failure_logged = false;
    });
}

pub(crate) fn reset_log_sink() {
    LOG_SINK.with(|cell| {
        *cell.borrow_mut() = LogSink {
            path: None,
            file: None,
            open_failure_logged: false,
        }
    });
}

pub(crate) fn append_log(args: fmt::Arguments<'_>) {
    let mut line = LineBuffer::new();
    let _ = fmt::Write::write_fmt(&mut line, args);
    let line = line.finish();

    LOG_SINK.with(|cell| {
        let mut sink = cell.borrow_mut();
        let Some(path) = sink.path.as_ref() else {
            return;
        };
        if sink.file.is_some() {
            let file = sink.file.as_mut().expect("file existence was just checked");
            if let Err(err) = file.write_all(line) {
                fail!(
                    "cannot write log line: {} (os error {})",
                    err.kind(),
                    err.raw_os_error().unwrap_or(-1)
                );
                sink.file = None;
            }
            return;
        }

        let opened = fs::OpenOptions::new().append(true).create(true).open(path);
        match opened {
            Ok(mut file) => {
                if let Err(err) = file.write_all(line) {
                    fail!(
                        "cannot write log line: {} (os error {})",
                        err.kind(),
                        err.raw_os_error().unwrap_or(-1)
                    );
                } else {
                    sink.file = Some(file);
                }
            }
            Err(err) => {
                if !sink.open_failure_logged {
                    sink.open_failure_logged = true;
                    fail!(
                        "cannot open log file: {} (os error {})",
                        err.kind(),
                        err.raw_os_error().unwrap_or(-1)
                    );
                }
            }
        }
    });
}

macro_rules! event_log {
    ($($arg:tt)*) => {
        $crate::data::persistence::append_log(format_args!($($arg)*))
    };
}

pub(crate) use event_log;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::state::STATE;

    #[test]
    fn event_log_appends_exact_lines() {
        let dir = crate::test_util::temp_dir("event-log-lines");
        let path = dir.join("profiler.log");
        bind_log_path(&path);
        event_log!("first");
        event_log!("second {}", 42);
        assert_eq!(fs::read_to_string(path).unwrap(), "first\nsecond 42\n");
    }

    #[test]
    fn event_log_accepts_the_exact_untruncated_line() {
        let dir = crate::test_util::temp_dir("event-log-exact");
        let path = dir.join("profiler.log");
        bind_log_path(&path);
        let exact = "A".repeat(EVENT_LOG_LINE_CAP - TRUNCATION_SUFFIX.len());
        event_log!("{exact}");

        let content = fs::read_to_string(path).unwrap();
        assert_eq!(
            content.len(),
            EVENT_LOG_LINE_CAP - TRUNCATION_SUFFIX.len() + 1
        );
        assert!(content.ends_with('\n'));
        assert!(!content.contains(TRUNCATION_SUFFIX));
    }

    #[test]
    fn event_log_truncates_on_utf8_boundaries() {
        let dir = crate::test_util::temp_dir("event-log-truncate");
        let path = dir.join("profiler.log");
        bind_log_path(&path);
        let long = "é".repeat(EVENT_LOG_LINE_CAP);
        event_log!("{long}");

        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.len(), EVENT_LOG_LINE_CAP);
        assert!(content.ends_with(TRUNCATION_SUFFIX));
        assert!(content.is_char_boundary(content.len() - TRUNCATION_SUFFIX.len()));
    }

    #[test]
    fn event_log_ignores_an_unbound_sink() {
        reset_log_sink();
        event_log!("dropped");
        assert!(
            !std::env::current_dir()
                .unwrap()
                .join("profiler.log")
                .exists()
        );
    }

    #[test]
    fn event_log_reuses_the_held_handle_across_calls() {
        let dir = crate::test_util::temp_dir("event-log-hold");
        let path = dir.join("profiler.log");
        bind_log_path(&path);
        event_log!("first");
        let moved = dir.join("moved.log");
        fs::rename(&path, &moved).unwrap();
        event_log!("second");
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(moved).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn event_log_switches_handles_when_rebound() {
        let dir = crate::test_util::temp_dir("event-log-switch");
        let a = dir.join("a");
        let b = dir.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        bind_log_path(&a.join("profiler.log"));
        event_log!("to-a");
        bind_log_path(&b.join("profiler.log"));
        event_log!("to-b");
        assert_eq!(
            fs::read_to_string(a.join("profiler.log")).unwrap(),
            "to-a\n"
        );
        assert_eq!(
            fs::read_to_string(b.join("profiler.log")).unwrap(),
            "to-b\n"
        );
    }

    #[test]
    fn event_log_writes_while_state_is_borrowed() {
        let dir = crate::test_util::temp_dir("event-log-state");
        let path = dir.join("profiler.log");
        bind_log_path(&path);
        STATE.with(|cell| {
            let state = &mut *cell.borrow_mut();
            event_log!("while borrowed: {}", state.run_seq_accumulated);
        });
        assert_eq!(fs::read_to_string(path).unwrap(), "while borrowed: 0\n");
    }
}
