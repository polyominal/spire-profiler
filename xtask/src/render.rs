//! Render the actual panel and an independent original-source layout with the
//! approved defense scale, in a private copy of the game and its user data.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use xshell::{Shell, cmd};

use crate::{
    bundle, check_abi, cross, discover, dotnet, game_version, git, sha256_file, shim,
    workspace_root,
};

#[allow(clippy::too_many_lines)] // Fixture generation and process verification share invocation-local paths.
pub fn run(shell: &Shell, reference: &Path) -> Result<()> {
    let root = workspace_root();
    let game = discover::locate_game()?;
    ensure!(
        game.platform != discover::Platform::Windows,
        "graphical parity requires a native game process; the WSL game bridge is not supported"
    );
    game_version::check_pin(&game)?;
    check_abi::run()?;
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    };
    let host = cross::MATRIX
        .iter()
        .find(|row| row.os == std::env::consts::OS && row.arch == arch)
        .context("graphical parity requires a supported native host")?;
    let triple = host.dir_triple;
    cmd!(
        shell,
        "cargo build --release --locked --package profiler_core --target {triple}"
    )
    .run()?;
    let native = (
        host.bundle_name,
        root.join("target")
            .join(triple)
            .join("release")
            .join(host.cdylib),
    );
    let project = reference.join("render-mod");
    shim::write_sources(&project, shim::ProjectKind::Mod)?;
    fs::copy(
        root.join("test-support/parity/UiRenderFixtures.cs"),
        project.join("UiRenderFixtures.cs"),
    )?;
    fs::copy(
        root.join("test-support/parity/baseline_render.gd"),
        reference.join("baseline_render.gd"),
    )?;
    let bootstrap = project.join("SpireProfilerMod.cs");
    let original = fs::read_to_string(&bootstrap)?;
    let needle = "            await ProfilerPanels.AttachPanelAsync(modDirectory);";
    ensure!(
        original.matches(needle).count() == 1,
        "render fixture bootstrap injection point changed"
    );
    fs::write(&bootstrap, original.replace(needle, &format!(r#"{needle}
            try
            {{
                string expectedData = Environment.GetEnvironmentVariable("SPIRE_PROFILER_RENDER_USER_DATA");
                if (Path.GetFullPath(Godot.OS.GetUserDataDir()) != Path.GetFullPath(expectedData))
                    throw new InvalidOperationException("Renderer did not use its private user data");
                Godot.GD.Print("UI_RENDER_ISOLATED executable=" + Godot.OS.GetExecutablePath() + " user_data=" + Godot.OS.GetUserDataDir());
                await UiRenderFixtures.RunAsync(Environment.GetEnvironmentVariable("SPIRE_PROFILER_RENDER_REFERENCE"), Environment.GetEnvironmentVariable("SPIRE_PROFILER_RENDER_OUTPUT"));
            }}
            catch (Exception error) {{ Godot.GD.PrintErr("UI_RENDER_FAIL " + error); }}
            finally {{ (Godot.Engine.GetMainLoop() as Godot.SceneTree)?.Quit(); }}"#)))?;
    let csproj = project.join("SpireProfiler.RenderTests.csproj");
    fs::write(
        &csproj,
        shim::build_csproj(
            &game.sts2_dll,
            &game.harmony_dll,
            &game.godot_sharp_dll,
            shim::ProjectKind::Mod,
        )
        .replacen(
            "  </ItemGroup>",
            "    <Compile Include=\"UiRenderFixtures.cs\" />\n  </ItemGroup>",
            1,
        ),
    )?;
    fs::write(
        project.join("NuGet.Config"),
        "<configuration><packageSources><clear /></packageSources></configuration>\n",
    )?;
    let binary = dotnet::resolve_dotnet(shell)?;
    {
        let _directory = shell.push_dir(&project);
        let _sdk_root = shell.push_env(
            "DOTNET_ROOT",
            binary.parent().expect("dotnet binary has a parent"),
        );
        let _telemetry = shell.push_env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
        cmd!(
            shell,
            "{binary} build {csproj} --configuration Release --nologo --verbosity quiet"
        )
        .run()?;
    }
    let mut digests = fs::read_to_string(project.join("source-digests.txt"))?
        .lines()
        .filter(|line| !line.ends_with("  SpireProfilerMod.cs"))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    for (label, path) in [
        ("SpireProfilerMod.cs", bootstrap),
        ("UiRenderFixtures.cs", project.join("UiRenderFixtures.cs")),
        ("baseline_render.gd", reference.join("baseline_render.gd")),
        ("fixture-assembly", project.join("bin/SpireProfiler.dll")),
        ("host-reducer", native.1.clone()),
        ("sts2.dll", game.sts2_dll.clone()),
        ("0Harmony.dll", game.harmony_dll.clone()),
        ("GodotSharp.dll", game.godot_sharp_dll.clone()),
    ] {
        digests.push_str(&format!("{}  {label}\n", sha256_file(&path)?));
    }
    fs::write(project.join("source-digests.txt"), digests)?;
    let isolated = PrivateGame::copy(shell, &game.game_root, &game.game_exe, reference)?;
    let mod_dir = isolated
        .executable
        .parent()
        .expect("game executable has a parent")
        .join("mods")
        .join(bundle::MOD_ID);
    bundle::assemble_bundle(
        root,
        &project,
        &mod_dir,
        &[native],
        &git::resolve_commit(shell),
    )?;
    let output = reference.join("rendered");
    fs::create_dir(&output)?;
    let mut isolated = isolated;
    isolated.prepare_user_data(game.platform)?;
    let log = fs::File::create(output.join("game.log"))?;
    let mut child = Command::new(&isolated.executable)
        .args([
            "--force-steam",
            "off",
            "--windowed",
            "--resolution",
            "1280x720",
            "--audio-driver",
            "Dummy",
            "--rendering-method",
            "gl_compatibility",
            "--rendering-driver",
            "opengl3",
            "--quit-after",
            "1800",
        ])
        .env("SPIRE_PROFILER_DATA_DIR", reference.join("render-data"))
        .env(
            "SPIRE_PROFILER_RENDER_REFERENCE",
            reference.join("ui_approved.json"),
        )
        .env("SPIRE_PROFILER_RENDER_OUTPUT", &output)
        .env(
            "SPIRE_PROFILER_RENDER_USER_DATA",
            isolated
                .user_data
                .as_ref()
                .expect("fixture settings own a user-data directory"),
        )
        .current_dir(&isolated.root)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("starting the real game renderer")?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("waiting for the private game renderer");
            }
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "graphical parity exceeded 120 seconds; inspect {}",
                output.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    ensure!(
        status.success(),
        "graphical parity game process failed: {status}"
    );
    let log = fs::read_to_string(output.join("game.log"))?;
    ensure!(
        log.contains("UI_RENDER_PASS baseline=")
            && log.contains("UI_RENDER_ISOLATED executable=")
            && !log.contains("UI_RENDER_FAIL"),
        "graphical parity did not pass; inspect {}",
        output.display()
    );
    let report: serde_json::Value = serde_json::from_slice(&fs::read(output.join("report.json"))?)?;
    ensure!(
        report["exact"] == true,
        "render report contains pixel differences"
    );
    println!(
        "rendered parity: exact RGBA match to approved defense scale; evidence at {}",
        output.display()
    );
    Ok(())
}

struct PrivateGame {
    root: PathBuf,
    executable: PathBuf,
    user_data: Option<PathBuf>,
}

impl PrivateGame {
    fn copy(shell: &Shell, source: &Path, executable: &Path, reference: &Path) -> Result<Self> {
        let relative_executable = executable.strip_prefix(source)?;
        ensure!(
            !reference.starts_with(source),
            "render scratch must be outside the installed game"
        );
        let root = reference.join("render-game");
        fs::create_dir(&root).context("reserving the private game directory")?;
        let game = Self {
            executable: root.join("install").join(relative_executable),
            root,
            user_data: None,
        };
        let destination = game.root.join("install");
        match discover::HostPlatform::detect()? {
            discover::HostPlatform::Macos => {
                // APFS clones share asset blocks without sharing writable files.
                // Dereference links so copied launch paths cannot lead back to the install.
                if cmd!(shell, "cp -cRL {source} {destination}").run().is_err() {
                    if destination.exists() {
                        fs::remove_dir_all(&destination)?;
                    }
                    cmd!(shell, "cp -RL {source} {destination}").run()?;
                }
            }
            discover::HostPlatform::Linux => {
                cmd!(
                    shell,
                    "cp --reflink=auto --recursive --dereference -- {source} {destination}"
                )
                .run()?;
            }
        }
        let executable_dir = game
            .executable
            .parent()
            .context("game executable has no parent")?;
        for name in ["mods", "mods_STEAMTEST"] {
            let directory = executable_dir.join(name);
            if fs::symlink_metadata(&directory).is_ok() {
                fs::remove_dir_all(directory)?;
            }
        }
        Ok(game)
    }

    fn prepare_user_data(&mut self, platform: discover::Platform) -> Result<()> {
        let home = std::env::var_os("HOME").context("the home directory is unavailable")?;
        let data = match platform {
            discover::Platform::Macos => PathBuf::from(home).join("Library/Application Support"),
            discover::Platform::Linux => std::env::var_os("XDG_DATA_HOME")
                .filter(|value| !value.is_empty())
                .map_or_else(|| PathBuf::from(home).join(".local/share"), PathBuf::from),
            discover::Platform::Windows => {
                unreachable!("native graphical hosts are checked before copying")
            }
        };
        self.reserve_user_data(&data)
    }

    fn reserve_user_data(&mut self, data: &Path) -> Result<()> {
        let base = data.join("SpireProfilerParity");
        fs::create_dir_all(&base)?;
        let mut serial = 0_u32;
        let name = loop {
            let name = format!("run-{}-{serial}", std::process::id());
            let path = base.join(&name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    self.user_data = Some(path);
                    break name;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    serial = serial
                        .checked_add(1)
                        .context("render user-data serial exhausted")?;
                }
                Err(error) => return Err(error).context("reserving private game user data"),
            }
        };
        let settings = self
            .user_data
            .as_ref()
            .expect("successful reservation owns user data")
            .join("default/1");
        fs::create_dir_all(&settings)?;
        // The pinned game's settings schema avoids migrations of this fixture.
        fs::write(
            settings.join("settings.save"),
            "{\"schema_version\":8,\"language\":\"eng\",\"fullscreen\":false,\"mod_settings\":{\"mods_enabled\":true},\"seen_ea_disclaimer\":true}\n",
        )?;
        fs::write(
            self.executable
                .parent()
                .expect("game executable has a parent")
                .join("override.cfg"),
            format!(
                "[application]\nconfig/use_custom_user_dir=true\nconfig/custom_user_dir_name=\"SpireProfilerParity/{name}\"\n"
            ),
        )?;
        Ok(())
    }
}

