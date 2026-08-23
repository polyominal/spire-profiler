//! Assembly of the installable mod bundle under target/mods/spire-profiler/:
//! manifest + C# host dll + the multi-key .gdextension + one native library
//! per platform key, each under the EXACT file name its \[libraries\] entry
//! names. The .gdextension is rendered from the build matrix, so the keys
//! and file names cannot drift from the libraries the build produces.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cross::{self, MATRIX};

/// The dotnet assembly is SpireProfiler; the game loads \<id\>.dll, so the
/// bundle renames the built dll to the id.
pub(crate) const MOD_ID: &str = "spire-profiler";

pub(crate) fn assemble_bundle(
    root: &Path,
    gen_dir: &Path,
    mod_dir: &Path,
    libs: &[(String, PathBuf)],
    commit: &str,
) -> Result<()> {
    // Wipe first so a removed library cannot linger.
    let _ = std::fs::remove_dir_all(mod_dir);
    std::fs::create_dir_all(mod_dir)?;
    write_manifest(root, mod_dir, commit)?;
    copy_file(
        &gen_dir.join("bin/SpireProfiler.dll"),
        &mod_dir.join(format!("{MOD_ID}.dll")),
    )?;
    // Each library lands under its .gdextension key's file name (the two
    // macOS builds both output libprofiler_core.dylib and get their arch
    // suffix here).
    for (name, source) in libs {
        copy_file(source, &mod_dir.join(name))?;
    }
    // There is no committed copy to drift.
    std::fs::write(
        mod_dir.join("spire_profiler.gdextension"),
        cross::render_gdextension(),
    )?;
    Ok(())
}

fn write_manifest(root: &Path, mod_dir: &Path, commit: &str) -> Result<()> {
    let template = std::fs::read_to_string(root.join("manifest.template.json"))
        .map_err(|e| anyhow::anyhow!("reading manifest.template.json: {e}"))?;
    let rendered = template.replace("@VERSION@", &manifest_version(commit));
    // Always checked (not a debug_assert): a manifest with the raw
    // placeholder must never reach the bundle.
    anyhow::ensure!(
        !rendered.contains("@VERSION@"),
        "the manifest template placeholder @VERSION@ was not substituted (the placeholder \
         must never reach the bundle)"
    );
    std::fs::write(mod_dir.join("manifest.json"), rendered)?;
    Ok(())
}

/// Degrades to \<pin\>-unknown when the commit cannot be resolved — the
/// build must never break over this.
fn manifest_version(commit: &str) -> String {
    format!("{}-{commit}", crate::game_version::PIN)
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    std::fs::copy(source, destination).map(|_| ()).map_err(|e| {
        anyhow::anyhow!(
            "copying {} -> {}: {e}",
            source.display(),
            destination.display()
        )
    })
}

/// Iterates the matrix directly — the .gdextension is rendered from the
/// same rows, so a missing file is a build gap.
pub(crate) fn missing_gdextension_libraries(mod_dir: &Path) -> Vec<String> {
    MATRIX
        .iter()
        .filter(|row| !mod_dir.join(row.bundle_name).is_file())
        .map(|row| format!("{}.{} -> {}", row.os, row.arch, row.bundle_name))
        .collect()
}
