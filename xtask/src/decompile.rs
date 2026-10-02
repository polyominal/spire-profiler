//! `cargo xtask decompile`: recover the game's Godot source via GDRE Tools —
//! locate the .pck, provision the pinned tool (download + SHA-256 verify),
//! run it headless, verify the output, drop a provenance record. Hosts are
//! macOS/Linux ([`discover::HostPlatform::detect`]); a WSL2 host finds the
//! Windows
//! install's .pck through the same layout detection as discovery.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use xshell::{Shell, cmd};

use crate::{discover, game_version, workspace_root};

/// Pinning (rather than "latest") is what makes the checksums meaningful.
const GDRE_VERSION: &str = "v2.5.0-beta.5";

const GDRE_MACOS_SHA256: &str = "01211b4dd82f874bb21dfc11483d19affad9cff9c1912eacf561972b750011e6";
const GDRE_LINUX_SHA256: &str = "6d2ae1ccf783a305b6b7891d946d723366a51da739fc755fa6d0d43b7f8eefc9";

fn gdre_tools_dir(root: &Path) -> PathBuf {
    root.join("tmp/gdre-tools")
}

pub(crate) fn default_output_dir(root: &Path) -> PathBuf {
    root.join("tmp/sts2-decompiled")
}

pub fn decompile(shell: &Shell, output_dir: Option<PathBuf>, yes: bool) -> Result<()> {
    let host = discover::HostPlatform::detect()?;
    let arch = discover::Arch::detect()?;
    let root = workspace_root();
    let output_dir = output_dir.unwrap_or_else(|| default_output_dir(root));

    let pck = locate_pck(host, arch).map_err(|e| {
        anyhow::anyhow!(
            "{e} (set STS2_GAME_DIR to the game root — {})",
            host.game_root_hint()
        )
    })?;
    // A relative STS2_GAME_DIR would feed a relative --recover arg to GDRE.
    let abs_pck = pck
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("resolving {}: {e}", pck.display()))?;
    println!("game PCK: {}", abs_pck.display());
    // A game the mod was not verified against is a footgun: refuse early.
    let game_version = game_version::check_pin_at(&release_info_for_pck(&abs_pck))?;

    // Provision BEFORE the overwrite prompt: the download can fail, and the
    // wipe must never destroy a previous decompilation over a failed rerun.
    let gdre = ensure_gdre_tools(shell, host, root)?;
    println!("GDRE Tools: {}", gdre.binary.display());

    // Overwrite semantics mirror the verified tool.
    if output_dir.exists() {
        if !yes
            && !prompt_yes_no(&format!(
                "{} exists; overwrite? [y/N] ",
                output_dir.display()
            ))?
        {
            println!("aborted.");
            return Ok(());
        }
        fs::remove_dir_all(&output_dir)?;
    }
    fs::create_dir_all(&output_dir)?;
    println!("this will take 1-2 minutes.");

    let abs_output = output_dir
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("resolving {}: {e}", output_dir.display()))?;
    run_gdre(root, &gdre.binary, &abs_pck, &abs_output)?;

    println!("verifying output...");
    let project_godot = abs_output.join("project.godot");
    if !project_godot.is_file() {
        bail!(
            "decompilation may have failed: project.godot not found at {}",
            project_godot.display()
        );
    }

    write_provenance(&abs_output, &abs_pck, &gdre, game_version)?;

    println!("decompilation complete: {}", abs_output.display());
    println!(
        "provenance: {}",
        abs_output.join(".provenance.json").display()
    );
    Ok(())
}

/// STS2_GAME_DIR first, else the Steam library list. Only roots a readable
/// libraryfolders.vdf enumerates are trusted (no default-root fallback).
fn locate_pck(host: discover::HostPlatform, arch: discover::Arch) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STS2_GAME_DIR") {
        let candidate = discover::pck_path_for(Path::new(&dir), host, arch);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(searched_paths_error(
            "the Slay the Spire 2 .pck was not found at any searched path",
            &[candidate],
        ));
    }

    let (vdf_paths, libraries) = discover::vdf_library_roots(host)?;
    let candidates: Vec<PathBuf> = libraries
        .iter()
        .map(|lib| discover::pck_path_for(&lib.join(discover::STEAM_GAME_REL), host, arch))
        .collect();
    // Reports like a missing VDF (the remedy is the same: install via Steam).
    if candidates.is_empty() {
        return Err(searched_paths_error(
            "no Steam libraryfolders.vdf found at any expected location",
            &vdf_paths,
        ));
    }
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err(searched_paths_error(
        "the Slay the Spire 2 .pck was not found at any searched path",
        &candidates,
    ))
}

fn searched_paths_error(headline: &str, paths: &[PathBuf]) -> anyhow::Error {
    let listing = paths
        .iter()
        .map(|path| format!("\n  - {}", path.display()))
        .collect::<String>();
    anyhow::anyhow!("{headline}:{listing}")
}

fn release_info_for_pck(pck: &Path) -> PathBuf {
    pck.with_file_name("release_info.json")
}

