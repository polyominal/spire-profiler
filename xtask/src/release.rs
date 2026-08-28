//! `cargo xtask release`: rebuild the mod and package the bundle as five
//! zips under dist/ — one universal archive plus one per target. Zipping
//! shells out to the `zip` CLI (present on the macOS/Linux hosts the bundle
//! can be built on) rather than implementing zip writing in Rust.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::{bundle, cross, sha256_file, workspace_root};

const OUT_DIR: &str = "dist";

pub fn release(shell: &Shell) -> Result<()> {
    let root = workspace_root();
    // Always rebuild first: a release is only as fresh as its bundle.
    crate::build::build(shell)?;

    let mods_dir = root.join("target/mods");
    let bundle_dir = mods_dir.join(bundle::MOD_ID);
    let version = bundle_version(&bundle_dir)?;
    let out_dir = root.join(OUT_DIR);
    std::fs::create_dir_all(&out_dir)?;
    crate::ensure_cli(shell, "zip", "--version", "packaging")?;

    let mut zips = Vec::new();
    zips.push(zip_universal(shell, &mods_dir, &out_dir, &version)?);
    // One zip per matrix row.
    for row in cross::MATRIX {
        zips.push(zip_target(
            shell,
            &bundle_dir,
            &out_dir,
            &version,
            &format!("{}-{}", row.os, row.arch),
            row.bundle_name,
        )?);
    }
    write_checksums(&out_dir, &zips)?;
    // A failed run may leave the staging dir behind; the next run wipes it.
    let _ = std::fs::remove_dir_all(out_dir.join(".stage"));

    println!(
        "release: {version} -> {} zips + SHA256SUMS under {}",
        zips.len(),
        out_dir.display()
    );
    Ok(())
}

fn bundle_version(bundle_dir: &Path) -> Result<String> {
    let path = bundle_dir.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    manifest_version(&text).map_err(|e| anyhow::anyhow!("in {}: {e}", path.display()))
}

/// Else the zip names would be invented.
fn manifest_version(text: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| anyhow::anyhow!("parsing the manifest as JSON: {e}"))?;
    value
        .get("version")
        .and_then(|version| version.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("the manifest has no string \"version\" field"))
}

/// Godot only consults the running platform's key; the rest are inert.
fn shared_files() -> Vec<String> {
    vec![
        "manifest.json".to_owned(),
        format!("{}.dll", bundle::MOD_ID),
        "spire_profiler.gdextension".to_owned(),
    ]
}

/// Zipping from target/mods makes extraction into mods/ the whole install.
fn zip_universal(shell: &Shell, mods_dir: &Path, out_dir: &Path, version: &str) -> Result<PathBuf> {
    let mod_id = bundle::MOD_ID;
    let out = out_dir.join(format!("{mod_id}-{version}-universal.zip"));
    let _dir = shell.push_dir(mods_dir);
    // -X strips extended attributes; Info-ZIP has no long form for it.
    cmd!(shell, "zip --recurse-paths --quiet -X {out} {mod_id}").run()?;
    Ok(out)
}

fn zip_target(
    shell: &Shell,
    bundle_dir: &Path,
    out_dir: &Path,
    version: &str,
    label: &str,
    lib: &str,
) -> Result<PathBuf> {
    let mod_id = bundle::MOD_ID;
    let out = out_dir.join(format!("{mod_id}-{version}-{label}.zip"));
    let stage_root = out_dir.join(".stage");
    let stage = stage_root.join(label).join(mod_id);

    let _ = std::fs::remove_dir_all(stage_root.join(label));
    std::fs::create_dir_all(&stage)?;
    for shared in &shared_files() {
        copy_file(&bundle_dir.join(shared), &stage.join(shared))?;
    }
    copy_file(&bundle_dir.join(lib), &stage.join(lib))?;

    // A plain block (not a closure) so the scratch dir is cleaned before
    // the error propagates.
    let result = {
        let _dir = shell.push_dir(stage_root.join(label));
        cmd!(shell, "zip --recurse-paths --quiet -X {out} {mod_id}").run()
    };
    let _ = std::fs::remove_dir_all(stage_root.join(label));
    result?;
    Ok(out)
}

/// The sums file sits beside the zips, so it must not embed the output path.
fn write_checksums(out_dir: &Path, zips: &[PathBuf]) -> Result<()> {
    let mut lines = Vec::new();
    for zip in zips {
        let name = zip
            .file_name()
            .expect("zip paths always end in a file name");
        lines.push(format!("{}  {}", sha256_file(zip)?, name.to_string_lossy()));
    }
    std::fs::write(out_dir.join("SHA256SUMS"), lines.join("\n") + "\n")?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_version_extracts_the_stamped_version() {
        let manifest = r#"{"id":"spire-profiler","version":"0.111.0-7392525","has_dll":true}"#;
        assert_eq!(manifest_version(manifest).unwrap(), "0.111.0-7392525");
        assert!(manifest_version(r#"{"version":42}"#).is_err());
        assert!(manifest_version(r#"{"id":"x"}"#).is_err());
    }
}
