//! Build the host reducer, managed runtime, and deterministic integration fixtures
//! against the installed, version-checked game assemblies. Every invocation
//! reserves and retains its own project under tmp/managed-tests for inspection.

use std::io;

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

use crate::{discover, dotnet, game_version, sha256_file, shim, workspace_root};

#[allow(clippy::too_many_lines)] // One build/run transaction shares project paths and scoped environment guards.
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
    let mut serial = 0_u32;
    let project = loop {
        let candidate = projects.join(format!("run-{}-{serial}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                serial = serial
                    .checked_add(1)
                    .context("managed test project serial exhausted")?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reserving {}", candidate.display()));
            }
        }
    };
    println!("managed-test: project retained at {}", project.display());
    shim::write_sources(&project, shim::ProjectKind::Tests)?;
    let project_file = project.join("SpireProfiler.ManagedTests.csproj");
    std::fs::write(
        &project_file,
        shim::build_csproj(
            &game.sts2_dll,
            &game.harmony_dll,
            &game.godot_sharp_dll,
            shim::ProjectKind::Tests,
        ),
    )?;
    // The pinned SDK provides net9.0; no package feed belongs in this harness.
    std::fs::write(
        project.join("NuGet.Config"),
        "<configuration><packageSources><clear /></packageSources></configuration>\n",
    )?;
    let mut digests = std::fs::read_to_string(project.join("source-digests.txt"))?;
    for (label, path) in [
        ("sts2.dll", &game.sts2_dll),
        ("0Harmony.dll", &game.harmony_dll),
        ("GodotSharp.dll", &game.godot_sharp_dll),
        ("host-reducer", &native),
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
    cmd!(
        shell,
        "{binary} build {project_file} --configuration Release --nologo --verbosity quiet"
    )
    .run()?;
    let executable = project.join("bin/SpireProfiler.ManagedTests.dll");
    let game_assemblies = game
        .sts2_dll
        .parent()
        .expect("discovery returns an assembly file path");
    cmd!(
        shell,
        "{binary} {executable} {game_assemblies} {project} {native}"
    )
    .run()?;
    println!("managed-test: PASS");
    Ok(())
}
