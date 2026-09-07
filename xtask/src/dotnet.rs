//! Hermetic .NET SDK bootstrap: a pinned tarball into the gitignored
//! dotnet-sdk/, hash-verified before extraction and resolved NEVER through
//! PATH. The build never uses NuGet: SDK 9.0.x bundles the net9.0 targeting
//! pack the shim csproj targets, so a newer major would quietly fetch it
//! from nuget.org on first build.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::workspace_root;

/// Bump the version, urls, and sha512s together from Microsoft's release
/// metadata (the hashes it publishes are SHA-512).
pub const DOTNET_VERSION: &str = "9.0.317";

pub fn bootstrap_dir() -> PathBuf {
    workspace_root().join("dotnet-sdk")
}

pub fn binary_in(dir: &Path) -> PathBuf {
    dir.join("dotnet")
}

pub fn pick(bootstrap_dir: &Path) -> Result<PathBuf> {
    let bootstrapped = binary_in(bootstrap_dir);
    if bootstrapped.is_file() {
        Ok(bootstrapped)
    } else {
        Err(anyhow::anyhow!(
            "no .NET SDK bootstrap at {}; run `cargo xtask install-tool` to bootstrap the \
             pinned SDK {DOTNET_VERSION}",
            bootstrap_dir.display()
        ))
    }
}

/// ALWAYS the bootstrap dir, never PATH; a wrong SDK can never silently
/// feed the build.
pub fn resolve_dotnet(shell: &Shell) -> Result<PathBuf> {
    let dir = bootstrap_dir();
    ensure_bootstrap_in(shell, &dir)?;
    let binary = pick(&dir)?;

    println!(
        "dotnet: {} ({DOTNET_VERSION}, bootstrapped)",
        binary.display()
    );
    Ok(binary)
}

/// Anything else fails outright (the build only runs on the four
/// macOS/Linux arches).
struct Pin {
    url: &'static str,
    sha512: &'static str,
}

fn pin(os: &str, arch: &str) -> Result<Pin> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(Pin {
            url: "https://builds.dotnet.microsoft.com/dotnet/Sdk/9.0.317/dotnet-sdk-9.0.317-osx-arm64.tar.gz",
            sha512: "f707a1c73e84c6d009baab2a274270bd11bbb58cd8244cf59594fe1662f50225d1665878d3af4e4b9649b6feccd95b693cf9cf28e127742b7a4e6287caa3eb2a",
        }),
        ("macos", "x86_64") => Ok(Pin {
            url: "https://builds.dotnet.microsoft.com/dotnet/Sdk/9.0.317/dotnet-sdk-9.0.317-osx-x64.tar.gz",
            sha512: "6ddc8617a4cca37ffe03f4b9482f5c73f45cb06b1afa2622b7bc13a870a7869ee298d9af8e40822cbf88e9087c3c3c5d20d182cd481d09cd83165ed9d80dfa11",
        }),
        ("linux", "x86_64") => Ok(Pin {
            url: "https://builds.dotnet.microsoft.com/dotnet/Sdk/9.0.317/dotnet-sdk-9.0.317-linux-x64.tar.gz",
            sha512: "145bf69dcb88c4b905feb531cfdd7894a75fc875d2a030e958a13d1fb1131521c8cebd8a8a6e0fbd1a433ebae9cde86356b6adad07b1ad81efb92b36ff8a3333",
        }),
        ("linux", "aarch64") => Ok(Pin {
            url: "https://builds.dotnet.microsoft.com/dotnet/Sdk/9.0.317/dotnet-sdk-9.0.317-linux-arm64.tar.gz",
            sha512: "fdf30fe705c91304d890115e955f738055f8c0885ea9891e7df1153321120fa2c38b6ae4dd132f871cb8facc0d1fabbd2b25ddd53d0a5b4293aa85d296e3b98d",
        }),
        _ => Err(anyhow::anyhow!(
            "no .NET SDK bootstrap for this host ({os}/{arch})"
        )),
    }
}

pub fn bootstrap_present(dir: &Path) -> bool {
    binary_in(dir).is_file()
}

/// DOTNET_ROOT is pinned so a stray env value cannot hijack the relocated
/// SDK's root resolution.
fn list_sdks_of(shell: &Shell, dir: &Path, binary: &Path) -> Result<String> {
    let _root = shell.push_env("DOTNET_ROOT", dir);
    Ok(Shell::cmd(shell, binary).arg("--list-sdks").read()?)
}