/// (os name, exe path within the extracted tools dir, asset SHA-256) per
/// host; the release zip's asset name is `GDRE_tools-{GDRE_VERSION}-{os}.zip`.
fn gdre_host(host: discover::HostPlatform) -> (&'static str, &'static str, &'static str) {
    match host {
        discover::HostPlatform::Macos => (
            "macos",
            "Godot RE Tools.app/Contents/MacOS/Godot RE Tools",
            GDRE_MACOS_SHA256,
        ),
        discover::HostPlatform::Linux => ("linux", "gdre_tools.x86_64", GDRE_LINUX_SHA256),
    }
}

#[cfg(unix)]
fn chmod_executable(exe: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(exe, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn chmod_executable(_exe: &Path) -> Result<()> {
    unreachable!("HostPlatform::detect rejects non-Unix hosts before any tool runs")
}

struct Gdre {
    binary: PathBuf,
    version: &'static str,
    host: &'static str,
}

struct GdrePin {
    version: &'static str,
    host: &'static str,
    executable: &'static str,
    checksum: &'static str,
}

impl GdrePin {
    fn directory(&self, root: &Path) -> PathBuf {
        gdre_tools_dir(root).join(format!("{}-{}", self.version, self.host))
    }

    fn cached(&self, root: &Path) -> Result<Option<Gdre>> {
        let directory = self.directory(root);
        let stamp = match fs::read_to_string(directory.join(".verified-sha256")) {
            Ok(stamp) => stamp,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("reading the GDRE extraction stamp"),
        };
        let binary = directory.join(self.executable);
        Ok(
            (stamp == self.checksum && binary.is_file()).then_some(Gdre {
                binary,
                version: self.version,
                host: self.host,
            }),
        )
    }
}

/// The cache key identifies the pin; the stamp attests to a complete extraction
/// of its verified archive. An interrupted extraction cannot satisfy resolution.
fn ensure_gdre_tools(shell: &Shell, host: discover::HostPlatform, root: &Path) -> Result<Gdre> {
    let (host, executable, checksum) = gdre_host(host);
    let pin = GdrePin {
        version: GDRE_VERSION,
        host,
        executable,
        checksum,
    };
    if let Some(tool) = pin.cached(root)? {
        prepare_executable(shell, &tool.binary)?;
        return Ok(tool);
    }
    crate::ensure_cli(shell, "curl", "--version", "the GDRE Tools download")?;
    crate::ensure_cli(shell, "unzip", "-v", "extraction")?;
    let tools_dir = pin.directory(root);
    let asset = format!("GDRE_tools-{}-{}.zip", pin.version, pin.host);
    let url = format!(
        "https://github.com/GDRETools/gdsdecomp/releases/download/{}/{asset}",
        pin.version
    );
    let cache = gdre_tools_dir(root).join("cache");
    fs::create_dir_all(&cache)?;
    let zip = cache.join(asset);
    if !zip.is_file() || crate::sha256_file(&zip)? != pin.checksum {
        download_and_verify(shell, &zip, &url, pin.checksum)?;
    }
    match fs::remove_dir_all(&tools_dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).context("removing incomplete GDRE extraction");
        }
        _ => {}
    }
    extract_zip(shell, &zip, &tools_dir)?;
    let binary = tools_dir.join(pin.executable);
    if !binary.is_file() {
        bail!("GDRE Tools executable not found at {}", binary.display());
    }
    prepare_executable(shell, &binary)?;
    fs::write(tools_dir.join(".verified-sha256"), pin.checksum)?;
    Ok(Gdre {
        binary,
        version: pin.version,
        host: pin.host,
    })
}

fn prepare_executable(shell: &Shell, binary: &Path) -> Result<()> {
    chmod_executable(binary)?;
    #[cfg(target_os = "macos")]
    remove_quarantine(shell, binary);
    #[cfg(not(target_os = "macos"))]
    let _ = shell;
    Ok(())
}

/// Verifies the pinned SHA-256 BEFORE extraction, so a truncated or
/// tampered download never reaches the tools dir.
fn download_and_verify(shell: &Shell, zip: &Path, url: &str, expected_sha256: &str) -> Result<()> {
    println!("downloading GDRE Tools from {url}");
    cmd!(
        shell,
        "curl --fail --location --retry 3 --output {zip} {url}"
    )
    .run()?;
    let actual = crate::sha256_file(zip)?;
    if actual != expected_sha256 {
        bail!(
            "checksum mismatch for {}: expected {expected_sha256}, got {actual}",
            zip.display()
        );
    }
    println!("checksum verified: {actual}");
    Ok(())
}

/// The checksum above is the security boundary: only a known-good archive
/// is extracted.
fn extract_zip(shell: &Shell, zip: &Path, dest: &Path) -> Result<()> {
    println!("extracting...");
    fs::create_dir_all(dest)?;
    cmd!(shell, "unzip -q -o {zip} -d {dest}").run()?;
    Ok(())
}

