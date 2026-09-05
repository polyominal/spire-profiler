//! Game discovery: STS2_GAME_DIR override, else the Steam library search.
//! The game tree's layout matches the host platform (macOS, Linux), with
//! one supported exception: the Windows install a WSL2 host sees over
//! `/mnt/<drive>`, detected from its data dir — the managed assemblies and the
//! version stamp the build consumes are platform-neutral. Native Windows
//! hosts are rejected at [`Platform::detect`]. Every derived path is
//! existence-checked, so a mis-set override or a renamed layout is
//! diagnosed instead of guessed.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub struct GamePaths {
    /// The detected layout's platform; differs from the host's only in
    /// the WSL2 setup (Windows game, Linux host).
    pub platform: Platform,
    pub game_root: PathBuf,
    /// Created lazily: the game makes it on its first mods-enabled boot,
    /// otherwise install-mod's copy does.
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
            "linux" => Ok(Platform::Linux),
            "windows" => bail!("native Windows hosts are unsupported; run cargo xtask inside WSL2"),
            _ => bail!("unknown host OS (supported: macOS, Linux)"),
        }
    }

    pub fn game_root_hint(self) -> &'static str {
        match self {
            Platform::Macos => "the directory containing SlayTheSpire2.app",
            Platform::Windows => "the directory containing SlayTheSpire2.exe",
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
    let host = Platform::detect()?;
    let arch = Arch::detect()?;
    match locate_game_in(host, arch) {
        Ok(game) => Ok(game),
        Err(e) => Err(anyhow::anyhow!(
            "could not locate the Slay the Spire 2 installation: {e}. Set STS2_GAME_DIR to the \
             game root ({}).",
            host.game_root_hint()
        )),
    }
}

fn locate_game_in(host: Platform, arch: Arch) -> Result<GamePaths> {
    // The override is authoritative and checked verbatim.
    if let Some(dir) = std::env::var_os("STS2_GAME_DIR") {
        return resolve_from_root_for(&PathBuf::from(dir), host, arch);
    }
    let roots = steam_library_roots(host)?;
    search_libraries(&roots, host, arch)
}

pub(crate) const STEAM_GAME_REL: &str = "steamapps/common/Slay the Spire 2";

/// Every "path" entry of every libraryfolders.vdf, then the platform's
/// default root (a healthy manifest already lists it).
fn steam_library_roots(host: Platform) -> Result<Vec<PathBuf>> {
    let (_, mut roots) = vdf_library_roots(host)?;
    roots.push(default_library_root(host)?);
    Ok(roots)
}

/// No default root is appended: only what a readable manifest enumerates
/// is trusted. This is [`steam_library_roots`] minus the fallback, for
/// consumers that must not accept an unmanifested install.
pub(crate) fn vdf_library_roots(host: Platform) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut vdf_paths = steam_vdf_candidates(host)?;
    let mut roots: Vec<PathBuf> = vdf_paths
        .iter()
        .flat_map(|vdf| vdf_library_paths(vdf).unwrap_or_default())
        .collect();
    if host == Platform::Linux {
        // WSL2: each mounted drive may carry a Windows Steam install, whose
        // own manifest enumerates the Windows-side libraries. Only those
        // entries are Windows-absolute, so only they get the drvfs
        // translation; native manifests carry host paths already.
        let windows_vdfs = wsl_windows_vdf_candidates();
        for vdf in &windows_vdfs {
            if let Some(paths) = vdf_library_paths(vdf) {
                roots.extend(
                    paths
                        .into_iter()
                        .filter_map(|path| windows_path_to_wsl(&path.to_string_lossy())),
                );
            }
        }
        vdf_paths.extend(windows_vdfs);
    }
    Ok((vdf_paths, roots))
}

/// The library roots one manifest enumerates; a missing VDF just means
/// that location has no Steam install.
fn vdf_library_paths(vdf: &Path) -> Option<Vec<PathBuf>> {
    let text = std::fs::read_to_string(vdf).ok()?;
    Some(libraryfolders_paths(&text))
}

