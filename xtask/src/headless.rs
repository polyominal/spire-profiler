//! `cargo xtask headless-test`: install the mod, boot the game headless
//! with the self-test flag, and gate on successful exit and boot markers. The C#
//! Log.Info markers land in the godot logs while the core's stderr
//! markers only appear in process output, so both are combined.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, bail};
use xshell::Shell;

use crate::{discover, install, managed, workspace_root};

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

// The managed gate pins this definition inventory to the verified game.
const CAPTURE_PRODUCERS: u64 = 1726;
const CAPTURE_TARGETS: u64 = 1776;
const CAPTURE_PATCHES: u64 = 3547;
// Run lifecycle and history UI patches are outside the capture installer.
const FIXED_OWNER_PATCHES: u64 = 10;
const MIN_PATCHES: u64 = CAPTURE_TARGETS + FIXED_OWNER_PATCHES;
const PATCH_COUNT_MARKER: &str = "[SpireProfiler] OWN PATCHES owner=dev.spireprofiler methods=";
const CAPTURE_MARKER: &str = "[SpireProfiler] CAPTURE VERIFIED owner=dev.spireprofiler ";

struct CaptureReport {
    targets: u64,
    patches: u64,
    producers: u64,
    damage_bridges: u64,
    temporal_bridges: u64,
}

impl CaptureReport {
    fn parse(line: &str) -> Option<Self> {
        let (_, tail) = line.split_once(CAPTURE_MARKER)?;
        let mut fields = tail.split_whitespace();
        let targets = fields.next()?.strip_prefix("targets=")?.parse().ok()?;
        let patches = fields.next()?.strip_prefix("patches=")?.parse().ok()?;
        let producers = fields.next()?.strip_prefix("producers=")?.parse().ok()?;
        let damage_bridges = fields
            .next()?
            .strip_prefix("damage_bridges=")?
            .parse()
            .ok()?;
        let temporal_bridges = fields
            .next()?
            .strip_prefix("temporal_bridges=")?
            .parse()
            .ok()?;
        Some(Self {
            targets,
            patches,
            producers,
            damage_bridges,
            temporal_bridges,
        })
    }

    fn check(output: &str, owned_methods: Option<u64>, failures: &mut Vec<String>) {
        let verified = output.lines().filter_map(Self::parse).any(|report| {
            report.targets == CAPTURE_TARGETS
                && report.patches == CAPTURE_PATCHES
                && report.producers == CAPTURE_PRODUCERS
                && report.damage_bridges == 4
                && report.temporal_bridges == 16
                && owned_methods.is_none_or(|owned| owned >= report.targets)
        });
        if verified {
            println!("[ok] owned capture targets and exact bridge inventory verified");
        } else {
            let failure = "owned capture verification marker missing or inconsistent";
            eprintln!("headless-test: ERROR: {failure}");
            failures.push(failure.to_owned());
        }
    }
}

pub fn headless_test(shell: &Shell) -> Result<()> {
    managed::run(shell)?;
    let game = install::install_mod(shell)?;

    let log_dir = game_log_dir(game.platform)?;
    let root = workspace_root();

    // A scratch dir keeps the self-test from polluting the real play data.
    let scratch_data_dir = root.join("tmp").join("headless-data");
    let _ = std::fs::remove_dir_all(&scratch_data_dir);

    // Bounds which godot log belongs to this run: a stale log must never
    // satisfy the verdict.
    let boot_started = SystemTime::now();
    let (game_out, boot_duration, exit_status) = run_game_captured(&game, &scratch_data_dir, root)?;
    let newest_log = newest_boot_log(&log_dir, boot_started);
    if newest_log.is_none() {
        eprintln!("headless-test: warning: no godot*.log written during this boot");
    }

    println!("--- headless-test verdict ---");
    println!("boot duration: {:.1} s", boot_duration.as_secs_f64());
    println!("game exit status: {exit_status}");
    if let Some((path, _)) = &newest_log {
        println!("game log: {}", path.display());
    }

    let log_text = newest_log.map(|(_, text)| text).unwrap_or_default();
    let combined = format!("{log_text}\n{game_out}");

    assemble_verdict(&combined, exit_status).report()
}

