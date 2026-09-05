//! Hermetic .NET SDK bootstrap: the pinned channel-9.0 SDK in the
//! gitignored dotnet-sdk/, provisioned by the vendored tools/dotnet-install.sh.
//! The build ALWAYS uses the bootstrap (never PATH) and never NuGet: SDK
//! 9.0.x bundles the net9.0 targeting pack the shim csproj targets.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::Shell;

use crate::workspace_root;

/// Latest 9.0.x GA.
pub const DOTNET_CHANNEL: &str = "9.0";

/// A newer channel would quietly fetch the 9.0 targeting pack from
/// nuget.org.
pub const MIN_MAJOR: u32 = 9;

pub fn bootstrap_dir() -> PathBuf {
    workspace_root().join("dotnet-sdk")
}

pub fn binary_in(dir: &Path) -> PathBuf {
    dir.join("dotnet")
}

pub fn installer_script() -> PathBuf {
    workspace_root().join("tools/dotnet-install.sh")
}

pub fn newest_major(sdk_list: &str) -> Option<u32> {
    sdk_list
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .next()?
                .split('.')
                .next()?
                .parse()
                .ok()
        })
        .max()
}

pub fn pick(bootstrap_dir: &Path) -> Result<PathBuf> {
    let bootstrapped = binary_in(bootstrap_dir);
    if bootstrapped.is_file() {
        Ok(bootstrapped)
    } else {
        Err(anyhow::anyhow!(
            "no .NET SDK bootstrap at {}; run `cargo xtask install-tool` to bootstrap the \
             pinned SDK {DOTNET_CHANNEL}",
            bootstrap_dir.display()
        ))
    }
}

/// A system SDK is never a substitute; another channel is rejected rather
/// than silently restoring a NuGet dependency.
fn verify_channel(binary: &Path, sdks: &str) -> Result<String> {
    match newest_major(sdks) {
        Some(major) if major == MIN_MAJOR => {
            let version = sdks
                .lines()
                .map(|line| line.split_whitespace().next().unwrap_or(""))
                .find(|version| {
                    version
                        .split('.')
                        .next()
                        .and_then(|major| major.parse::<u32>().ok())
                        == Some(MIN_MAJOR)
                })
                .unwrap_or("unknown")
                .to_owned();
            Ok(version)
        }
        Some(major) => Err(anyhow::anyhow!(
            "the .NET SDK at {} is not the pinned channel (newest SDK major {major}, channel \
             {DOTNET_CHANNEL}); run `cargo xtask install-tool` to refresh it",
            binary.display()
        )),
        None => Err(anyhow::anyhow!(
            "the dotnet at {} reports no SDKs; run `cargo xtask install-tool` to refresh the \
             pinned SDK {DOTNET_CHANNEL}",
            binary.display()
        )),
    }
}

/// ALWAYS the bootstrap dir, never PATH.
pub fn resolve_dotnet(shell: &Shell) -> Result<PathBuf> {
    let dir = bootstrap_dir();
    let binary = match pick(&dir) {
        Ok(binary) => binary,
        Err(_) => {
            ensure_bootstrap_in(shell, &dir)?;
            pick(&dir)?
        }
    };
    let sdks = list_sdks_of(shell, &dir, &binary).map_err(|e| {
        anyhow::anyhow!(
            "the dotnet at {} is not runnable; run `cargo xtask install-tool` to refresh the \
             pinned SDK {DOTNET_CHANNEL}: {e}",
            binary.display()
        )
    })?;
    let version = verify_channel(&binary, &sdks)?;
    println!("dotnet: {} ({version}, bootstrapped)", binary.display());
    Ok(binary)
}

pub fn bootstrap_present(dir: &Path) -> bool {
    binary_in(dir).is_file()
}

/// --no-path keeps the install off PATH.
pub fn bootstrap_command(script: &Path, dir: &Path) -> Vec<String> {
    vec![
        script.display().to_string(),
        "--channel".to_owned(),
        DOTNET_CHANNEL.to_owned(),
        "--install-dir".to_owned(),
        dir.display().to_string(),
        "--no-path".to_owned(),
    ]
}

/// DOTNET_ROOT is pinned so a stray env value cannot hijack the relocated
/// SDK's root resolution.
fn list_sdks_of(shell: &Shell, dir: &Path, binary: &Path) -> Result<String> {
    let _root = shell.push_env("DOTNET_ROOT", dir);
    Ok(Shell::cmd(shell, binary).arg("--list-sdks").read()?)
}

pub fn ensure_bootstrap(shell: &Shell) -> Result<()> {
    ensure_bootstrap_in(shell, &bootstrap_dir())
}

/// Idempotent; a wrong-channel SDK is refreshed. macOS/Linux only: the
/// vendored installer is a shell script.
pub fn ensure_bootstrap_in(shell: &Shell, dir: &Path) -> Result<()> {
    if bootstrap_present(dir) {
        let sdks = list_sdks_of(shell, dir, &binary_in(dir))?;
        if newest_major(&sdks) == Some(MIN_MAJOR) {
            println!(
                "dotnet bootstrap: present at {} ({})",
                dir.display(),
                first_line(&sdks)
            );
            return Ok(());
        }
        println!(
            "dotnet bootstrap: {} at {} is not the pinned channel {DOTNET_CHANNEL}; refreshing \
             from the pinned tarball",
            first_line(&sdks),
            dir.display()
        );
        std::fs::remove_dir_all(dir)?;
    }
    println!(
        "dotnet bootstrap: downloading the pinned SDK {DOTNET_CHANNEL} into {} (first run \
         only; ~500 MB, cached for subsequent runs)",
        dir.display()
    );
    let args = bootstrap_command(&installer_script(), dir);
    let _telemetry = shell.push_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    let _skip_first_run = shell.push_env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1");
    // Run through bash explicitly so a lost exec bit cannot matter.
    Shell::cmd(shell, "bash").args(&args).run()?;
    println!(
        "dotnet bootstrap: installed {} into {}",
        first_line(&list_sdks_of(shell, dir, &binary_in(dir))?),
        dir.display()
    );
    Ok(())
}

fn first_line(output: &str) -> &str {
    output.lines().next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_major_parses_sdk_list_output() {
        assert_eq!(
            newest_major("10.0.302 [/usr/local/share/dotnet/sdk]\n9.0.317 [/opt/dotnet/sdk]\n"),
            Some(10)
        );
        assert_eq!(newest_major("9.0.317 [/opt/dotnet/sdk]"), Some(9));
        assert_eq!(newest_major(""), None);
        assert_eq!(newest_major("no sdks here"), None);
    }

    #[test]
    fn verify_channel_rejects_every_channel_but_the_pin() {
        let binary = Path::new("/repo/dotnet-sdk/dotnet");
        let version =
            verify_channel(binary, "9.0.317 [/repo/dotnet-sdk/sdk]").expect("the pin must pass");
        assert_eq!(version, "9.0.317");

        let err = verify_channel(binary, "10.0.302 [/x]").expect_err("a 10.x SDK must fail");
        let err = err.to_string();
        assert!(err.contains("major 10"), "must name the found major: {err}");
        assert!(err.contains(DOTNET_CHANNEL), "must name the channel: {err}");
        assert!(
            err.contains("`cargo xtask install-tool`"),
            "must be actionable: {err}"
        );

        verify_channel(binary, "").expect_err("an SDK-less dir must fail");
    }
}
