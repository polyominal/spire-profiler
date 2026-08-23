//! Short-hash resolution via `git rev-parse`. Any failure degrades to
//! "unknown" so the build never breaks over the build id.

use std::path::Path;

use xshell::{Shell, cmd};

pub fn resolve_commit(root: &Path) -> String {
    let shell = match Shell::new() {
        Ok(shell) => shell,
        Err(_) => return "unknown".to_owned(),
    };
    let _dir = shell.push_dir(root);
    cmd!(shell, "git rev-parse --short=8 HEAD")
        .read()
        .map(|output| output.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_checked_out_commit_to_an_8_char_hex_hash() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives one level below the workspace root");
        let hash = resolve_commit(root);
        assert_eq!(hash.len(), 8, "expected an 8-char short hash, got {hash:?}");
        assert!(
            hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "expected hex characters, got {hash:?}"
        );
    }
}