fn assemble_verdict(output: &str, exit_status: ExitStatus) -> Verdict {
    let mut failures = Vec::new();
    if !exit_status.success() {
        let failure = format!("game exited unsuccessfully: {exit_status}");
        eprintln!("headless-test: ERROR: {failure}");
        failures.push(failure);
    }
    let owned_methods = check_patch_count(output, &mut failures);
    CaptureReport::check(output, owned_methods, &mut failures);
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
            eprintln!("FAIL");
            bail!("headless-test: FAIL ({} gates failed)", self.failures.len())
        }
    }
}

/// Both log streams can contain the same marker; counts include this owner only.
fn check_patch_count(output: &str, failures: &mut Vec<String>) -> Option<u64> {
    let count = output
        .match_indices(PATCH_COUNT_MARKER)
        .filter_map(|(index, _)| {
            let digits: String = output[index + PATCH_COUNT_MARKER.len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            digits.parse::<u64>().ok()
        })
        .max();
    match count {
        Some(patch_count) if patch_count >= MIN_PATCHES => {
            println!("[ok] owned Harmony methods: {patch_count} (>= {MIN_PATCHES})");
        }
        Some(patch_count) => {
            eprintln!("headless-test: ERROR: patched methods: {patch_count} (< {MIN_PATCHES})");
            failures.push(format!("patched methods: {patch_count} (< {MIN_PATCHES})"));
        }
        None => {
            eprintln!("headless-test: ERROR: patch-count marker '{PATCH_COUNT_MARKER}N' not found");
            failures.push("patch-count marker not found".to_owned());
        }
    }
    count
}

/// The shim's load/attach markers and the registration line prove the
/// gdext classes registered; the draw markers prove parent and both child
/// dispatches fired, and `chart draw ok` proves clean parent CallErrors.
const GATE_MARKERS: [&str; 11] = [
    "[SpireProfiler] INFO: chart self-test (combat):",
    "[SpireProfiler] INFO: chart self-test (run):",
    "[SpireProfiler] INFO: combat 1 summary written",
    "[SpireProfiler] INFO: run 1 recorded (victory)",
    "[SpireProfiler] GDExtension load result: Ok",
    "[SpireProfiler] profiler panel attached",
    "[SpireProfiler] INFO: panel class registered",
    "[SpireProfiler] INFO: chart _draw active",
    "[SpireProfiler] INFO: chart body _draw active",
    "[SpireProfiler] INFO: chart overlay _draw active",
    "[SpireProfiler] INFO: chart draw ok",
];

fn check_gate_markers(output: &str, failures: &mut Vec<String>) {
    for marker in GATE_MARKERS {
        if output.contains(marker) {
            println!("[ok] {marker}");
        } else {
            eprintln!("headless-test: ERROR: missing marker: {marker}");
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
        eprintln!(
            "headless-test: ERROR: {} unexpected [SpireProfiler] error line(s):",
            unexpected.len()
        );
        for line in &unexpected {
            eprintln!("    {line}");
        }
        failures.push(format!(
            "{} unexpected [SpireProfiler] error line(s)",
            unexpected.len()
        ));
    }
}

/// Honors STS2_USER_DATA_DIR, then falls back to the game platform's
/// user-data dir.
fn game_log_dir(platform: discover::Platform) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("STS2_USER_DATA_DIR") {
        return Ok(PathBuf::from(dir).join("logs"));
    }
    Ok(user_data_dir(platform)?.join("logs"))
}

/// project.godot sets a custom user dir named SlayTheSpire2.
fn user_data_dir(platform: discover::Platform) -> Result<PathBuf> {
    match platform {
        discover::Platform::Macos => {
            let home = std::env::var_os("HOME").ok_or_else(no_user_data_home)?;
            Ok(PathBuf::from(home).join("Library/Application Support/SlayTheSpire2"))
        }
        // The game is the WSL2-mounted Windows install: ask Windows for
        // %APPDATA% through interop (which headless-test needs anyway to
        // spawn the exe).
        discover::Platform::Windows => windows_user_data_dir(),
        discover::Platform::Linux => {
            let home = std::env::var_os("HOME").ok_or_else(no_user_data_home)?;
            match std::env::var_os("XDG_DATA_HOME") {
                Some(xdg) => Ok(PathBuf::from(xdg).join("SlayTheSpire2")),
                None => Ok(PathBuf::from(home).join(".local/share/SlayTheSpire2")),
            }
        }
    }
}

fn windows_user_data_dir() -> Result<PathBuf> {
    // chcp 65001 puts cmd's stdout in UTF-8, so a non-ASCII profile name
    // survives the codepage crossing into from_utf8_lossy.
    let win_appdata = run_trimmed(
        "cmd.exe",
        &["/c", "chcp 65001 >nul & echo %APPDATA%"],
        "querying %APPDATA% via WSL interop (enable [interop] in /etc/wsl.conf, or set \
         STS2_USER_DATA_DIR to skip the query)",
    )?;
    let wsl_appdata = run_trimmed(
        "wslpath",
        &["-u", &win_appdata],
        &format!("translating {win_appdata} with wslpath"),
    )?;
    Ok(PathBuf::from(wsl_appdata).join("SlayTheSpire2"))
}

/// Trimmed stdout of a command; a spawn failure, a non-zero exit, and
/// empty output are each an error carrying the caller's context.
fn run_trimmed(program: &str, args: &[&str], context: &str) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("{context}: {e}"))?;
    if !output.status.success() {
        // stderr usually names the real failure (e.g. wslpath's "No such
        // file or directory"); status alone does not.
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        };
        bail!("{context}: exited with{}{detail}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        bail!("{context}: produced no output");
    }
    Ok(stdout)
}

