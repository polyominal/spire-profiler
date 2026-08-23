//! The bundle's native matrix: one library per (os, arch). The matrix is
//! the single source of truth for the platform rows: the .gdextension
//! `[libraries]` section, the C# host's runtime selector, and the release
//! zips all derive from it. The library names are specified nowhere else.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

/// `os`/`arch` key the .gdextension row; the triples drive cargo-zigbuild
/// and locate the artifact; `cdylib` is renamed to `bundle_name`. The Linux
/// triple carries the `.2.17` glibc floor (the oldest distro Steam still
/// supports); cargo strips the suffix from the output dir.
pub(crate) struct NativeRow {
    pub(crate) os: &'static str,
    pub(crate) arch: &'static str,
    pub(crate) zigbuild_triple: &'static str,
    pub(crate) dir_triple: &'static str,
    pub(crate) cdylib: &'static str,
    pub(crate) bundle_name: &'static str,
}

pub(crate) const MATRIX: &[NativeRow] = &[
    NativeRow {
        os: "macos",
        arch: "arm64",
        zigbuild_triple: "aarch64-apple-darwin",
        dir_triple: "aarch64-apple-darwin",
        cdylib: "libprofiler_core.dylib",
        bundle_name: "libprofiler_core.macos.arm64.dylib",
    },
    NativeRow {
        os: "macos",
        arch: "x86_64",
        zigbuild_triple: "x86_64-apple-darwin",
        dir_triple: "x86_64-apple-darwin",
        cdylib: "libprofiler_core.dylib",
        bundle_name: "libprofiler_core.macos.x86_64.dylib",
    },
    NativeRow {
        os: "linux",
        arch: "x86_64",
        zigbuild_triple: "x86_64-unknown-linux-gnu.2.17",
        dir_triple: "x86_64-unknown-linux-gnu",
        cdylib: "libprofiler_core.so",
        bundle_name: "libprofiler_core.linux.x86_64.so",
    },
    NativeRow {
        os: "windows",
        arch: "x86_64",
        zigbuild_triple: "x86_64-pc-windows-gnu",
        dir_triple: "x86_64-pc-windows-gnu",
        cdylib: "profiler_core.dll",
        bundle_name: "libprofiler_core.windows.x86_64.dll",
    },
];

/// The keys and file names live only here; no committed copy to drift.
pub(crate) fn render_gdextension() -> String {
    let libraries = MATRIX
        .iter()
        .map(|row| format!("{}.{} = \"{}\"", row.os, row.arch, row.bundle_name))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[configuration]\n\
         entry_symbol = \"gdextension_entry\"\n\
         compatibility_minimum = \"4.5\"\n\
         \n\
         [libraries]\n\
         {libraries}\n"
    )
}

pub(crate) fn build_matrix(shell: &Shell, root: &Path) -> Result<Vec<(String, PathBuf)>> {
    ensure_zigbuild(shell)?;
    let zig = crate::zig::resolve_zig(shell)?;
    // Pin the resolved zig (never PATH) for the whole native build.
    let _zigbuild_zig_path = shell.push_env("CARGO_ZIGBUILD_ZIG_PATH", zig.binary.as_path());
    ensure_targets(shell)?;

    let mut libs = Vec::new();
    let mut zigbuild = Shell::cmd(shell, "cargo");
    zigbuild = zigbuild
        .arg("zigbuild")
        .arg("--release")
        .arg("-p")
        .arg("profiler_core");
    for row in MATRIX {
        zigbuild = zigbuild.arg("--target").arg(row.zigbuild_triple);
    }
    zigbuild.run()?;
    for row in MATRIX {
        let artifact = root
            .join("target")
            .join(row.dir_triple)
            .join("release")
            .join(row.cdylib);
        println!(
            "cross: {} built at {}",
            row.zigbuild_triple,
            artifact.display()
        );
        libs.push((row.bundle_name.to_string(), artifact));
    }
    Ok(libs)
}

pub(crate) const ZIGBUILD_VERSION: &str = "0.23.0";

fn ensure_zigbuild(shell: &Shell) -> Result<()> {
    let remedy = format!(
        "install it with `cargo +stable install cargo-zigbuild --version {ZIGBUILD_VERSION} --locked`"
    );
    let output = cmd!(shell, "cargo-zigbuild --version")
        .read()
        .map_err(|_| {
            anyhow::anyhow!(
                "cross-compilation needs cargo-zigbuild, which is not installed; {remedy}"
            )
        })?;
    if !output.contains(ZIGBUILD_VERSION) {
        return Err(anyhow::anyhow!(
            "cross-compilation needs cargo-zigbuild {ZIGBUILD_VERSION}, but the installed one \
             reports '{}'; {remedy}",
            output.trim()
        ));
    }
    Ok(())
}

/// Zig supplies only the linker and C libraries; rustc still needs each
/// target's std.
pub(crate) fn ensure_targets(shell: &Shell) -> Result<()> {
    let installed = installed_targets(shell)?;
    let missing: Vec<&str> = MATRIX
        .iter()
        .map(|row| row.dir_triple)
        .filter(|triple| !installed.lines().any(|line| line.trim() == *triple))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let targets = missing.join(" ");
    println!("cross: installing rust stds for: {targets}");
    cmd!(shell, "rustup target add {missing...}")
        .run()
        .map_err(|e| {
            anyhow::anyhow!(
                "installing the missing rust stds via `rustup target add {targets}` failed: {e}"
            )
        })
}

fn installed_targets(shell: &Shell) -> Result<String> {
    Ok(cmd!(shell, "rustup target list --installed").read()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate would silently shadow a library in the bundle or in
    /// Godot's lookup.
    #[test]
    fn matrix_keys_and_bundle_names_are_unique() {
        let mut keys: Vec<String> = MATRIX
            .iter()
            .map(|r| format!("{}.{}", r.os, r.arch))
            .collect();
        keys.sort_unstable();
        let mut unique_keys = keys.clone();
        unique_keys.dedup();
        assert_eq!(
            unique_keys, keys,
            "matrix rows must have distinct os.arch keys"
        );

        let mut names: Vec<&str> = MATRIX.iter().map(|r| r.bundle_name).collect();
        names.sort_unstable();
        let mut unique_names = names.clone();
        unique_names.dedup();
        assert_eq!(unique_names, names, "bundle names must be distinct");
    }

    #[test]
    fn rendered_gdextension_matches_the_matrix() {
        let rendered = render_gdextension();
        assert!(rendered.contains("[configuration]\n"));
        assert!(rendered.contains("entry_symbol = \"gdextension_entry\"\n"));
        assert!(rendered.contains("compatibility_minimum = \"4.5\"\n"));
        let libraries = rendered
            .split_once("[libraries]\n")
            .expect("rendered extension has a libraries section")
            .1;
        assert_eq!(libraries.lines().count(), MATRIX.len());
        for row in MATRIX {
            let entry = format!("{}.{} = \"{}\"\n", row.os, row.arch, row.bundle_name);
            assert!(rendered.contains(&entry), "missing rendered entry: {entry}");
        }
    }
}