/// Best-effort; only acts when the attribute is present.
#[cfg(target_os = "macos")]
fn remove_quarantine(shell: &Shell, path: &Path) {
    let has_attr = cmd!(shell, "xattr -l {path}")
        .read()
        .is_ok_and(|out| out.contains("com.apple.quarantine"));
    if !has_attr {
        return;
    }
    println!("removing quarantine attribute...");
    if cmd!(shell, "xattr -d com.apple.quarantine {path}")
        .run()
        .is_err()
    {
        eprintln!("decompile: warning: failed to remove quarantine attribute (ignored)");
    }
}

/// std::process::Command (not xshell): the spawn needs the pre_exec hook.
fn run_gdre(root: &Path, gdre: &Path, pck: &Path, output: &Path) -> Result<()> {
    println!(
        "running: {} --headless --recover={} --output={}",
        gdre.display(),
        pck.display(),
        output.display()
    );

    let mut command = Command::new(gdre);
    command
        .arg("--headless")
        .arg(format!("--recover={}", pck.display()))
        .arg(format!("--output={}", output.display()))
        // Strip the loader-override vars so GDRE never resolves a library
        // from the toolchain dirs.
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env_remove("LD_PRELOAD")
        .current_dir(root);
    reset_child_signal_dispositions(&mut command);

    let status = command
        .status()
        .map_err(|e| anyhow::anyhow!("decompilation failed: {e}"))?;
    if !status.success() {
        bail!("decompilation failed: GDRE exited {status}");
    }
    Ok(())
}

/// Cargo leaves SA_SIGINFO set on SIGUSR1 across exec; GDRE's NativeAOT
/// runtime uses SIGUSR1 internally and crashes. Resetting every signal to
/// SIG_DFL makes the spawn equivalent to a plain shell launch.
#[cfg(unix)]
fn reset_child_signal_dispositions(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: sigaction/sigemptyset are async-signal-safe, the only kind
    // pre_exec permits between fork and exec.
    unsafe {
        command.pre_exec(|| {
            let mut default_action: libc::sigaction = std::mem::zeroed();
            libc::sigemptyset(&mut default_action.sa_mask);
            default_action.sa_sigaction = libc::SIG_DFL;
            default_action.sa_flags = 0;
            // 1..=31 is the full set of standard signals on both platforms.
            for sig in 1..=31 {
                libc::sigaction(sig, &default_action, std::ptr::null_mut());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn reset_child_signal_dispositions(_command: &mut Command) {}

fn write_provenance(output: &Path, pck: &Path, gdre: &Gdre, game_version: &str) -> Result<()> {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system clock before the epoch: {e}"))?
        .as_secs();
    let json = serde_json::json!({
        // Unix epoch seconds.
        "utc_timestamp": utc,
        "host_platform": gdre.host,
        "pck_path": pck,
        "gdre_version": gdre.version,
        "game_version": game_version,
        "gdre_export_log_present": output.join("gdre_export.log").is_file(),
    });
    let text = serde_json::to_string_pretty(&json)
        .map_err(|e| anyhow::anyhow!("serializing provenance: {e}"))?;
    let path = output.join(".provenance.json");
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn prompt_yes_no(prompt: &str) -> Result<bool> {
    use std::io::{BufRead, Write};

    print!("{prompt}");
    std::io::stdout()
        .flush()
        .map_err(|e| anyhow::anyhow!("flushing the prompt: {e}"))?;
    let mut answer = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the prompt: {e}"))?;
    if read == 0 {
        return Ok(false);
    }
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_tool_requires_its_own_pin_and_completed_extraction() -> Result<()> {
        let scratch_root = workspace_root().join("tmp/xtask-gdre-tests");
        fs::create_dir_all(&scratch_root)?;
        let scratch = tempfile::tempdir_in(scratch_root)?;
        let root = scratch.path();
        let old = GdrePin {
            version: "old",
            host: "linux",
            executable: "gdre",
            checksum: "old-hash",
        };
        let pin = GdrePin {
            version: "new",
            host: "linux",
            executable: "gdre",
            checksum: "new-hash",
        };
        let other_host = GdrePin {
            host: "macos",
            ..pin
        };
        for candidate in [&old, &pin] {
            fs::create_dir_all(candidate.directory(root))?;
            fs::write(
                candidate.directory(root).join(candidate.executable),
                "fixture executable",
            )?;
        }
        fs::write(old.directory(root).join(".verified-sha256"), old.checksum)?;
        assert!(old.cached(root)?.is_some());
        assert!(
            pin.cached(root)?.is_none(),
            "a binary alone does not complete extraction"
        );
        fs::write(pin.directory(root).join(".verified-sha256"), old.checksum)?;
        assert!(
            pin.cached(root)?.is_none(),
            "the extraction must match the archive pin"
        );
        fs::write(pin.directory(root).join(".verified-sha256"), pin.checksum)?;
        let tool = pin.cached(root)?.expect("complete pinned extraction");
        assert!(other_host.cached(root)?.is_none());
        write_provenance(root, &root.join("fixture.pck"), &tool, "verified-game")?;
        let provenance: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join(".provenance.json"))?)?;
        assert_eq!(provenance["gdre_version"], "new");
        assert_eq!(provenance["host_platform"], "linux");
        assert_eq!(provenance["game_version"], "verified-game");
        Ok(())
    }
}
