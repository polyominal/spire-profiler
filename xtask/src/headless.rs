//! `cargo xtask headless-test`: install the mod, boot the game headless
//! with the self-test flag, and gate on markers in the boot output. The C#
//! Log.Info markers land in the godot logs while the core's eprintln
//! markers only appear on stderr, so both are combined.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, bail};
use xshell::Shell;

use crate::{discover, game_version, install, workspace_root};

const BOOT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Headless, Steam disabled, the self-test sequence, fixed quit-after.
const GAME_ARGS: [&str; 6] = [
    "--headless",
    "--force-steam",
    "off",
    "--spire-profiler-self-test",
    "--quit-after",
    "1800",
];

/// Below this a large chunk of the catalog silently failed. Static count is
/// 35 class-level + 11 orb + 158 catalog = 204; the runtime count also
/// depends on the pinned version's hook resolution, so the floor keeps a
/// small margin for drift while still catching a large failure.
const MIN_PATCHES: u64 = 199;

pub fn headless_test(shell: &Shell) -> Result<()> {
    install::install_mod(shell)?;
    let game = discover::locate_game()?;
    // Refuse to boot a game the mod was not verified against.
    game_version::check_pin(&game)?;

    let log_dir = home_log_dir()?;
    let root = workspace_root();

    // A scratch dir keeps the self-test from polluting the real play data.
    let scratch_data_dir = root.join("tmp").join("headless-data");
    let _ = std::fs::remove_dir_all(&scratch_data_dir);

    // Bounds which godot log belongs to this run: a stale log must never
    // satisfy the verdict.
    let boot_started = SystemTime::now();
    let (game_out, boot_duration, exit_code) =
        run_game_captured(&game.game_exe, &scratch_data_dir, root)?;
    let (log_path, log_text) = find_newest_log(&log_dir, boot_started);

    let combined = format!("{log_text}\n{game_out}");

    println!("--- headless-test verdict ---");
    println!("boot duration: {:.1} s", boot_duration.as_secs_f64());
    println!(
        "game exit code: {}",
        exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    if let Some(path) = &log_path {
        println!("game log: {}", path.display());
    }

    assemble_verdict(&combined).report()
}

fn assemble_verdict(output: &str) -> Verdict {
    let mut failures = Vec::new();
    check_patch_count(output, &mut failures);
    check_gate_markers(output, &mut failures);
    check_unexpected_errors(output, &mut failures);
    Verdict { failures }
}

struct Verdict {
    failures: Vec<String>,
}

impl Verdict {
    fn report(self) -> Result<()> {
        if self.failures.is_empty() {
            println!("PASS");
            Ok(())
        } else {
            println!("FAIL");
            bail!("headless-test: FAIL ({} gates failed)", self.failures.len())
        }
    }
}

/// The threshold catches a silently half-broken catalog; deduped by max.
fn check_patch_count(output: &str, failures: &mut Vec<String>) {
    match output
        .match_indices("patched methods: ")
        .filter_map(|(index, _)| {
            let digits: String = output[index + "patched methods: ".len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            digits.parse::<u64>().ok()
        })
        .max()
    {
        Some(patch_count) if patch_count >= MIN_PATCHES => {
            println!(
                "[ok] harmony patches applied; patched methods: {patch_count} (>= {MIN_PATCHES})"
            );
        }
        Some(patch_count) => {
            println!("[FAIL] patched methods: {patch_count} (< {MIN_PATCHES})");
            failures.push(format!("patched methods: {patch_count} (< {MIN_PATCHES})"));
        }
        None => {
            println!(
                "[FAIL] patch-count marker '[SpireProfiler] harmony patches applied; patched methods: N' not found"
            );
            failures.push("patch-count marker not found".to_owned());
        }
    }
}

/// The shim's load/attach markers and the registration line prove both
/// gdext classes registered; `chart draw ok` proves dispatch fired with
/// zero CallErrors.
const GATE_MARKERS: [&str; 9] = [
    "[SpireProfiler] chart self-test (combat):",
    "[SpireProfiler] chart self-test (run):",
    "[SpireProfiler] combat 1 summary written",
    "[SpireProfiler] run 1 recorded (victory)",
    "[SpireProfiler] GDExtension load result: Ok",
    "[SpireProfiler] profiler panel attached",
    "[SpireProfiler] panel class registered",
    "[SpireProfiler] chart _draw active",
    "[SpireProfiler] chart draw ok",
];

fn check_gate_markers(output: &str, failures: &mut Vec<String>) {
    for marker in GATE_MARKERS {
        if output.contains(marker) {
            println!("[ok] {marker}");
        } else {
            println!("[FAIL] missing marker: {marker}");
            failures.push(format!("missing marker: {marker}"));
        }
    }
}

fn check_unexpected_errors(output: &str, failures: &mut Vec<String>) {
    let unexpected: Vec<&str> = output
        .lines()
        .filter(|line| is_unexpected_error(line))
        .collect();
    if unexpected.is_empty() {
        println!("[ok] no unexpected [SpireProfiler] error lines");
    } else {
        println!(
            "[FAIL] {} unexpected [SpireProfiler] error line(s):",
            unexpected.len()
        );
        for line in &unexpected {
            println!("    {line}");
        }
        failures.push(format!(
            "{} unexpected [SpireProfiler] error line(s)",
            unexpected.len()
        ));
    }
}

/// Honors STS2_USER_DATA_DIR, then falls back to the per-OS user-data dir.
fn home_log_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STS2_USER_DATA_DIR") {
        return Ok(PathBuf::from(dir).join("logs"));
    }
    let platform = discover::Platform::detect()?;
    Ok(user_data_dir(platform)?.join("logs"))
}

