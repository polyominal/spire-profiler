//! Short-hash resolution via `git rev-parse`. Any failure degrades to
//! "unknown" so the build never breaks over the build id.

use xshell::{Shell, cmd};

pub fn resolve_commit(shell: &Shell) -> String {
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
        let shell = Shell::new().expect("cargo test always runs with a working directory");
        shell.change_dir(crate::workspace_root());
        let hash = resolve_commit(&shell);
        assert_eq!(hash.len(), 8, "expected an 8-char short hash, got {hash:?}");
        assert!(
            hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "expected hex characters, got {hash:?}"
        );
    }
}
