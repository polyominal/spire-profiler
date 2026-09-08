//! Info-ZIP updates existing archives, so releases write fresh ZIPs in
//! dist/.stage. Failed staging is discarded on the next attempt; explicit file
//! lists select each platform from the checked bundle.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use xshell::{Shell, cmd};

use crate::{bundle, cross, sha256_file, workspace_root};

const SHARED_FILES: [&str; 3] = [
    "manifest.json",
    "spire-profiler.dll",
    "spire_profiler.gdextension",
];

pub fn release(shell: &Shell) -> Result<()> {
    let commit = crate::git::release_commit(shell)?;
    crate::smoke(shell)?;
    crate::build::build(shell)?;
    ensure!(
        crate::git::release_commit(shell)? == commit,
        "HEAD changed during the release build"
    );
    let short = cmd!(shell, "git rev-parse --short=8 {commit}").read()?;
    let version = format!("{}-{short}", crate::game_version::PIN);
    let root = workspace_root();
    let out_dir = root.join("dist");
    crate::ensure_cli(shell, "zip", "--version", "packaging")?;
    crate::ensure_cli(shell, "unzip", "-v", "archive validation")?;
    let bundle_dir = root.join("target/mods").join(bundle::MOD_ID);
    package(shell, &bundle_dir, &out_dir, &version)?;
    println!("release: {version} -> {}", out_dir.display());
    Ok(())
}

fn package(shell: &Shell, bundle_dir: &Path, out_dir: &Path, version: &str) -> Result<()> {
    let mut universal = SHARED_FILES.to_vec();
    universal.extend(cross::MATRIX.iter().map(|row| row.bundle_name));
    validate_files(bundle_dir, &universal)?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle_dir.join("manifest.json"))?)?;
    ensure!(
        manifest["version"].as_str() == Some(version),
        "bundle version differs from release commit"
    );
    let stage = out_dir.join(".stage");
    match fs::remove_dir_all(&stage) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).context("removing stale release staging directory");
        }
        _ => {}
    }
    fs::create_dir_all(&stage)?;
    let mut variants = vec![("universal".to_owned(), universal)];
    for row in cross::MATRIX {
        let mut files = SHARED_FILES.to_vec();
        files.push(row.bundle_name);
        variants.push((format!("{}-{}", row.os, row.arch), files));
    }
    let mods_dir = bundle_dir
        .parent()
        .context("bundle has no parent directory")?;
    let _dir = shell.push_dir(mods_dir);
    let mut names = Vec::new();
    let mut sums = String::new();
    for (label, files) in variants {
        let name = format!("{}-{version}-{label}.zip", bundle::MOD_ID);
        let zip = stage.join(&name);
        let paths = files
            .iter()
            .map(|file| format!("{}/{file}", bundle::MOD_ID));
        cmd!(shell, "zip --must-match --quiet --strip-extra --test {zip}")
            .args(paths)
            .env("ZIPOPT", "")
            .run()?;
        sums.push_str(&format!("{}  {name}\n", sha256_file(&zip)?));
        names.push(name);
    }
    fs::write(stage.join("SHA256SUMS"), sums)?;
    // Partial publication must not leave checksums for the previous set.
    match fs::remove_file(out_dir.join("SHA256SUMS")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).context("invalidating published checksums");
        }
        _ => {}
    }
    for name in names {
        fs::rename(stage.join(&name), out_dir.join(&name))?;
    }
    fs::rename(stage.join("SHA256SUMS"), out_dir.join("SHA256SUMS"))?;
    fs::remove_dir_all(&stage)?;
    Ok(())
}

