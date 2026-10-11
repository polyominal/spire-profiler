//! Hermetic zig bootstrap: a pinned tarball into the gitignored zig-sdk/,
//! resolved through CARGO_ZIGBUILD_ZIG_PATH (never PATH). install-tool
//! provisions it eagerly; the build bootstraps it lazily when missing.

use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::{ensure_cli, workspace_root};

/// Bump the version, urls, and shasums together from ziglang.org's index.
pub const ZIG_VERSION: &str = "0.17.0";

pub fn bootstrap_dir() -> PathBuf {
    workspace_root().join("zig-sdk")
}

pub fn binary_in(dir: &Path) -> PathBuf {
    dir.join("zig")
}

/// ALWAYS the bootstrap dir, never PATH; a wrong zig can never silently
/// feed the build.
pub fn resolve_zig(shell: &Shell) -> Result<PathBuf> {
    let dir = bootstrap_dir();
    ensure_bootstrap_in(shell, &dir)?;
    let binary = binary_in(&dir);
    println!("zig: {} ({ZIG_VERSION}, bootstrapped)", binary.display());
    Ok(binary)
}

/// Anything else fails outright (the build only runs on the four
/// macOS/Linux arches).
struct Pin {
    url: &'static str,
    sha256: &'static str,
}

fn pin(os: &str, arch: &str) -> Result<Pin> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(Pin {
            url: "https://ziglang.org/download/0.17.0/zig-aarch64-macos-0.17.0.tar.xz",
            sha256: "b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a",
        }),
        ("macos", "x86_64") => Ok(Pin {
            url: "https://ziglang.org/download/0.17.0/zig-x86_64-macos-0.17.0.tar.xz",
            sha256: "4f9a1c5269aa17ebda5e6d3c2b89d6cbf36f7d2b22a0306e9ab98f25f95529c6",
        }),
        ("linux", "x86_64") => Ok(Pin {
            url: "https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz",
            sha256: "1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026",
        }),
        ("linux", "aarch64") => Ok(Pin {
            url: "https://ziglang.org/download/0.17.0/zig-aarch64-linux-0.17.0.tar.xz",
            sha256: "9e8d11661d4ae3bd57702a3832781e23ad151dde5798e16a5ccd503f65234ff8",
        }),
        _ => Err(anyhow::anyhow!(
            "no zig bootstrap for this host ({os}/{arch})"
        )),
    }
}

pub fn bootstrap_present(dir: &Path) -> bool {
    binary_in(dir).is_file()
}

pub fn ensure_bootstrap(shell: &Shell) -> Result<()> {
    ensure_bootstrap_in(shell, &bootstrap_dir())
}

pub fn ensure_bootstrap_in(shell: &Shell, dir: &Path) -> Result<()> {
    if bootstrap_present(dir) {
        let version = zig_version_of(shell, &binary_in(dir))?;
        if version == ZIG_VERSION {
            return Ok(());
        }
        fs::remove_dir_all(dir)?;
    }
    ensure_cli(shell, "curl", "--version", "the zig bootstrap")?;
    ensure_cli(shell, "tar", "--version", "the zig bootstrap")?;
    let pin = pin(env::consts::OS, env::consts::ARCH)?;
    println!(
        "zig bootstrap: downloading the pinned zig {ZIG_VERSION} into {} (first run only; \
         ~50 MB, cached for subsequent runs)",
        dir.display()
    );
    fs::create_dir_all(dir)?;
    let tarball = dir.join(format!("zig-{ZIG_VERSION}.tar.xz"));
    let url = pin.url;
    if !sha256_matches(&tarball, pin.sha256)? {
        cmd!(
            shell,
            "curl --location --silent --show-error --retry 3 --output {tarball} {url}"
        )
        .run()?;
        if !sha256_matches(&tarball, pin.sha256)? {
            let _ = fs::remove_file(&tarball);
            return Err(anyhow::anyhow!(
                "zig bootstrap: sha256 mismatch for {} (expected {}); the bad download was \
                 removed, re-run `cargo xtask install-tool`",
                tarball.display(),
                pin.sha256
            ));
        }
    }
    cmd!(
        shell,
        "tar --extract --xz --file {tarball} --strip-components=1 --directory {dir}"
    )
    .run()?;
    let version = zig_version_of(shell, &binary_in(dir))?;
    if version != ZIG_VERSION {
        return Err(anyhow::anyhow!(
            "zig bootstrap: extracted zig is '{version}', expected {ZIG_VERSION}"
        ));
    }
    println!(
        "zig bootstrap: installed {ZIG_VERSION} into {}",
        dir.display()
    );
    Ok(())
}

fn zig_version_of(shell: &Shell, binary: &Path) -> Result<String> {
    Ok(Shell::cmd(shell, binary)
        .arg("version")
        .read()
        .map(|output| output.trim().to_string())?)
}

fn sha256_matches(tarball: &Path, expected: &str) -> Result<bool> {
    if !tarball.is_file() {
        return Ok(false);
    }
    Ok(crate::sha256_file(tarball)? == expected)
}