/// WSL2 mounts Windows drives at `/mnt/<letter>`; the default Steam
/// location under each is a candidate.
fn wsl_windows_vdf_candidates() -> Vec<PathBuf> {
    // Only WSL mounts Windows drives; a native Linux host has nothing to
    // probe under /mnt.
    let is_wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"));
    if !is_wsl {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir("/mnt") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.len() == 1 && name.as_bytes()[0].is_ascii_lowercase()
        })
        .map(|entry| {
            entry
                .path()
                .join("Program Files (x86)/Steam/steamapps/libraryfolders.vdf")
        })
        .collect()
}

/// A Windows absolute path from a Windows VDF ("D:\\SteamLibrary": the
/// VDF escapes backslashes, one per pair) translated to its drvfs mount.
/// A bare drive root ("D:\\") yields None: no Steam library lives there.
fn windows_path_to_wsl(value: &str) -> Option<PathBuf> {
    let drive = value.chars().next()?;
    if !drive.is_ascii_alphabetic() || value.as_bytes().get(1) != Some(&b':') {
        return None;
    }
    let rest = value[2..].replace(r"\\", "/");
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }
    Some(PathBuf::from(format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        rest
    )))
}

/// Relative to home on both supported hosts (macOS/Linux).
fn steam_vdf_candidates(host: Platform) -> Result<Vec<PathBuf>> {
    let relative: &[&str] = match host {
        Platform::Macos => &["Library/Application Support/Steam/steamapps/libraryfolders.vdf"],
        Platform::Linux => &[
            ".local/share/Steam/steamapps/libraryfolders.vdf",
            ".steam/steam/steamapps/libraryfolders.vdf",
        ],
        Platform::Windows => unreachable!("Platform::detect rejects native Windows hosts"),
    };
    let home = std::env::var_os("HOME").ok_or_else(no_home)?;
    Ok(relative
        .iter()
        .map(|rel| PathBuf::from(home.clone()).join(rel))
        .collect())
}