fn validate_files(dir: &Path, expected: &[&str]) -> Result<()> {
    ensure!(
        fs::symlink_metadata(dir)?.is_dir(),
        "bundle must be a directory, not a symlink"
    );
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file(),
            "nonregular bundle file: {:?}",
            entry.path()
        );
        actual.insert(entry.file_name());
    }
    let expected: BTreeSet<_> = expected.iter().map(std::ffi::OsString::from).collect();
    ensure!(
        actual == expected,
        "unexpected bundle files: {actual:?}; expected {expected:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "0.111.0-73925250";
    const FILES: &[&str] = &[
        "manifest.json",
        "spire-profiler.dll",
        "spire_profiler.gdextension",
        "libprofiler_core.macos.arm64.dylib",
        "libprofiler_core.macos.x86_64.dylib",
        "libprofiler_core.linux.x86_64.so",
        "libprofiler_core.windows.x86_64.dll",
    ];
    const LABELS: &[&str] = &[
        "universal",
        "macos-arm64",
        "macos-x86_64",
        "linux-x86_64",
        "windows-x86_64",
    ];

    fn fixture(shell: &Shell) -> Result<xshell::TempDir> {
        let temp = shell.create_temp_dir()?;
        let bundle = temp.path().join(bundle::MOD_ID);
        fs::create_dir(&bundle)?;
        for file in FILES {
            fs::write(bundle.join(file), file)?;
        }
        fs::write(
            bundle.join("manifest.json"),
            format!(r#"{{"version":"{VERSION}"}}"#),
        )?;
        Ok(temp)
    }

    #[test]
    fn packages_exact_variants_and_checksums_without_stale_entries() -> Result<()> {
        let shell = Shell::new()?;
        let temp = fixture(&shell)?;
        shell.change_dir(temp.path());
        let bundle = temp.path().join(bundle::MOD_ID);
        let out = temp.path().join("dist");
        fs::create_dir(&out)?;
        let archive_name = format!("spire-profiler-{VERSION}-universal.zip");
        let stale = out.join(&archive_name);
        fs::write(temp.path().join("obsolete"), "old release")?;
        cmd!(shell, "zip --quiet {stale} obsolete").run()?;
        fs::create_dir(out.join(".stage"))?;
        fs::copy(&stale, out.join(".stage").join(archive_name))?;
        package(&shell, &bundle, &out, VERSION)?;
        let mut sums = String::new();
        for (index, label) in LABELS.iter().enumerate() {
            let name = format!("spire-profiler-{VERSION}-{label}.zip");
            let zip = out.join(&name);
            let listing = cmd!(shell, "unzip -Z -1 {zip}").read()?;
            let expected: Vec<_> = FILES
                .iter()
                .enumerate()
                .filter(|(i, _)| index == 0 || *i < 3 || *i == index + 2)
                .map(|(_, file)| format!("spire-profiler/{file}"))
                .collect();
            assert_eq!(listing.lines().collect::<Vec<_>>(), expected);
            let extracted = temp.path().join(label);
            cmd!(shell, "unzip -q {zip} -d {extracted}").run()?;
            for entry in expected {
                assert_eq!(
                    fs::read(extracted.join(&entry))?,
                    fs::read(temp.path().join(entry))?
                );
            }
            sums.push_str(&format!("{}  {name}\n", sha256_file(&zip)?));
        }
        assert_eq!(fs::read_to_string(out.join("SHA256SUMS"))?, sums);
        assert_eq!(fs::read_dir(&out)?.count(), 6);
        Ok(())
    }

    #[test]
    fn rejects_invalid_bundle_files_and_stamp() -> Result<()> {
        let shell = Shell::new()?;
        let temp = fixture(&shell)?;
        let bundle = temp.path().join(bundle::MOD_ID);
        let out = temp.path().join("dist");
        assert!(package(&shell, &bundle, &out, "wrong-version").is_err());
        let file = bundle.join(FILES[3]);
        fs::remove_file(&file)?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        fs::create_dir(&file)?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        fs::remove_dir(&file)?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("spire-profiler.dll", &file)?;
            assert!(package(&shell, &bundle, &out, VERSION).is_err());
            fs::remove_file(&file)?;
        }
        fs::write(&file, FILES[3])?;
        fs::write(bundle.join("extra"), "unexpected")?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        fs::remove_file(bundle.join("extra"))?;
        validate_files(&bundle, FILES)?;
        #[cfg(unix)]
        {
            let linked = temp.path().join("linked");
            std::os::unix::fs::symlink(&bundle, &linked)?;
            assert!(validate_files(&linked, FILES).is_err());
        }
        Ok(())
    }

    #[test]
    fn failed_staging_and_publication_leave_no_stale_checksums() -> Result<()> {
        let shell = Shell::new()?;
        let temp = fixture(&shell)?;
        let bundle = temp.path().join(bundle::MOD_ID);
        let out = temp.path().join("dist");
        fs::create_dir(&out)?;
        let stage = out.join(".stage");
        fs::write(&stage, "not a directory")?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        fs::remove_file(&stage)?;
        let universal = out.join(format!("spire-profiler-{VERSION}-universal.zip"));
        fs::write(&universal, "previous release")?;
        let sums = out.join("SHA256SUMS");
        fs::create_dir(&sums)?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        assert_eq!(fs::read_to_string(&universal)?, "previous release");
        fs::remove_dir(&sums)?;
        fs::write(&sums, "previous checksums")?;
        fs::create_dir(out.join(format!("spire-profiler-{VERSION}-macos-arm64.zip")))?;
        assert!(package(&shell, &bundle, &out, VERSION).is_err());
        cmd!(shell, "unzip -tqq {universal}").run()?;
        assert!(!sums.exists());
        Ok(())
    }
}
