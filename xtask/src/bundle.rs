//! Assembly of the installable bundle: manifest, managed host, and the native
//! attribution reducer libraries selected by the platform matrix.

use std::path::Path;

use anyhow::Result;

use crate::cross;

/// The dotnet assembly is SpireProfiler; the game loads \<id\>.dll, so the
/// bundle renames the built dll to the id.
pub(crate) const MOD_ID: &str = "spire-profiler";

pub(crate) fn assemble_bundle(
    root: &Path,
    gen_dir: &Path,
    mod_dir: &Path,
    libs: &[cross::NativeArtifact],
    commit: &str,
) -> Result<()> {
    // Wipe first so a removed library cannot linger.
    match std::fs::remove_dir_all(mod_dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(anyhow::anyhow!("removing {}: {error}", mod_dir.display()));
        }
        _ => {}
    }
    std::fs::create_dir_all(mod_dir)?;
    write_manifest(root, mod_dir, commit)?;
    copy_file(
        &gen_dir.join("bin/SpireProfiler.dll"),
        &mod_dir.join(format!("{MOD_ID}.dll")),
    )?;
    // Both macOS builds use the same Cargo output name and get their
    // architecture suffix here.
    for (name, source) in libs {
        copy_file(source, &mod_dir.join(name))?;
    }
    Ok(())
}

fn write_manifest(root: &Path, mod_dir: &Path, commit: &str) -> Result<()> {
    let template = std::fs::read_to_string(root.join("manifest.template.json"))
        .map_err(|e| anyhow::anyhow!("reading manifest.template.json: {e}"))?;
    let rendered = template.replace("@VERSION@", &manifest_version(commit));
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
