//! `cargo xtask decompile`: recover the game's Godot source via GDRE Tools —
//! locate the .pck, provision the pinned tool (download + SHA-256 verify),
//! run it headless, verify the output, drop a provenance record. Hosts are
//! macOS/Linux ([`discover::Platform::detect`]); a WSL2 host finds the
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
    let host = discover::Platform::detect()?;
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
    game_version::check_pin_at(&release_info_for_pck(&abs_pck))?;

    // Provision BEFORE the overwrite prompt: the download can fail, and the
    // wipe must never destroy a previous decompilation over a failed rerun.
    let gdre = ensure_gdre_tools(shell, host, root)?;
    println!("GDRE Tools: {}", gdre.display());

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
    run_gdre(root, &gdre, &abs_pck, &abs_output)?;

    println!("verifying output...");
    let project_godot = abs_output.join("project.godot");
    if !project_godot.is_file() {
        bail!(
            "decompilation may have failed: project.godot not found at {}",
            project_godot.display()
        );
    }

    write_provenance(&abs_output, &abs_pck, host)?;

    println!("decompilation complete: {}", abs_output.display());
    println!(
        "provenance: {}",
        abs_output.join(".provenance.json").display()
    );
    Ok(())
}

/// STS2_GAME_DIR first, else the Steam library list. Only roots a readable
/// libraryfolders.vdf enumerates are trusted (no default-root fallback).
fn locate_pck(host: discover::Platform, arch: discover::Arch) -> Result<PathBuf> {
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
fn gdre_host(host: discover::Platform) -> (&'static str, &'static str, &'static str) {
    match host {
        discover::Platform::Macos => (
            "macos",
            "Godot RE Tools.app/Contents/MacOS/Godot RE Tools",
            GDRE_MACOS_SHA256,
        ),
        discover::Platform::Linux => ("linux", "gdre_tools.x86_64", GDRE_LINUX_SHA256),
        discover::Platform::Windows => {
            unreachable!("Platform::detect rejects native Windows hosts")
        }
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
    unreachable!("Platform::detect rejects non-Unix hosts before any tool runs")
}

/// Idempotent: a rerun with the binary present skips the download.
fn ensure_gdre_tools(shell: &Shell, host: discover::Platform, root: &Path) -> Result<PathBuf> {
    let (os_name, exe_rel, checksum) = gdre_host(host);
    let tools_dir = gdre_tools_dir(root);
    let exe = tools_dir.join(exe_rel);
    if exe.is_file() {
        println!("GDRE Tools: present at {}", tools_dir.display());
    } else {
        crate::ensure_cli(shell, "unzip", "-v", "extraction")?;
        let asset = format!("GDRE_tools-{GDRE_VERSION}-{os_name}.zip");
        let url = format!(
            "https://github.com/GDRETools/gdsdecomp/releases/download/{GDRE_VERSION}/{asset}"
        );
        let cache = tools_dir.join("cache");
        fs::create_dir_all(&cache)?;
        let zip = cache.join("gdre_tools.zip");
        // Best-effort; a leftover zip re-downloads only when the exe is gone.
        let result = download_and_verify(shell, &zip, &url, checksum)
            .and_then(|()| extract_zip(shell, &zip, &tools_dir));
        let _ = fs::remove_dir_all(&cache);
        result?;
    }

    if !exe.is_file() {
        bail!("GDRE Tools executable not found at {}", exe.display());
    }
    // The zip may not carry the exec bit; 0o755 must be set before the run.
    chmod_executable(&exe)?;
    // The quarantine xattr blocks execution until stripped (best-effort).
    #[cfg(target_os = "macos")]
    remove_quarantine(shell, &exe);
    Ok(exe)
}

/// Verifies the pinned SHA-256 BEFORE extraction, so a truncated or
/// tampered download never reaches the tools dir.
fn download_and_verify(shell: &Shell, zip: &Path, url: &str, expected_sha256: &str) -> Result<()> {
    println!("downloading GDRE Tools from {url}");
    cmd!(shell, "curl -fL --retry 3 -o {zip} {url}").run()?;
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

fn write_provenance(output: &Path, pck: &Path, host: discover::Platform) -> Result<()> {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("system clock before the epoch: {e}"))?
        .as_secs();
    // The pin check already ran, so this is the verified version; recorded
    // so check-catalog can reject a tree decompiled from an older game.
    let version = game_version::installed_version_from(&release_info_for_pck(pck))?;
    let json = serde_json::json!({
        // Unix epoch seconds.
        "utc_timestamp": utc,
        "host_platform": gdre_host(host).0,
        "pck_path": pck,
        "gdre_version": GDRE_VERSION,
        "game_version": version,
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
