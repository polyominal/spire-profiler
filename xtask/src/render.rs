//! Render the actual managed panel and an independent replay of original drawing
//! commands in the same game process. Temporary installation is always restored.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use xshell::{Shell, cmd};

use crate::{build, discover, dotnet, sha256_file, shim, workspace_root};

#[allow(clippy::too_many_lines)] // The generated fixture assembly and installation rollback share one transaction.
pub fn run(shell: &Shell, reference: &Path) -> Result<()> {
    let root = workspace_root();
    let game = build::build(shell)?;
    ensure!(
        game.platform != discover::Platform::Windows,
        "graphical parity requires a native game process; the WSL game bridge is not supported"
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
    fs::write(&bootstrap, original.replace(needle, &format!(
        "{needle}\n            try {{ await UiRenderFixtures.RunAsync(Environment.GetEnvironmentVariable(\"SPIRE_PROFILER_RENDER_REFERENCE\"), Environment.GetEnvironmentVariable(\"SPIRE_PROFILER_RENDER_OUTPUT\")); }}\n            catch (Exception error) {{ Godot.GD.PrintErr(\"UI_RENDER_FAIL \" + error); }}\n            finally {{ (Godot.Engine.GetMainLoop() as Godot.SceneTree)?.Quit(); }}"
    )))?;
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
    let installed = game.mods_dir.join("spire-profiler");
    let backup = reference.join("installed-mod-backup");
    let existed = installed.exists();
    if existed {
        fs::create_dir(&backup)?;
        for entry in fs::read_dir(&installed)? {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_file(),
                "installed mod contains a non-file entry: {}",
                entry.path().display()
            );
            fs::copy(entry.path(), backup.join(entry.file_name()))?;
        }
    }
    let output = reference.join("rendered");
    fs::create_dir(&output)?;
    let result = (|| -> Result<()> {
        if existed {
            fs::remove_dir_all(&installed)?;
        }
        fs::create_dir(&installed)?;
        for entry in fs::read_dir(root.join("target/mods/spire-profiler"))? {
            let entry = entry?;
            fs::copy(entry.path(), installed.join(entry.file_name()))?;
        }
        fs::copy(
            project.join("bin/SpireProfiler.dll"),
            installed.join("spire-profiler.dll"),
        )?;
        let log = fs::File::create(output.join("game.log"))?;
        let mut child = Command::new(&game.game_exe)
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
                reference.join("ui_reference.json"),
            )
            .env("SPIRE_PROFILER_RENDER_OUTPUT", &output)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .context("starting the real game renderer")?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
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
            log.contains("UI_RENDER_PASS baseline=") && !log.contains("UI_RENDER_FAIL"),
            "graphical parity did not pass; inspect {}",
            output.display()
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("report.json"))?)?;
        ensure!(
            report["exact"] == true,
            "render report contains pixel differences"
        );
        println!(
            "rendered parity: exact RGBA match; evidence at {}",
            output.display()
        );
        Ok(())
    })();
    let restored = (|| -> Result<()> {
        if installed.exists() {
            fs::remove_dir_all(&installed)?;
        }
        if existed {
            fs::create_dir(&installed)?;
            for entry in fs::read_dir(&backup)? {
                let entry = entry?;
                let restored = installed.join(entry.file_name());
                fs::copy(entry.path(), &restored)?;
                ensure!(
                    sha256_file(&entry.path())? == sha256_file(&restored)?,
                    "restored mod hash differs: {}",
                    restored.display()
                );
            }
        }
        Ok(())
    })();
    restored.context("restoring the user's installed mod after graphical parity")?;
    println!("rendered parity: original installed mod restored and hashes verified");
    result
}
