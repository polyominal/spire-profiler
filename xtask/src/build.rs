//! `cargo xtask build`: assemble the cross-platform mod bundle under
//! target/mods/: manifest, managed host assembly, and one
//! native library per platform key.

use anyhow::Result;
use xshell::Shell;

use crate::{bundle, check_abi, cross, discover, game_version, git, shim, workspace_root};

pub fn build(shell: &Shell) -> Result<discover::GamePaths> {
    let root = workspace_root();
    // Cheap host rejection before the expensive cross matrix runs.
    discover::HostPlatform::detect()?;

    let game = discover::locate_game()?;
    // Fail fast on a game version the mod was not verified against.
    game_version::check_pin(&game)?;

    check_abi::run(shell)?;
    let libs = cross::build_matrix(shell, root)?;

    let build_commit = git::resolve_commit(shell);
    println!("build commit: {build_commit}");
    let project = shim::Project::source(shim::ProjectKind::Mod);
    project.build(shell, &game)?;
    let mod_dir = root.join("target/mods").join(bundle::MOD_ID);
    bundle::assemble_bundle(root, &project.output, &mod_dir, &libs, &build_commit)?;

    println!("game root: {}", game.game_root.display());
    println!(
        "mod target: {}",
        game.mods_dir.join(bundle::MOD_ID).display()
    );
    let lib_names: Vec<&str> = libs.iter().map(|(name, _)| *name).collect();
    println!(
        "bundle: {} (native libraries: {})",
        mod_dir.display(),
        lib_names.join(", ")
    );
    Ok(game)
}