/// project.godot sets a custom user dir named SlayTheSpire2.
fn user_data_dir(platform: discover::Platform) -> Result<PathBuf> {
    match platform {
        discover::Platform::Macos => {
            let home = std::env::var_os("HOME").ok_or_else(no_user_data_home)?;
            Ok(PathBuf::from(home).join("Library/Application Support/SlayTheSpire2"))
        }
        discover::Platform::Windows => {
            // Windows-only paths must not get the host separator from join.
            match std::env::var_os("APPDATA") {
                Some(appdata) => Ok(PathBuf::from(format!(
                    r"{}\SlayTheSpire2",
                    appdata.to_string_lossy()
                ))),
                None => {
                    let profile = std::env::var_os("HOME").ok_or_else(no_user_data_home)?;
                    Ok(PathBuf::from(format!(
                        r"{}\AppData\Roaming\SlayTheSpire2",
                        profile.to_string_lossy()
                    )))
                }
            }
        }
        discover::Platform::Linux => {
            let home = std::env::var_os("HOME").ok_or_else(no_user_data_home)?;
            match std::env::var_os("XDG_DATA_HOME") {
                Some(xdg) => Ok(PathBuf::from(xdg).join("SlayTheSpire2")),
                None => Ok(PathBuf::from(home).join(".local/share/SlayTheSpire2")),
            }
        }
    }
}

fn no_user_data_home() -> anyhow::Error {
    anyhow::anyhow!("the home directory is not available and STS2_USER_DATA_DIR is unset")
}

/// Tees combined output to stderr while capturing it for the verdict.
fn run_game_captured(
    game_exe: &Path,
    scratch_data_dir: &Path,
    root: &Path,
) -> Result<(String, Duration, Option<i32>)> {
    println!("booting the game headless (first boot may take 30-60s) ...");
    eprintln!("$ {} {}", game_exe.display(), GAME_ARGS.join(" "));

    // std::process::Command (not xshell): piped stdout/stderr plus try_wait
    // polling for the watchdog.
    let mut child = Command::new(game_exe)
        .args(GAME_ARGS)
        .env("SPIRE_PROFILER_DATA_DIR", scratch_data_dir)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawning {}: {e}", game_exe.display()))?;

    let streams: [Box<dyn std::io::Read + Send>; 2] = [
        Box::new(child.stdout.take().expect("stdout was piped")),
        Box::new(child.stderr.take().expect("stderr was piped")),
    ];
    let (receiver, pumps) = spawn_pumps(streams);

    // Print and capture output as it arrives.
    let boot_start = Instant::now();
    let mut captured = String::new();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if boot_start.elapsed() > BOOT_TIMEOUT {
                    // Best-effort: the game may have exited between the
                    // try_wait and the kill.
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!(
                        "game boot timed out after {} s; watchdog killed the process",
                        BOOT_TIMEOUT.as_secs()
                    );
                }
                for line in receiver.try_iter() {
                    eprint!("{line}");
                    captured.push_str(&line);
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => bail!("waiting for the game: {e}"),
        }
    };

    // The pipes hit EOF, so the pumps close the channel.
    for pump in pumps {
        // Best-effort: a pump panic could drop lines the verdict needs.
        if pump.join().is_err() {
            eprintln!("headless-test: warning: an output pump thread panicked");
        }
    }
    for line in receiver {
        eprint!("{line}");
        captured.push_str(&line);
    }

    Ok((captured, boot_start.elapsed(), status.code()))
}

/// One pump thread per stream; lines (not bytes) keep interleaving sane.
fn spawn_pumps(
    streams: [Box<dyn std::io::Read + Send>; 2],
) -> (mpsc::Receiver<String>, Vec<thread::JoinHandle<()>>) {
    let (sender, receiver) = mpsc::channel::<String>();
    let mut pumps = Vec::new();
    for stream in streams {
        let sender = sender.clone();
        pumps.push(thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line_buffer = Vec::new();
            loop {
                line_buffer.clear();
                match reader.read_until(b'\n', &mut line_buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender
                            .send(String::from_utf8_lossy(&line_buffer).into_owned())
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }));
    }
    drop(sender);
    (receiver, pumps)
}

/// The game rotates its previous log at boot, so the newest in-window file
/// is this run's; a stale log must never satisfy the patch-count marker.
fn find_newest_log(log_dir: &Path, boot_started: SystemTime) -> (Option<PathBuf>, String) {
    match newest_boot_log(log_dir, boot_started) {
        Some((path, text)) => (Some(path), text),
        None => {
            println!("game log: no godot*.log written during this boot");
            (None, String::new())
        }
    }
}

fn newest_boot_log(log_dir: &Path, boot_started: SystemTime) -> Option<(PathBuf, String)> {
    let entries = std::fs::read_dir(log_dir).ok()?;
    let mut candidates: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            entry.file_type().is_ok_and(|t| t.is_file())
                && name.starts_with("godot")
                && name.ends_with(".log")
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .filter(|(modified, _)| *modified >= boot_started)
        .collect();
    candidates.sort_by_key(|candidate| candidate.0);
    let (_, path) = candidates.pop()?;
    let text = std::fs::read_to_string(&path).ok()?;
    Some((path, text))
}

/// Tagged \[SpireProfiler\] and reads as an error.
fn is_unexpected_error(line: &str) -> bool {
    line.contains("[SpireProfiler]") && line.contains("ERROR")
}
