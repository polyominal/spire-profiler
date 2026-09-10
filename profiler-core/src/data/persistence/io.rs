//! Atomic file IO and the data-directory layout: [`write_file`]'s
//! tmp+rename, [`read_file`]'s whole-file reads under the 64 MiB budget,
//! and [`ensure_data_dir`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::MAX_JSON_SIZE;
use crate::data::state::STATE;
use crate::fail;

/// Creates the configured store; false before init or on a logged IO error.
pub fn ensure_data_dir() -> bool {
    let Some(runs_dir) = STATE.with(|s| {
        s.borrow()
            .store_paths
            .as_ref()
            .map(|paths| paths.runs_dir.clone())
    }) else {
        return false;
    };
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
    if bytes.len() > MAX_JSON_SIZE {
        fail!(
            "cannot write '{}': {} bytes exceeds the {MAX_JSON_SIZE}-byte limit",
            path.display(),
            bytes.len()
        );
        return false;
    }
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

pub(crate) enum ReadFile {
    Missing,
    Content(String),
    Failed,
}

/// Missing directories are empty; every other scan failure rejects the
/// whole listing so an incomplete scan cannot seed record IDs.
pub(crate) fn read_dir(path: &Path) -> Option<Vec<fs::DirEntry>> {
    let result = match fs::read_dir(path) {
        Ok(entries) => entries.collect::<std::io::Result<Vec<_>>>(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
        Err(err) => Err(err),
    };
    match result {
        Ok(entries) => Some(entries),
        Err(err) => {
            fail!("cannot scan '{}': {err}", path.display());
            None
        }
    }
}

/// An empty file yields a zero-length string: "no data yet" is a state.
pub(crate) fn read_file(path: &Path) -> ReadFile {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ReadFile::Missing,
        Err(err) => {
            fail!(
                "cannot open '{}': {} (os error {})",
                path.display(),
                err.kind(),
                err.raw_os_error().unwrap_or(-1)
            );
            return ReadFile::Failed;
        }
    };
    if meta.len() > MAX_JSON_SIZE as u64 {
        fail!(
            "'{}' is too large to read ({} bytes > {MAX_JSON_SIZE})",
            path.display(),
            meta.len()
        );
        return ReadFile::Failed;
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
            return ReadFile::Failed;
        }
    };
    match String::from_utf8(bytes) {
        Ok(content) => ReadFile::Content(content),
        Err(err) => {
            fail!("'{}' is not valid UTF-8: {err}", path.display());
            ReadFile::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;

    #[test]
    fn ensure_data_dir_creates_both_dirs() {
        let dir = unique_dir("ensure");
        init_state(&dir.join("data"));
        assert!(ensure_data_dir());
        assert!(dir.join("data").is_dir());
        assert!(dir.join("data/runs").is_dir());
    }

    #[test]
    fn ensure_data_dir_reports_failure() {
        // A blocker file where a directory component belongs makes
        // create_dir_all fail without crashing.
        let dir = unique_dir("ensure-fail");
        fs::write(dir.join("blocker"), "x").unwrap();
        init_state(&dir.join("blocker/data"));
        assert!(!ensure_data_dir());
    }

    #[test]
    fn write_file_round_trips_and_leaves_no_temp() {
        let dir = unique_dir("write");
        let path = dir.join("f.json");
        assert!(write_file(&path, "hello"));
        assert!(matches!(read_file(&path), ReadFile::Content(text) if text == "hello"));
        assert!(!dir.join("f.json.tmp").exists());
        assert!(write_file(&path, "bye"));
        assert!(matches!(read_file(&path), ReadFile::Content(text) if text == "bye"));
        assert!(!dir.join("f.json.tmp").exists());
    }

    #[test]
    fn write_file_failures_preserve_existing_content() {
        let dir = crate::test_util::unique_dir("write-file-failures");
        let path = dir.join("f.json");
        let tmp = dir.join("f.json.tmp");
        fs::write(&path, "old").unwrap();
        assert!(!write_file(&path.join("child"), "new"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        fs::create_dir(&tmp).unwrap();
        assert!(!write_file(&path, "new"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        fs::remove_dir(&tmp).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("old"), "preserved").unwrap();
        assert!(!write_file(&path, "new"));
        assert_eq!(fs::read_to_string(path.join("old")).unwrap(), "preserved");
        assert!(!tmp.exists(), "failed rename cleans up its temporary file");
    }

    #[test]
    fn read_file_missing_vs_empty() {
        let dir = unique_dir("read");
        assert!(matches!(
            read_file(&dir.join("missing.json")),
            ReadFile::Missing
        ));
        fs::write(dir.join("empty.json"), "").unwrap();
        assert!(
            matches!(read_file(&dir.join("empty.json")), ReadFile::Content(text) if text.is_empty())
        );
    }

    #[test]
    fn read_file_reports_invalid_utf8_as_failed() {
        let dir = unique_dir("read-invalid");
        let path = dir.join("invalid.jsonl");
        fs::write(&path, [0xff]).unwrap();
        assert!(matches!(read_file(&path), ReadFile::Failed));
    }
}
