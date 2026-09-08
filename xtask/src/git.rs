//! Developer builds tolerate missing Git metadata; releases require a
//! committed HEAD and a clean index and worktree, including untracked files.

use anyhow::{Context, Result, ensure};
use xshell::{Shell, cmd};

pub(crate) fn release_commit(shell: &Shell) -> Result<String> {
    let commit = cmd!(shell, "git rev-parse --verify")
        .arg("HEAD^{commit}")
        .read()
        .context("release requires a committed Git HEAD")?;
    let status = cmd!(
        shell,
        "git status --porcelain=v1 --untracked-files=all --ignore-submodules=none"
    )
    .read()
    .context("checking release inputs for changes")?;
    ensure!(
        status.is_empty(),
        "release requires clean Git inputs:\n{status}"
    );
    Ok(commit)
}

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

    #[test]
    fn release_rejects_dirty_inputs_and_git_failures_without_changing_developer_builds()
    -> Result<()> {
        let shell = Shell::new()?;
        let temp = shell.create_temp_dir()?;
        let root = crate::workspace_root();
        let repo = temp.path().join("repo");
        cmd!(shell, "git clone --quiet --shared {root} {repo}").run()?;
        shell.change_dir(&repo);
        let commit = release_commit(&shell)?;
        cmd!(shell, "git config status.showUntrackedFiles no").run()?;
        std::fs::write(repo.join("README.md"), "changed tracked input")?;
        assert!(release_commit(&shell).is_err());
        assert_ne!(resolve_commit(&shell), "unknown");
        cmd!(shell, "git add README.md").run()?;
        assert!(release_commit(&shell).is_err());
        cmd!(
            shell,
            "git restore --source=HEAD --staged --worktree README.md"
        )
        .run()?;
        std::fs::write(repo.join("untracked-input"), "new input")?;
        assert!(release_commit(&shell).is_err());
        std::fs::remove_file(repo.join("untracked-input"))?;
        assert_eq!(release_commit(&shell)?, commit);
        std::fs::write(repo.join(".git/index"), "invalid index")?;
        assert!(release_commit(&shell).is_err());

        shell.change_dir(temp.path());
        assert!(release_commit(&shell).is_err());
        assert_eq!(resolve_commit(&shell), "unknown");
        cmd!(shell, "git init --quiet unborn").run()?;
        shell.change_dir(temp.path().join("unborn"));
        assert!(release_commit(&shell).is_err());
        assert_eq!(resolve_commit(&shell), "unknown");
        Ok(())
    }
}