fn default_library_root(host: Platform) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(no_home)?;
    match host {
        Platform::Macos => Ok(PathBuf::from(home).join("Library/Application Support/Steam")),
        Platform::Linux => Ok(PathBuf::from(home).join(".local/share/Steam")),
        Platform::Windows => unreachable!("Platform::detect rejects native Windows hosts"),
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
fn search_libraries(roots: &[PathBuf], host: Platform, arch: Arch) -> Result<GamePaths> {
    let mut last_error = None;
    for root in roots {
        let game_root = root.join(STEAM_GAME_REL);
        match resolve_from_root_for(&game_root, host, arch) {
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

struct LayoutPaths {
    data_dir: PathBuf,
    mods_dir: PathBuf,
    game_exe: PathBuf,
    release_info: PathBuf,
    pck: PathBuf,
}

/// Windows/Linux follow the Godot export convention: the game root IS the
/// exe dir, and the two layouts differ only in names — Godot spells the
/// Linux platform `linuxbsd` in the data dir, the binary is
/// `SlayTheSpire2` (Windows adds `.exe`), and the pck is
/// `SlayTheSpire2.pck` on both.
fn layout_paths(game_root: &Path, platform: Platform, arch: Arch) -> LayoutPaths {
    match platform {
        Platform::Macos => {
            let macos_dir = game_root.join("SlayTheSpire2.app/Contents/MacOS");
            let resources = game_root.join("SlayTheSpire2.app/Contents/Resources");
            LayoutPaths {
                data_dir: resources.join(format!("data_sts2_macos_{}", arch.data_dir_suffix())),
                mods_dir: macos_dir.join("mods"),
                game_exe: macos_dir.join("Slay the Spire 2"),
                release_info: resources.join("release_info.json"),
                pck: resources.join("Slay the Spire 2.pck"),
            }
        }
        Platform::Windows | Platform::Linux => {
            let (data_os, exe_name) = match platform {
                Platform::Windows => ("windows", "SlayTheSpire2.exe"),
                Platform::Linux => ("linuxbsd", "SlayTheSpire2"),
                Platform::Macos => unreachable!("the macOS arm is above"),
            };
            LayoutPaths {
                data_dir: game_root.join(format!("data_sts2_{data_os}_{}", arch.data_dir_suffix())),
                mods_dir: game_root.join("mods"),
                game_exe: game_root.join(exe_name),
                release_info: game_root.join("release_info.json"),
                pck: game_root.join("SlayTheSpire2.pck"),
            }
        }
    }
}

/// The pck path under a game root, layout-detected like discovery proper,
/// without the per-file existence checks a full GamePaths resolve demands.
pub(crate) fn pck_path_for(game_root: &Path, host: Platform, arch: Arch) -> PathBuf {
    layout_paths(game_root, detect_layout(game_root, host, arch), arch).pck
}

/// The data dir is the layout's signature. The host layout wins when
/// present; a Linux host additionally probes for the Windows layout (the
/// WSL2 install over `/mnt/<drive>`, the one supported foreign layout).
/// Anything else validates as the host layout so the error names its
/// first missing piece. Detection is content-based, never gated on a WSL
/// check: a dual-boot box pointing STS2_GAME_DIR at a mounted Windows
/// install resolves the same way — build and install-mod work there, and
/// only headless-test needs the interop the box then lacks.
fn detect_layout(game_root: &Path, host: Platform, arch: Arch) -> Platform {
    let has_layout = |platform: Platform| layout_paths(game_root, platform, arch).data_dir.is_dir();
    if host == Platform::Linux && !has_layout(host) && has_layout(Platform::Windows) {
        return Platform::Windows;
    }
    host
}

/// Fixed order so the first missing piece is what the error names.
pub(crate) fn resolve_from_root_for(
    game_root: &Path,
    host: Platform,
    arch: Arch,
) -> Result<GamePaths> {
    let platform = detect_layout(game_root, host, arch);
    let layout = layout_paths(game_root, platform, arch);

    let sts2_dll = layout.data_dir.join("sts2.dll");
    let harmony_dll = layout.data_dir.join("0Harmony.dll");
    let godot_sharp_dll = layout.data_dir.join("GodotSharp.dll");

    // release_info.json is checked first (the version-pin source); the
    // game-exe guard runs last so install-mod never deletes into a tree
    // without a real game binary.
    if !layout.release_info.is_file() {
        bail!(
            "release_info.json not found at {}",
            layout.release_info.display()
        );
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
    if !layout.game_exe.is_file() {
        bail!("game executable not found at {}", layout.game_exe.display());
    }

    Ok(GamePaths {
        platform,
        game_root: game_root.to_path_buf(),
        mods_dir: layout.mods_dir,
        sts2_dll,
        harmony_dll,
        godot_sharp_dll,
        game_exe: layout.game_exe,
        release_info: layout.release_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wiped per call, so a crashed run cannot leak state; parallel tests
    /// need distinct labels.
    fn wiped_dir(label: &str) -> PathBuf {
        let dir = crate::workspace_root().join("tmp/wiped").join(label);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp/wiped is writable");
        dir
    }

    struct FakeTree {
        root: PathBuf,
    }

    impl FakeTree {
        fn new(name: &str) -> Self {
            Self {
                root: wiped_dir(&format!("xtask-discover-{name}")),
            }
        }

        fn touch(&self, rel: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().expect("every tree file has a parent"))
                .expect("tmp/wiped is writable");
            std::fs::write(&path, []).expect("tmp/wiped is writable");
        }
    }

    fn touch_windows_tree(tree: &FakeTree) {
        tree.touch("release_info.json");
        tree.touch("data_sts2_windows_x86_64/sts2.dll");
        tree.touch("data_sts2_windows_x86_64/0Harmony.dll");
        tree.touch("data_sts2_windows_x86_64/GodotSharp.dll");
        tree.touch("SlayTheSpire2.exe");
    }

    #[test]
    fn windows_layout_is_detected_from_a_linux_host() {
        let tree = FakeTree::new("windows-from-linux");
        touch_windows_tree(&tree);
        // No mods/ on purpose: a never-modded install must still resolve.
        let game = resolve_from_root_for(&tree.root, Platform::Linux, Arch::X86_64)
            .expect("a complete Windows tree resolves");
        assert_eq!(game.platform, Platform::Windows);
        assert_eq!(game.game_exe, tree.root.join("SlayTheSpire2.exe"));
        assert_eq!(game.mods_dir, tree.root.join("mods"));
        assert_eq!(
            pck_path_for(&tree.root, Platform::Linux, Arch::X86_64),
            tree.root.join("SlayTheSpire2.pck")
        );
    }

    #[test]
    fn linux_layout_is_detected_on_a_linux_host() {
        let tree = FakeTree::new("linux-from-linux");
        tree.touch("release_info.json");
        tree.touch("data_sts2_linuxbsd_x86_64/sts2.dll");
        tree.touch("data_sts2_linuxbsd_x86_64/0Harmony.dll");
        tree.touch("data_sts2_linuxbsd_x86_64/GodotSharp.dll");
        tree.touch("SlayTheSpire2");
        let game = resolve_from_root_for(&tree.root, Platform::Linux, Arch::X86_64)
            .expect("a complete Linux tree resolves");
        assert_eq!(game.platform, Platform::Linux);
    }

    #[test]
    fn an_unrelated_dir_reports_the_host_layouts_first_missing_piece() {
        let tree = FakeTree::new("unrelated");
        let error = resolve_from_root_for(&tree.root, Platform::Linux, Arch::X86_64)
            .expect_err("an empty dir resolves no layout");
        assert!(
            error.to_string().contains(&format!(
                "release_info.json not found at {}",
                tree.root.display()
            )),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_partial_foreign_tree_reports_its_own_missing_piece() {
        let tree = FakeTree::new("partial-windows");
        tree.touch("release_info.json");
        tree.touch("data_sts2_windows_x86_64/sts2.dll");
        tree.touch("data_sts2_windows_x86_64/0Harmony.dll");
        tree.touch("data_sts2_windows_x86_64/GodotSharp.dll");
        let error = resolve_from_root_for(&tree.root, Platform::Linux, Arch::X86_64)
            .expect_err("a Windows tree without the exe is incomplete");
        assert!(
            error.to_string().contains("SlayTheSpire2.exe"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn windows_vdf_paths_translate_to_drvfs_mounts() {
        assert_eq!(
            windows_path_to_wsl(r"D:\\SteamLibrary"),
            Some(PathBuf::from("/mnt/d/SteamLibrary"))
        );
        assert_eq!(
            windows_path_to_wsl(r"E:\\Games\\Steam Library"),
            Some(PathBuf::from("/mnt/e/Games/Steam Library"))
        );
    }

    #[test]
    fn bare_drive_roots_carry_no_library_path() {
        assert_eq!(windows_path_to_wsl(r"D:\\"), None);
        assert_eq!(windows_path_to_wsl("D:"), None);
    }

    #[test]
    fn non_windows_vdf_paths_stay_untranslated() {
        assert_eq!(windows_path_to_wsl("/home/tester/.local/share/Steam"), None);
        assert_eq!(windows_path_to_wsl("relative/path"), None);
    }

    #[test]
    fn a_windows_tree_is_not_a_game_for_a_macos_host() {
        let tree = FakeTree::new("windows-from-macos");
        touch_windows_tree(&tree);
        let error = resolve_from_root_for(&tree.root, Platform::Macos, Arch::Arm64)
            .expect_err("WSL2 aside, foreign layouts are unsupported");
        assert!(
            error.to_string().contains("SlayTheSpire2.app"),
            "unexpected error: {error}"
        );
    }

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
