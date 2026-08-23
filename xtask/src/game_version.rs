//! Version pinning: every command that touches the installed game fails
//! fast when a Steam update has changed the game under us. Bump PIN
//! deliberately (never automatically) after re-verifying the mod.

use std::path::Path;

use anyhow::{Result, bail};

use crate::discover::GamePaths;

pub const PIN: &str = "v0.111.0";

pub fn installed_version(game: &GamePaths) -> Result<String> {
    installed_version_from(&game.release_info)
}

pub fn check_pin(game: &GamePaths) -> Result<()> {
    check_version(&installed_version(game)?, &game.release_info)
}

pub(crate) fn check_pin_at(release_info: &Path) -> Result<()> {
    check_version(&installed_version_from(release_info)?, release_info)
}

/// Diagnosed as "bump the pin", never silently accepted.
fn check_version(installed: &str, release_info: &Path) -> Result<()> {
    if installed != PIN {
        bail!(
            "the installed game version {installed} does not match the pinned version {PIN} \
             (from {}); a Steam update changed the game — bump game_version::PIN deliberately \
             and re-verify the mod against the new game",
            release_info.display()
        );
    }
    Ok(())
}

/// Both failures name the release_info path; a bare error would not say
/// which file.
fn installed_version_from(release_info: &Path) -> Result<String> {
    let text = std::fs::read_to_string(release_info)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", release_info.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parsing {}: {e}", release_info.display()))?;
    let version = value
        .get("version")
        .and_then(|version| version.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("{} has no string \"version\" field", release_info.display())
        })?;
    Ok(version.to_owned())
}