/// WSL interop inherits only the env vars WSLENV names; the /p flag
/// hands the Windows side the \\wsl$ translation of the Linux path.
fn share_path_with_windows(command: &mut Command, var: &str) {
    let existing = std::env::var("WSLENV").unwrap_or_default();
    command.env("WSLENV", merged_wslenv(&existing, var));
}

/// Any existing spec for the same var is replaced: stale flags (no /p, or
/// /l) would otherwise compete with the translation.
fn merged_wslenv(existing: &str, var: &str) -> String {
    let mut wslenv: Vec<String> = existing
        .split(':')
        .filter(|entry| !entry.is_empty() && entry.split('/').next() != Some(var))
        .map(str::to_owned)
        .collect();
    wslenv.push(format!("{var}/p"));
    wslenv.join(":")
}

fn no_user_data_home() -> anyhow::Error {
    anyhow::anyhow!("the home directory is not available and STS2_USER_DATA_DIR is unset")
}

/// Tees combined output to stderr while capturing it for the verdict.
fn run_game_captured(
    game: &discover::GamePaths,
    scratch_data_dir: &Path,
    root: &Path,
) -> Result<(String, Duration, ExitStatus)> {
    println!("booting the game headless (first boot may take 30-60s) ...");
    eprintln!(
        "headless-test: $ {} {}",
        game.game_exe.display(),
        GAME_ARGS.join(" ")
    );

    // std::process::Command (not xshell): piped stdout/stderr plus try_wait
    // polling for the watchdog.
    let mut command = Command::new(&game.game_exe);
    command
        .args(GAME_ARGS)
        .env("SPIRE_PROFILER_DATA_DIR", scratch_data_dir)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // A Windows game means a WSL2 host (Platform::detect rejects native
    // Windows). Without this bridge the shim would write self-test data
    // into the real play data dir.
    if game.platform == discover::Platform::Windows {
        share_path_with_windows(&mut command, "SPIRE_PROFILER_DATA_DIR");
    }

    let mut child = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawning {}: {e}", game.game_exe.display()))?;

    let streams: [Box<dyn std::io::Read + Send>; 2] = [
        Box::new(child.stdout.take().expect("stdout was piped")),
        Box::new(child.stderr.take().expect("stderr was piped")),
    ];
    let (receiver, pumps) = spawn_pumps(streams);

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
            eprintln!("headless-test: warning: output pump thread panicked");
        }
    }
    for line in receiver {
        eprint!("{line}");
        captured.push_str(&line);
    }

    Ok((captured, boot_start.elapsed(), status))
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

