//! `cargo xtask build`: assemble the cross-platform mod bundle under
//! target/mods/: manifest + C# host dll + multi-key .gdextension + one
//! native library per platform key.

use std::path::{Path, PathBuf};

use anyhow::Result;
use xshell::{Shell, cmd};

use crate::{bundle, check_abi, cross, discover, game_version, git, shim, workspace_root};

pub fn build(shell: &Shell) -> Result<()> {
    let root = workspace_root();
    // Cheap host rejection before the expensive cross matrix runs.
    discover::Platform::detect()?;

    check_abi::run()?;

    // One multi-target cargo-zigbuild invocation builds the whole matrix.
    let libs = cross::build_matrix(shell, root)?;

    let game = discover::locate_game()?;
    // Fail fast on a game version the mod was not verified against.
    game_version::check_pin(&game)?;

    let build_commit = git::resolve_commit(root);
    println!("build commit: {build_commit}");
    let gen_dir = build_host_project(shell, root, &game)?;
    let mod_dir = root.join("target/mods").join(bundle::MOD_ID);
    bundle::assemble_bundle(root, &gen_dir, &mod_dir, &libs, &build_commit)?;

    // A gap is a build failure, never silent.
    let missing = bundle::missing_gdextension_libraries(&mod_dir);
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "the distribution bundle is missing native libraries: {}",
            missing.join(", ")
        ));
    }

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
    Ok(())
}

fn build_host_project(shell: &Shell, root: &Path, game: &discover::GamePaths) -> Result<PathBuf> {
    let gen_dir = root.join("target/xtask-gen");
    refresh_gen_dir(&gen_dir)?;
    write_if_changed(&gen_dir.join("shim.cs"), &shim::build_shim_cs())?;
    write_if_changed(
        &gen_dir.join(CSPROJ_NAME),
        &shim::build_csproj(&game.sts2_dll, &game.harmony_dll, &game.godot_sharp_dll),
    )?;
    run_dotnet_build(shell, &gen_dir)?;
    Ok(gen_dir)
}

/// The refresh skips it so an unchanged csproj keeps its mtime; any other
/// project file is removed — dotnet build refuses a dir with two.
const CSPROJ_NAME: &str = "SpireProfiler.csproj";

fn refresh_gen_dir(gen_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(gen_dir)?;
    for entry in std::fs::read_dir(gen_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "csproj")
            && entry.file_name() != std::ffi::OsStr::new(CSPROJ_NAME)
        {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Keeps mtime, so MSBuild treats the build as up to date.
fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    match std::fs::read_to_string(path) {
        Ok(existing) if existing == content => Ok(()),
        _ => Ok(std::fs::write(path, content)?),
    }
}

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
    cmd!(shell, "{binary} build -c Release --nologo -v q").run()?;
    Ok(())
}