impl Drop for PrivateGame {
    fn drop(&mut self) {
        // An interrupted process may leave private copies, never a changed install.
        for path in self.user_data.iter().chain(std::iter::once(&self.root)) {
            if let Err(error) = fs::remove_dir_all(path) {
                eprintln!(
                    "render: could not remove private test directory {}: {error}",
                    path.display()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_copies_and_abandoned_runs_cannot_change_the_installation() -> Result<()> {
        let shell = Shell::new()?;
        let scratch = shell.create_temp_dir()?;
        let installed = scratch.path().join("installed");
        fs::create_dir_all(installed.join("mods/spire-profiler"))?;
        fs::write(installed.join("game"), "executable")?;
        fs::write(scratch.path().join("asset"), "original asset")?;
        std::os::unix::fs::symlink(scratch.path().join("asset"), installed.join("asset"))?;
        fs::write(scratch.path().join("override.cfg"), "user override")?;
        std::os::unix::fs::symlink(
            scratch.path().join("override.cfg"),
            installed.join("override.cfg"),
        )?;
        fs::write(installed.join("mods/spire-profiler/mod.dll"), "user mod")?;
        let data = scratch.path().join("user-data");
        fs::create_dir_all(data.join("SlayTheSpire2/default/1"))?;
        fs::write(
            data.join("SlayTheSpire2/default/1/settings.save"),
            "user settings",
        )?;
        let first = scratch.path().join("first");
        let second = scratch.path().join("second");
        fs::create_dir(&first)?;
        fs::create_dir(&second)?;
        let (mut a, mut b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                PrivateGame::copy(&Shell::new()?, &installed, &installed.join("game"), &first)
            });
            let b = scope.spawn(|| {
                PrivateGame::copy(&Shell::new()?, &installed, &installed.join("game"), &second)
            });
            Ok::<_, anyhow::Error>((
                a.join().expect("first clone thread")?,
                b.join().expect("second clone thread")?,
            ))
        })?;
        a.reserve_user_data(&data)?;
        b.reserve_user_data(&data)?;
        assert_ne!(a.user_data, b.user_data);
        assert!(!a.root.join("install/mods").exists());
        fs::write(a.root.join("install/asset"), "fixture asset")?;
        fs::write(first.join("report.json"), "evidence")?;
        let abandoned = a.root.clone();
        // Model termination before destructors: the next run must not reuse it.
        std::mem::forget(a);
        assert!(PrivateGame::copy(&shell, &installed, &installed.join("game"), &first).is_err());
        drop(b);
        assert!(!second.join("render-game").exists());
        assert!(abandoned.exists());
        assert_eq!(fs::read_to_string(first.join("report.json"))?, "evidence");
        assert_eq!(
            fs::read_to_string(installed.join("asset"))?,
            "original asset"
        );
        assert_eq!(
            fs::read_to_string(installed.join("override.cfg"))?,
            "user override"
        );
        assert_eq!(
            fs::read_to_string(installed.join("mods/spire-profiler/mod.dll"))?,
            "user mod"
        );
        assert_eq!(
            fs::read_to_string(data.join("SlayTheSpire2/default/1/settings.save"))?,
            "user settings"
        );
        Ok(())
    }

    #[test]
    fn failed_copy_removes_only_its_owned_scratch() -> Result<()> {
        let shell = Shell::new()?;
        let scratch = shell.create_temp_dir()?;
        let missing = scratch.path().join("missing");
        let reference = scratch.path().join("reference");
        fs::create_dir(&reference)?;
        fs::write(reference.join("ui_reference.json"), "keep")?;
        assert!(PrivateGame::copy(&shell, &missing, &missing.join("game"), &reference).is_err());
        assert!(!reference.join("render-game").exists());
        assert_eq!(
            fs::read_to_string(reference.join("ui_reference.json"))?,
            "keep"
        );
        Ok(())
    }
}