/// The log's mtime comes from the game's clock, `boot_started` from the
/// host's; under WSL2 the Windows game and the WSL clock skew after a
/// Windows sleep, and a strictly in-window filter would drop this run's
/// fresh log. The verdict still gates on the process output, so the
/// slack cannot let a stale log pass on its own.
const LOG_CLOCK_SLACK: Duration = Duration::from_secs(60);

/// The game rotates its previous log at boot, so the newest in-window
/// file is this run's.
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
        .filter(|(modified, _)| *modified + LOG_CLOCK_SLACK >= boot_started)
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

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;

    fn complete_boot_output() -> String {
        format!(
            "{PATCH_COUNT_MARKER}{MIN_PATCHES}\n{CAPTURE_MARKER}targets={CAPTURE_TARGETS} patches={CAPTURE_PATCHES} producers={CAPTURE_PRODUCERS} damage_bridges=4 temporal_bridges=16\n{}",
            GATE_MARKERS.join("\n")
        )
    }

    #[test]
    fn verdict_requires_successful_exit_with_complete_markers() {
        let output = complete_boot_output();
        let success = ExitStatus::from_raw(0);
        let failure = ExitStatus::from_raw(7 << 8);
        assert!(assemble_verdict(&output, success).report().is_ok());
        let verdict = assemble_verdict(&output, failure);
        assert_eq!(
            verdict.failures,
            [format!("game exited unsuccessfully: {failure}")]
        );
        assert!(verdict.report().is_err());

        let signal = ExitStatus::from_raw(libc::SIGTERM);
        let verdict = assemble_verdict(&output, signal);
        assert_eq!(
            verdict.failures,
            [format!("game exited unsuccessfully: {signal}")]
        );
        assert!(verdict.report().is_err());
    }

    #[test]
    fn successful_exit_still_requires_markers_and_no_profiler_errors() {
        let output = complete_boot_output();
        let success = ExitStatus::from_raw(0);
        let missing_count = output.replace(PATCH_COUNT_MARKER, "");
        let verdict = assemble_verdict(&missing_count, success);
        assert_eq!(verdict.failures, ["patch-count marker not found"]);
        assert!(verdict.report().is_err());

        let missing_marker = output.replace(GATE_MARKERS[0], "");
        let verdict = assemble_verdict(&missing_marker, success);
        assert_eq!(
            verdict.failures,
            [format!("missing marker: {}", GATE_MARKERS[0])]
        );
        assert!(verdict.report().is_err());

        let with_error = format!("{output}\n[SpireProfiler] ERROR: patch failed");
        let verdict = assemble_verdict(&with_error, success);
        assert_eq!(
            verdict.failures,
            ["1 unexpected [SpireProfiler] error line(s)"]
        );
        assert!(verdict.report().is_err());
    }

    #[test]
    fn patch_count_ignores_unrelated_matching_text() {
        let unrelated = "[OtherMod] OWN PATCHES owner=dev.spireprofiler methods=999999\n\
                         [SpireProfiler] OWN PATCHES owner=another.mod methods=999999";
        let mut failures = Vec::new();
        check_patch_count(unrelated, &mut failures);
        assert_eq!(failures, ["patch-count marker not found"]);

        let insufficient = MIN_PATCHES - 1;
        let output = format!("{unrelated}\n{PATCH_COUNT_MARKER}{insufficient}");
        let mut failures = Vec::new();
        check_patch_count(&output, &mut failures);
        assert_eq!(
            failures,
            [format!("patched methods: {insufficient} (< {MIN_PATCHES})")]
        );
    }

    #[test]
    fn patch_count_uses_maximum_matching_count_and_accepts_other_mods() {
        for (counts, passes) in [
            (vec![MIN_PATCHES - 1], false),
            (vec![MIN_PATCHES], true),
            (vec![MIN_PATCHES + 1], true),
            (vec![0, MIN_PATCHES, 0], true),
            (vec![MIN_PATCHES, MIN_PATCHES], true),
            (vec![0, MIN_PATCHES - 1, 0], false),
        ] {
            let output = counts
                .iter()
                .map(|count| format!("{PATCH_COUNT_MARKER}{count}"))
                .collect::<Vec<_>>()
                .join("\n");
            let mut failures = Vec::new();
            check_patch_count(&output, &mut failures);
            assert_eq!(failures.is_empty(), passes, "counts: {counts:?}");
        }

        let mut failures = Vec::new();
        check_patch_count(
            &format!("{PATCH_COUNT_MARKER}unknown\n{PATCH_COUNT_MARKER}18446744073709551616"),
            &mut failures,
        );
        assert_eq!(failures, ["patch-count marker not found"]);
    }

    #[test]
    fn capture_verdict_requires_owned_consistent_complete_bridges() {
        let valid = complete_boot_output();
        for corrupt in [
            valid.replace(
                CAPTURE_MARKER,
                "[OtherMod] CAPTURE VERIFIED owner=dev.spireprofiler ",
            ),
            valid.replace("damage_bridges=4", "damage_bridges=3"),
            valid.replace("temporal_bridges=16", "temporal_bridges=15"),
            valid.replace(&format!("patches={CAPTURE_PATCHES}"), "patches=1"),
            valid.replace(&format!("producers={CAPTURE_PRODUCERS}"), "producers=0"),
            valid.replace(
                &format!("targets={CAPTURE_TARGETS}"),
                "targets=18446744073709551616",
            ),
        ] {
            let mut failures = Vec::new();
            CaptureReport::check(&corrupt, Some(MIN_PATCHES), &mut failures);
            assert_eq!(
                failures,
                ["owned capture verification marker missing or inconsistent"]
            );
        }
        let mut failures = Vec::new();
        CaptureReport::check(&valid, Some(CAPTURE_TARGETS - 1), &mut failures);
        assert!(
            !failures.is_empty(),
            "capture targets must be among owned methods"
        );
        failures.clear();
        CaptureReport::check(&valid, Some(MIN_PATCHES), &mut failures);
        assert!(failures.is_empty());
    }

    #[test]
    fn wslenv_merge_appends_the_translation_flag() {
        assert_eq!(merged_wslenv("", "FOO"), "FOO/p");
        assert_eq!(merged_wslenv("BAR/l", "FOO"), "BAR/l:FOO/p");
    }

    #[test]
    fn wslenv_merge_replaces_stale_specs_of_the_same_var() {
        assert_eq!(merged_wslenv("FOO", "FOO"), "FOO/p");
        assert_eq!(merged_wslenv("FOO/l:BAR", "FOO"), "BAR:FOO/p");
        assert_eq!(merged_wslenv("FOO/l:FOO/p:BAR", "FOO"), "BAR:FOO/p");
        // A var whose name merely starts with the var's is untouched.
        assert_eq!(merged_wslenv("FOOBAR/l", "FOO"), "FOOBAR/l:FOO/p");
    }
}
