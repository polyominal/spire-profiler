//! Hermetic zig bootstrap: a pinned tarball into the gitignored zig-sdk/,
//! resolved through CARGO_ZIGBUILD_ZIG_PATH (never PATH). install-tool
//! provisions it eagerly; the build bootstraps it lazily when missing.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::workspace_root;

/// Bump the version, urls, and shasums together from ziglang.org's index.
pub const ZIG_VERSION: &str = "0.16.0";

pub fn bootstrap_dir() -> PathBuf {
    workspace_root().join("zig-sdk")
}

pub fn binary_in(dir: &Path) -> PathBuf {
    dir.join("zig")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zig {
    pub binary: PathBuf,
    pub version: String,
}

pub fn pick(bootstrap_dir: &Path) -> Result<PathBuf> {
    let bootstrapped = binary_in(bootstrap_dir);
    if bootstrapped.is_file() {
        Ok(bootstrapped)
    } else {
        Err(anyhow::anyhow!(
            "no zig bootstrap at {}; run `cargo xtask install-tool` to bootstrap the pinned \
             zig {ZIG_VERSION}",
            bootstrap_dir.display()
        ))
    }
}

/// ALWAYS the bootstrap dir, never PATH; a wrong zig can never silently
/// feed the build.
pub fn resolve_zig(shell: &Shell) -> Result<Zig> {
    let dir = bootstrap_dir();
    let binary = match pick(&dir) {
        Ok(binary) => binary,
        Err(_) => {
            // The build provisions the pin itself instead of asking for
            // install-tool.
            ensure_bootstrap_in(shell, &dir)?;
            pick(&dir)?
        }
    };
    // Refreshing wipes the dir and reinstalls the pinned tarball.
    if zig_version_of(shell, &binary)? != ZIG_VERSION {
        ensure_bootstrap_in(shell, &dir)?;
    }
    let binary = pick(&dir)?;
    let version = zig_version_of(shell, &binary)?;
    println!("zig: {} ({version}, bootstrapped)", binary.display());
    Ok(Zig { binary, version })
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
            url: "https://ziglang.org/download/0.16.0/zig-aarch64-macos-0.16.0.tar.xz",
            sha256: "b23d70deaa879b5c2d486ed3316f7eaa53e84acf6fc9cc747de152450d401489",
        }),
        ("macos", "x86_64") => Ok(Pin {
            url: "https://ziglang.org/download/0.16.0/zig-x86_64-macos-0.16.0.tar.xz",
            sha256: "0387557ed1877bc6a2e1802c8391953baddba76081876301c522f52977b52ba7",
        }),
        ("linux", "x86_64") => Ok(Pin {
            url: "https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz",
            sha256: "70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00",
        }),
        ("linux", "aarch64") => Ok(Pin {
            url: "https://ziglang.org/download/0.16.0/zig-aarch64-linux-0.16.0.tar.xz",
            sha256: "ea4b09bfb22ec6f6c6ceac57ab63efb6b46e17ab08d21f69f3a48b38e1534f17",
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
        std::fs::remove_dir_all(dir)?;
    }
    let pin = pin(std::env::consts::OS, std::env::consts::ARCH)?;
    println!(
        "zig bootstrap: downloading the pinned zig {ZIG_VERSION} into {} (first run only; \
         ~50 MB, cached for subsequent runs)",
        dir.display()
    );
    std::fs::create_dir_all(dir)?;
    let tarball = dir.join(format!("zig-{ZIG_VERSION}.tar.xz"));
    let url = pin.url;
    if !sha256_matches(&tarball, pin.sha256)? {
        cmd!(
            shell,
            "curl --location --silent --show-error --retry 3 --output {tarball} {url}"
        )
        .run()?;
        if !sha256_matches(&tarball, pin.sha256)? {
            let _ = std::fs::remove_file(&tarball);
            return Err(anyhow::anyhow!(
                "zig bootstrap: sha256 mismatch for {} (expected {}); the bad download was \
                 removed, re-run `cargo xtask install-tool`",
                tarball.display(),
                pin.sha256
            ));
        }
    }
    cmd!(shell, "tar -xJf {tarball} --strip-components=1 -C {dir}").run()?;
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
