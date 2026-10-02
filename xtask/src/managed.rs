//! Build the host reducer, managed runtime, and deterministic integration fixtures
//! against the installed, version-checked game assemblies. Every invocation
//! reserves and retains its own project under tmp/managed-tests for inspection.

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

use crate::{discover, dotnet, game_version, sha256_file, shim, workspace_root};

#[allow(
    clippy::too_many_lines,
    reason = "One build/run transaction shares project paths and scoped environment guards."
)]
pub fn run(shell: &Shell) -> Result<()> {
    let game = discover::locate_game()?;
    game_version::check_pin(&game)?;
    let binary = dotnet::resolve_dotnet(shell)?;
    let root = workspace_root();
    cmd!(shell, "cargo build --package profiler_core --locked").run()?;
    let native = root.join("target/debug").join(format!(
        "{}profiler_core{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let native = native
        .canonicalize()
        .context("locating the host attribution reducer")?;
    let projects = root.join("tmp/managed-tests");
    std::fs::create_dir_all(&projects)?;
    let project = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&projects)
        .with_context(|| format!("reserving a managed test project in {}", projects.display()))?
        .keep();
    println!("managed-test: project retained at {}", project.display());
    let snapshot = shim::Project::source(shim::ProjectKind::Tests).snapshot(shell, &project)?;
    snapshot.build(shell, &game)?;
    let executable = project.join("bin/SpireProfiler.ManagedTests.dll");
    let mut digests = std::fs::read_to_string(project.join("source-digests.txt"))?;
    for (label, path) in [
        ("sts2.dll", &game.sts2_dll),
        ("0Harmony.dll", &game.harmony_dll),
        ("GodotSharp.dll", &game.godot_sharp_dll),
        ("host-reducer", &native),
        ("managed-tests", &executable),
    ] {
        digests.push_str(&format!("{}  {label}\n", sha256_file(path)?));
    }
    std::fs::write(project.join("source-digests.txt"), digests)?;

    let _directory = shell.push_dir(&project);
    let _telemetry = shell.push_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    let _logo = shell.push_env("DOTNET_NOLOGO", "1");
    let _first_run = shell.push_env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1");
    let _sdk_root = shell.push_env(
        "DOTNET_ROOT",
        binary
            .parent()
            .expect("the bootstrapped dotnet binary has a parent"),
    );
    let game_assemblies = game
        .sts2_dll
        .parent()
        .expect("discovery returns an assembly file path");
    let game_version = game_version::PIN;
    cmd!(
        shell,
        "{binary} {executable} {game_assemblies} {project} {native} {game_version}"
    )
    .run()?;
    println!("managed-test: PASS");
    Ok(())
}
