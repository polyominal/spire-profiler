//! Game discovery: STS2_GAME_DIR override, else the Steam library search.
//! Every derived path is existence-checked, so a mis-set override or a
//! renamed layout is diagnosed instead of guessed.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub struct GamePaths {
    pub game_root: PathBuf,
    pub mods_dir: PathBuf,
    pub sts2_dll: PathBuf,
    pub harmony_dll: PathBuf,
    pub godot_sharp_dll: PathBuf,
    pub game_exe: PathBuf,
    pub release_info: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        match std::env::consts::OS {
            "macos" => Ok(Platform::Macos),
            "windows" => Ok(Platform::Windows),
            "linux" => Ok(Platform::Linux),
            _ => bail!("unknown host OS (supported: macOS, Windows, Linux)"),
        }
    }

    pub fn game_root_hint(self) -> &'static str {
        match self {
            Platform::Macos => "the directory containing SlayTheSpire2.app",
            Platform::Windows => "the directory containing Slay the Spire 2.exe",
            Platform::Linux => "the directory containing the Slay the Spire 2 executable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X86_64,
}

impl Arch {
    pub fn detect() -> Result<Self> {
        match std::env::consts::ARCH {
            "aarch64" => Ok(Arch::Arm64),
            "x86_64" => Ok(Arch::X86_64),
            _ => bail!("unknown host architecture (supported: aarch64, x86_64)"),
        }
    }

    fn data_dir_suffix(self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X86_64 => "x86_64",
        }
    }
}

pub fn locate_game() -> Result<GamePaths> {
    let platform = Platform::detect()?;
    let arch = Arch::detect()?;
    match locate_game_in(platform, arch) {
        Ok(game) => Ok(game),
        Err(e) => Err(anyhow::anyhow!(
            "could not locate the Slay the Spire 2 installation: {e}. Set STS2_GAME_DIR to the \
             game root ({}).",
            platform.game_root_hint()
        )),
    }
}

fn locate_game_in(platform: Platform, arch: Arch) -> Result<GamePaths> {
    // The override is authoritative and checked verbatim.
    if let Some(dir) = std::env::var_os("STS2_GAME_DIR") {
        return resolve_from_root_for(&PathBuf::from(dir), platform, arch);
    }
    let roots = steam_library_roots(platform)?;
    search_libraries(&roots, platform, arch)
}

const STEAM_GAME_REL: &str = "steamapps/common/Slay the Spire 2";

/// Every "path" entry of every libraryfolders.vdf, then the platform's
/// default root (a healthy manifest already lists it).
fn steam_library_roots(platform: Platform) -> Result<Vec<PathBuf>> {
    let (_, mut roots) = vdf_library_roots(platform)?;
    roots.push(default_library_root(platform)?);
    Ok(roots)
}

/// No default root is appended: only what a readable manifest enumerates is
/// trusted. This is `steam_library_roots` minus the fallback, for callers
/// (decompile) that must not accept an unmanifested install.
pub(crate) fn vdf_library_roots(platform: Platform) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let vdf_paths = steam_vdf_candidates(platform)?;
    let mut roots = Vec::new();
    for candidate in &vdf_paths {
        // A missing VDF just means that location has no Steam install.
        if let Ok(text) = std::fs::read_to_string(candidate) {
            roots.extend(libraryfolders_paths(&text));
        }
    }
    Ok((vdf_paths, roots))
}

/// Relative to home on macOS/Linux, to the install root on Windows.
fn steam_vdf_candidates(platform: Platform) -> Result<Vec<PathBuf>> {
    let relative: &[&str] = match platform {
        Platform::Macos => &["Library/Application Support/Steam/steamapps/libraryfolders.vdf"],
        Platform::Linux => &[
            ".local/share/Steam/steamapps/libraryfolders.vdf",
            ".steam/steam/steamapps/libraryfolders.vdf",
        ],
        Platform::Windows => &[r"Steam\steamapps\libraryfolders.vdf"],
    };
    match platform {
        Platform::Windows => {
            // Backslashes are explicit: Windows-only paths must not get the
            // host separator from join.
            let program_files = std::env::var_os("ProgramFiles(x86)")
                .unwrap_or_else(|| OsStr::new(r"C:\Program Files (x86)").to_os_string());
            Ok(relative
                .iter()
                .map(|rel| PathBuf::from(format!(r"{}\{}", program_files.to_string_lossy(), rel)))
                .collect())
        }
        _ => {
            let home = std::env::var_os("HOME").ok_or_else(no_home)?;
            Ok(relative
                .iter()
                .map(|rel| PathBuf::from(home.clone()).join(rel))
                .collect())
        }
    }
}

