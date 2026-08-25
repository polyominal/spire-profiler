//! Atomic file IO and the data-directory layout: [`write_file`]'s
//! tmp+rename, [`read_file`]'s whole-file reads under the 64 MiB budget,
//! and [`ensure_data_dir`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{MAX_JSON_SIZE, RUNS_DIR_NAME};
use crate::data::state::STATE;
use crate::fail;

/// Creates `<data_dir>` and the runs store (false + a fail log on error).
pub fn ensure_data_dir() -> bool {
    let (data_dir, runs_dir) = STATE.with(|s| {
        let st = s.borrow();
        let data_dir = st.data_dir.clone();
        let runs_dir = data_dir.join(RUNS_DIR_NAME);
        (data_dir, runs_dir)
    });
    if let Err(err) = fs::create_dir_all(&data_dir) {
        fail!(
            "cannot create data directory '{}': {} (os error {})",
            data_dir.display(),
            err.kind(),
            err.raw_os_error().unwrap_or(-1)
        );
        return false;
    }
    if let Err(err) = fs::create_dir_all(&runs_dir) {
        fail!(
            "cannot create runs store directory '{}': {} (os error {})",
            runs_dir.display(),
            err.kind(),
            err.raw_os_error().unwrap_or(-1)
        );
        return false;
    }
    true
}

/// A sibling `.tmp` file, then a rename (atomic on POSIX).
pub fn write_file(path: &Path, bytes: &str) -> bool {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = PathBuf::from(tmp_name);
    let write_result =
        fs::File::create(&tmp_path).and_then(|mut file| file.write_all(bytes.as_bytes()));
    if let Err(err) = write_result {
        // Deliberately silent cleanup: the real error is already reported,
        // and a stale .tmp is inert (the next write truncates it).
        let _ = fs::remove_file(&tmp_path);
        fail!(
            "cannot write '{}': {} (os error {})",
            tmp_path.display(),
            err.kind(),
            err.raw_os_error().unwrap_or(-1)
        );
        return false;
    }
    match fs::rename(&tmp_path, path) {
        Ok(()) => true,
        Err(err) => {
            let _ = fs::remove_file(&tmp_path);
            fail!(
                "cannot move '{}' into place: {} (os error {})",
                path.display(),
                err.kind(),
                err.raw_os_error().unwrap_or(-1)
            );
            false
        }
    }
}

/// An empty file yields a zero-length string: "no data yet" is a state.
pub fn read_file(path: &Path) -> Option<String> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            fail!(
                "cannot open '{}': {} (os error {})",
                path.display(),
                err.kind(),
                err.raw_os_error().unwrap_or(-1)
            );
            return None;
        }
    };
    if meta.len() > MAX_JSON_SIZE as u64 {
        fail!(
            "'{}' is too large to read ({} bytes > {MAX_JSON_SIZE})",
            path.display(),
            meta.len()
        );
        return None;
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            fail!(
                "cannot open '{}': {} (os error {})",
                path.display(),
                err.kind(),
                err.raw_os_error().unwrap_or(-1)
            );
            return None;
        }
    };
    match String::from_utf8(bytes) {
        Ok(content) => Some(content),
        Err(err) => {
            fail!("'{}' is not valid UTF-8: {err}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;

    #[test]
    fn ensure_data_dir_creates_both_dirs() {
        let dir = temp_dir("ensure");
        init_state(&dir.join("data"));
        assert!(ensure_data_dir());
        assert!(dir.join("data").is_dir());
        assert!(dir.join("data/runs").is_dir());
    }

    #[test]
    fn ensure_data_dir_reports_failure() {
        // A blocker file where a directory component belongs makes
        // create_dir_all fail without crashing.
        let dir = temp_dir("ensure-fail");
        fs::write(dir.join("blocker"), "x").unwrap();
        init_state(&dir.join("blocker/data"));
        assert!(!ensure_data_dir());
    }

    #[test]
    fn write_file_round_trips_and_leaves_no_temp() {
        let dir = temp_dir("write");
        let path = dir.join("f.json");
        assert!(write_file(&path, "hello"));
        assert_eq!(read_file(&path).unwrap(), "hello");
        assert!(!dir.join("f.json.tmp").exists());
        assert!(write_file(&path, "bye"));
        assert_eq!(read_file(&path).unwrap(), "bye");
        assert!(!dir.join("f.json.tmp").exists());
    }

    #[test]
    fn read_file_missing_vs_empty() {
        let dir = temp_dir("read");
        assert!(read_file(&dir.join("missing.json")).is_none());
        fs::write(dir.join("empty.json"), "").unwrap();
        assert_eq!(read_file(&dir.join("empty.json")).unwrap(), "");
    }
}