/// The bootstrap dir must hold exactly the pinned SDK: with a second SDK
/// alongside, msbuild would pick the newest and reintroduce the NuGet
/// targeting-pack fetch the pin exists to prevent.
fn installed_version(sdks: &str) -> Option<&str> {
    let mut lines = sdks.lines();
    let version = lines.next()?.split_whitespace().next()?;
    if lines.next().is_some() {
        return None;
    }
    Some(version)
}

pub fn ensure_bootstrap(shell: &Shell) -> Result<()> {
    ensure_bootstrap_in(shell, &bootstrap_dir())
}

pub fn ensure_bootstrap_in(shell: &Shell, dir: &Path) -> Result<()> {
    let pin = pin(std::env::consts::OS, std::env::consts::ARCH)?;
    let tarball = dir.join(format!("dotnet-sdk-{DOTNET_VERSION}.tar.gz"));
    if bootstrap_present(dir) {
        let current = list_sdks_of(shell, dir, &binary_in(dir))
            .is_ok_and(|sdks| installed_version(&sdks) == Some(DOTNET_VERSION));
        if current {
            return Ok(());
        }
        let cached_tarball_matches = sha512_matches(&tarball, pin.sha512)?;
        // The install is host state, but a verified tarball is the reusable
        // download cache; preserve it while removing every extracted file.
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == tarball && cached_tarball_matches {
                continue;
            }
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }
    }
    crate::ensure_cli(shell, "curl", "--version", "the .NET SDK bootstrap")?;
    crate::ensure_cli(shell, "tar", "--version", "the .NET SDK bootstrap")?;
    println!(
        "dotnet bootstrap: downloading the pinned .NET SDK {DOTNET_VERSION} into {} (first \
         run only; ~250 MB, cached for subsequent runs)",
        dir.display()
    );
    std::fs::create_dir_all(dir)?;
    let url = pin.url;
    if !sha512_matches(&tarball, pin.sha512)? {
        cmd!(
            shell,
            "curl --location --fail --silent --show-error --retry 3 --output {tarball} {url}"
        )
        .run()?;
        if !sha512_matches(&tarball, pin.sha512)? {
            let _ = std::fs::remove_file(&tarball);
            return Err(anyhow::anyhow!(
                "dotnet bootstrap: sha512 mismatch for {} (expected {}); the bad download was \
                 removed, re-run `cargo xtask install-tool`",
                tarball.display(),
                pin.sha512
            ));
        }
    }
    cmd!(shell, "tar -xzf {tarball} -C {dir}").run()?;
    let binary = binary_in(dir);
    let sdks = list_sdks_of(shell, dir, &binary).map_err(|e| {
        anyhow::anyhow!(
            "dotnet bootstrap: the verified SDK at {} cannot run on this host: {e}",
            binary.display()
        )
    })?;
    if installed_version(&sdks) != Some(DOTNET_VERSION) {
        return Err(anyhow::anyhow!(
            "dotnet bootstrap: extracted SDK reports '{}', expected {DOTNET_VERSION}",
            sdks.lines().next().unwrap_or("no SDKs")
        ));
    }
    println!(
        "dotnet bootstrap: installed {DOTNET_VERSION} into {}",
        dir.display()
    );
    Ok(())
}

fn sha512_matches(tarball: &Path, expected: &str) -> Result<bool> {
    if !tarball.is_file() {
        return Ok(false);
    }
    Ok(crate::sha512_file(tarball)? == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_version_accepts_exactly_one_sdk() {
        assert_eq!(
            installed_version("9.0.317 [/repo/dotnet-sdk/sdk]\n"),
            Some("9.0.317")
        );
        assert_eq!(installed_version(""), None);
        // A second SDK alongside the pin must fail closed.
        assert_eq!(installed_version("9.0.317 [/x]\n10.0.302 [/x]\n"), None);
    }

    #[test]
    fn pin_urls_embed_the_version() {
        for (os, arch) in [
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
        ] {
            let pin = pin(os, arch).expect("every supported host has a pin");
            assert!(
                pin.url.contains(DOTNET_VERSION),
                "the {os}/{arch} url must embed {DOTNET_VERSION}: {}",
                pin.url
            );
            assert_eq!(pin.sha512.len(), 128, "SHA-512 hex is 128 chars");
        }
        assert!(
            pin("windows", "x86_64").is_err(),
            "native Windows has no pin"
        );
    }
}