fn default_library_root(platform: Platform) -> Result<PathBuf> {
    match platform {
        Platform::Macos => Ok(PathBuf::from(std::env::var_os("HOME").ok_or_else(no_home)?)
            .join("Library/Application Support/Steam")),
        Platform::Linux => {
            Ok(PathBuf::from(std::env::var_os("HOME").ok_or_else(no_home)?)
                .join(".local/share/Steam"))
        }
        Platform::Windows => {
            let program_files = std::env::var_os("ProgramFiles(x86)")
                .unwrap_or_else(|| OsStr::new(r"C:\Program Files (x86)").to_os_string());
            Ok(PathBuf::from(format!(
                r"{}\Steam",
                program_files.to_string_lossy()
            )))
        }
    }
}

fn no_home() -> anyhow::Error {
    anyhow::anyhow!("the home directory is unavailable and no STS2_GAME_DIR override was provided")
}

/// A line scan, not a parser: Valve's format is hand-maintained and only
/// the "path" lines matter.
fn libraryfolders_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let value = line.strip_prefix("\"path\"")?;
            let value = value.trim_start().strip_prefix('"')?;
            let (value, _) = value.split_once('"')?;
            Some(PathBuf::from(value))
        })
        .collect()
}

/// The error names every root searched and the final root's first missing
/// piece.
fn search_libraries(roots: &[PathBuf], platform: Platform, arch: Arch) -> Result<GamePaths> {
    let mut last_error = None;
    for root in roots {
        let game_root = root.join(STEAM_GAME_REL);
        match resolve_from_root_for(&game_root, platform, arch) {
            Ok(game) => return Ok(game),
            Err(e) => last_error = Some(e),
        }
    }
    let searched = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(anyhow::anyhow!(
        "game not found in any Steam library (searched: {searched}); the final root failed \
         with: {}",
        last_error.expect("at least one library root is always searched")
    ))
}

/// Fixed order so the first missing piece is what the error names.
pub(crate) fn resolve_from_root_for(
    game_root: &Path,
    platform: Platform,
    arch: Arch,
) -> Result<GamePaths> {
    let (data_dir, mods_dir, game_exe, release_info) = match platform {
        Platform::Macos => {
            let macos_dir = game_root.join("SlayTheSpire2.app/Contents/MacOS");
            let resources = game_root.join("SlayTheSpire2.app/Contents/Resources");
            let data_dir = resources.join(format!("data_sts2_macos_{}", arch.data_dir_suffix()));
            (
                data_dir,
                macos_dir.join("mods"),
                macos_dir.join("Slay the Spire 2"),
                resources.join("release_info.json"),
            )
        }
        // Windows/Linux follow the Godot export convention: the game root IS
        // the exe dir. Godot spells the Linux platform name `linuxbsd` in the
        // data dir and the binary is `SlayTheSpire2`, not the guessed
        // spellings.
        Platform::Windows => (
            game_root.join(format!("data_sts2_windows_{}", arch.data_dir_suffix())),
            game_root.join("mods"),
            game_root.join("Slay the Spire 2.exe"),
            game_root.join("release_info.json"),
        ),
        Platform::Linux => (
            game_root.join(format!("data_sts2_linuxbsd_{}", arch.data_dir_suffix())),
            game_root.join("mods"),
            game_root.join("SlayTheSpire2"),
            game_root.join("release_info.json"),
        ),
    };

    let sts2_dll = data_dir.join("sts2.dll");
    let harmony_dll = data_dir.join("0Harmony.dll");
    let godot_sharp_dll = data_dir.join("GodotSharp.dll");

    // release_info.json is checked first (the version-pin source); the
    // game-exe guard runs last so install-mod never deletes into a tree
    // without a real game binary.
    if !release_info.is_file() {
        bail!("release_info.json not found at {}", release_info.display());
    }
    if !sts2_dll.is_file() {
        bail!("sts2.dll not found at {}", sts2_dll.display());
    }
    if !harmony_dll.is_file() {
        bail!("0Harmony.dll not found at {}", harmony_dll.display());
    }
    if !godot_sharp_dll.is_file() {
        bail!("GodotSharp.dll not found at {}", godot_sharp_dll.display());
    }
    if !mods_dir.is_dir() {
        bail!("mods directory not found at {}", mods_dir.display());
    }
    if !game_exe.is_file() {
        bail!("game executable not found at {}", game_exe.display());
    }

    Ok(GamePaths {
        game_root: game_root.to_path_buf(),
        mods_dir,
        sts2_dll,
        harmony_dll,
        godot_sharp_dll,
        game_exe,
        release_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraryfolders_paths_extract_every_path_value() {
        let vdf = "\"libraryfolders\"\r\n{\r\n\t\"0\"\r\n\t{\r\n\t\t\"path\"\t\t\"/home/tester/.local/share/Steam\"\r\n\t\t\"label\"\t\t\"\"\r\n\t}\r\n\t\"1\"\r\n\t{\r\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\r\n\t}\r\n}\r\n";
        assert_eq!(
            libraryfolders_paths(vdf),
            vec![
                PathBuf::from("/home/tester/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary"),
            ]
        );
    }
}
