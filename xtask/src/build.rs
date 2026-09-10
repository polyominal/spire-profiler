//! `cargo xtask build`: assemble the cross-platform mod bundle under
//! target/mods/: manifest + C# host dll + multi-key .gdextension + one
//! native library per platform key.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::{bundle, check_abi, cross, discover, game_version, git, shim, workspace_root};

pub fn build(shell: &Shell) -> Result<discover::GamePaths> {
    let root = workspace_root();
    // Cheap host rejection before the expensive cross matrix runs.
    discover::Platform::detect()?;

    check_abi::run()?;

    let libs = cross::build_matrix(shell, root)?;

    let game = discover::locate_game()?;
    // Fail fast on a game version the mod was not verified against.
    game_version::check_pin(&game)?;

    let build_commit = git::resolve_commit(shell);
    println!("build commit: {build_commit}");
    let gen_dir = build_host_project(shell, root, &game)?;
    let mod_dir = root.join("target/mods").join(bundle::MOD_ID);
    bundle::assemble_bundle(root, &gen_dir, &mod_dir, &libs, &build_commit)?;

    println!("game root: {}", game.game_root.display());
    println!(
        "mod target: {}",
        game.mods_dir.join(bundle::MOD_ID).display()
    );
    let lib_names: Vec<&str> = libs.iter().map(|(name, _)| name.as_str()).collect();
    println!(
        "bundle: {} (native libraries: {})",
        mod_dir.display(),
        lib_names.join(", ")
    );
    Ok(game)
}

fn build_host_project(shell: &Shell, root: &Path, game: &discover::GamePaths) -> Result<PathBuf> {
    let gen_dir = root.join("target/xtask-gen");
    shim::write_sources(&gen_dir, shim::ProjectKind::Mod)?;
    shim::write_if_changed(
        &gen_dir.join(CSPROJ_NAME),
        &shim::build_csproj(
            &game.sts2_dll,
            &game.harmony_dll,
            &game.godot_sharp_dll,
            shim::ProjectKind::Mod,
        ),
    )?;
    run_dotnet_build(shell, &gen_dir)?;
    Ok(gen_dir)
}

const CSPROJ_NAME: &str = "SpireProfiler.csproj";

fn run_dotnet_build(shell: &Shell, gen_dir: &Path) -> Result<()> {
    let binary = crate::dotnet::resolve_dotnet(shell)?;
    let _dir = shell.push_dir(gen_dir);
    let _telemetry_optout = shell.push_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    let _nologo = shell.push_env("DOTNET_NOLOGO", "1");
    let _skip_first_run = shell.push_env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1");
    // Pin the relocated SDK's root so a stray DOTNET_ROOT cannot hijack it.
    let _root = shell.push_env(
        "DOTNET_ROOT",
        binary
            .parent()
            .expect("the bootstrapped binary always has a parent dir"),
    );
    cmd!(
        shell,
        "{binary} build {CSPROJ_NAME} --configuration Release --nologo --verbosity quiet"
    )
    .run()?;
    Ok(())
}
