//! `cargo xtask install-mod`: build the bundle, then replace the game's
//! copy of the mod with it.

use std::path::Path;

use anyhow::Result;
use xshell::Shell;

use crate::{build, bundle, discover, workspace_root};

/// Wiped first so files removed from the bundle cannot linger.
pub fn install_mod(shell: &Shell) -> Result<()> {
    build::build(shell)?;

    let game = discover::locate_game()?;
    let source = workspace_root().join("target/mods").join(bundle::MOD_ID);
    let destination = game.mods_dir.join(bundle::MOD_ID);

    match std::fs::remove_dir_all(&destination) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(anyhow::anyhow!("removing {}: {e}", destination.display())),
    }
    copy_bundle(&source, &destination)?;
    println!("install-mod: installed to {}", destination.display());
    Ok(())
}

/// A subdirectory would mean the bundle contract changed; error rather
/// than recurse.
fn copy_bundle(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", destination.display()))?;
    for entry in std::fs::read_dir(source)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", source.display()))?
    {
        let entry = entry.map_err(|e| anyhow::anyhow!("reading {}: {e}", source.display()))?;
        let source_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", source_path.display()))?;
        if file_type.is_dir() {
            return Err(anyhow::anyhow!(
                "the bundle at {} contains the unexpected directory {}; the bundle is flat by \
                 construction",
                source.display(),
                source_path.display()
            ));
        }
        let destination_path = destination.join(entry.file_name());
        std::fs::copy(&source_path, &destination_path).map_err(|e| {
            anyhow::anyhow!(
                "copying {} -> {}: {e}",
                source_path.display(),
                destination_path.display()
            )
        })?;
    }
    Ok(())
}
