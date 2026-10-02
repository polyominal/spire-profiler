//! SDK projects own managed compile inputs. MSBuild evaluates that same list for
//! ABI checks and retained fixture snapshots. Only the native library selector
//! is generated from the platform matrix.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use xshell::{Cmd, Shell};

use crate::cross::MATRIX;
use crate::{discover, dotnet, sha256_file, workspace_root};

#[derive(Clone, Copy)]
pub enum ProjectKind {
    Mod,
    Tests,
}

impl ProjectKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Mod => "SpireProfiler.csproj",
            Self::Tests => "SpireProfiler.ManagedTests.csproj",
        }
    }
}

pub struct Project {
    kind: ProjectKind,
    directory: PathBuf,
    pub output: PathBuf,
}

impl Project {
    pub fn source(kind: ProjectKind) -> Self {
        Self {
            kind,
            directory: workspace_root().join("shim"),
            output: workspace_root().join("target/xtask-gen"),
        }
    }

    fn configuration_files(&self) -> [&str; 4] {
        [
            self.kind.file_name(),
            "Directory.Build.props",
            "NuGet.Config",
            ".editorconfig",
        ]
    }

    fn command<'a>(&self, shell: &'a Shell, binary: &Path) -> Cmd<'a> {
        shell
            .cmd(binary)
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
            .env(
                "DOTNET_ROOT",
                binary.parent().expect("the bootstrapped SDK has a parent"),
            )
            .env("SpireBuildDir", &self.output)
    }

    fn input_paths(&self, shell: &Shell, binary: &Path, resources: bool) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(&self.output)?;
        write_if_changed(
            &self.output.join("NativeLibrarySelector.g.cs"),
            &native_library_selector(),
        )?;
        let item_names: &[&str] = if resources {
            &["Compile", "EmbeddedResource"]
        } else {
            &["Compile"]
        };
        let json = self
            .command(shell, binary)
            .args(["msbuild", "-nologo", "-property:Configuration=Release"])
            .arg(format!("-getItem:{}", item_names.join(",")))
            .arg(self.directory.join(self.kind.file_name()))
            .read()?;
        let result: serde_json::Value = serde_json::from_str(&json)?;
        let items = item_names
            .iter()
            .map(|name| {
                result["Items"][name]
                    .as_array()
                    .with_context(|| format!("MSBuild must return its evaluated {name} items"))
            })
            .collect::<Result<Vec<_>>>()?;
        items
            .into_iter()
            .flatten()
            .map(|item| {
                item["FullPath"]
                    .as_str()
                    .map(PathBuf::from)
                    .context("MSBuild input items must have a FullPath")
            })
            .collect()
    }

    fn input_name<'a>(&self, input: &'a Path) -> Result<&'a Path> {
        input
            .strip_prefix(&self.directory)
            .or_else(|_| input.strip_prefix(&self.output))
            .with_context(|| format!("source outside project directories: {}", input.display()))
    }

    pub fn read_sources(&self, shell: &Shell) -> Result<Vec<(String, String)>> {
        let binary = dotnet::resolve_dotnet(shell)?;
        self.input_paths(shell, &binary, false)?
            .into_iter()
            .map(|path| {
                Ok((
                    self.input_name(&path)?.to_string_lossy().into_owned(),
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("reading managed source {}", path.display()))?,
                ))
            })
            .collect()
    }

    pub fn snapshot(&self, shell: &Shell, destination: &Path) -> Result<Self> {
        let binary = dotnet::resolve_dotnet(shell)?;
        let snapshot = Self {
            kind: self.kind,
            directory: destination.join("src"),
            output: destination.to_owned(),
        };
        std::fs::create_dir_all(&snapshot.directory)?;
        for input in self.input_paths(shell, &binary, true)? {
            let base = if input.starts_with(&self.directory) {
                &snapshot.directory
            } else {
                &snapshot.output
            };
            let output = base.join(self.input_name(&input)?);
            std::fs::create_dir_all(output.parent().expect("snapshot sources have a parent"))?;
            std::fs::copy(input, output)?;
        }
        for name in self.configuration_files() {
            std::fs::copy(self.directory.join(name), snapshot.directory.join(name))?;
        }
        Ok(snapshot)
    }

    pub fn build(&self, shell: &Shell, game: &discover::GamePaths) -> Result<()> {
        let binary = dotnet::resolve_dotnet(shell)?;
        let mut digests = String::new();
        for input in self.input_paths(shell, &binary, true)? {
            digests.push_str(&format!(
                "{}  {}\n",
                sha256_file(&input)?,
                self.input_name(&input)?.display()
            ));
        }
        for name in self.configuration_files() {
            digests.push_str(&format!(
                "{}  {name}\n",
                sha256_file(&self.directory.join(name))?
            ));
        }
        write_if_changed(&self.output.join("source-digests.txt"), &digests)?;
        self.command(shell, &binary)
            .arg("build")
            .arg(self.directory.join(self.kind.file_name()))
            .args([
                "--configuration",
                "Release",
                "--nologo",
                "--verbosity",
                "quiet",
            ])
            .env("SpireSts2Dll", &game.sts2_dll)
            .env("SpireHarmonyDll", &game.harmony_dll)
            .env("SpireGodotSharpDll", &game.godot_sharp_dll)
            .run()?;
        Ok(())
    }
}

fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    match std::fs::read_to_string(path) {
        Ok(existing) if existing == contents => Ok(()),
        _ => Ok(std::fs::write(path, contents)?),
    }
}

fn native_library_selector() -> String {
    let windows = lib_for("windows", "x86_64");
    let linux = lib_for("linux", "x86_64");
    let mac_x64 = lib_for("macos", "x86_64");
    let mac_arm64 = lib_for("macos", "arm64");
    format!(
        "// Generated from the xtask platform matrix.\n\
         using System;\n\
         using System.Runtime.InteropServices;\n\n\
         namespace SpireProfiler;\n\n\
         internal static class NativeLibrarySelector\n\
         {{\n    \
             internal static string FileName() => \
         OperatingSystem.IsWindows() ? \"{windows}\" : \
         OperatingSystem.IsLinux() ? \"{linux}\" : \
         RuntimeInformation.ProcessArchitecture == Architecture.X64 \
         ? \"{mac_x64}\" : \
         OperatingSystem.IsMacOS() ? \"{mac_arm64}\" : \
         throw new PlatformNotSupportedException(\"spire-profiler ships no native library for \
         this platform\");\n\
         }}\n"
    )
}

/// The selector hardcodes the row shape, so a dropped row must fail loudly.
fn lib_for(os: &str, arch: &str) -> &'static str {
    MATRIX
        .iter()
        .find(|row| row.os == os && row.arch == arch)
        .unwrap_or_else(|| panic!("the native matrix must contain a {os}.{arch} row"))
        .bundle_name
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn selector_covers_the_shipped_libraries() {
        let output = native_library_selector();
        for row in MATRIX {
            assert!(
                output.contains(row.bundle_name),
                "missing shipped library {}",
                row.bundle_name
            );
        }
        assert!(output.contains("PlatformNotSupportedException"));
    }

    #[test]
    fn msbuild_selects_new_sources_and_snapshots_exclude_stale_outputs() -> Result<()> {
        let shell = Shell::new()?;
        let root = workspace_root();
        let scratch_root = root.join("tmp/xtask-shim-tests");
        fs::create_dir_all(&scratch_root)?;
        let scratch = tempfile::tempdir_in(&scratch_root)?;
        let mut project = Project {
            kind: ProjectKind::Mod,
            directory: scratch.path().join("source & spaces"),
            output: scratch.path().join("build & spaces"),
        };
        fs::create_dir_all(&project.directory)?;
        for name in project
            .configuration_files()
            .into_iter()
            .chain([ProjectKind::Tests.file_name()])
        {
            fs::copy(root.join("shim").join(name), project.directory.join(name))?;
        }
        for name in [
            "new/Feature.cs",
            "tests/Fixture.cs",
            "obj/Stale.cs",
            "bin/Stale.cs",
        ] {
            let path = project.directory.join(name);
            fs::create_dir_all(path.parent().expect("fixture paths have parents"))?;
            fs::write(path, "// fixture source\n")?;
        }
        fs::write(
            project.directory.join("tests/producer-inventory.json"),
            "reviewed fixture resource\n",
        )?;
        fs::create_dir_all(&project.output)?;
        fs::write(
            project.output.join("shim.cs"),
            "#error obsolete copied host\n",
        )?;
        for (kind, count) in [(ProjectKind::Mod, 2), (ProjectKind::Tests, 3)] {
            project.kind = kind;
            let sources = project.read_sources(&shell)?;
            assert!(sources.iter().any(|(name, _)| name == "new/Feature.cs"));
            assert_eq!(
                sources.iter().any(|(name, _)| name == "tests/Fixture.cs"),
                matches!(kind, ProjectKind::Tests)
            );
            assert_eq!(sources.len(), count);
        }
        let snapshot = project.snapshot(&shell, &scratch.path().join("retained & spaces"))?;
        let expected = snapshot.read_sources(&shell)?;
        fs::write(
            project.directory.join("new/Feature.cs"),
            "#error changed original\n",
        )?;
        fs::write(
            project.directory.join("tests/producer-inventory.json"),
            "changed fixture resource\n",
        )?;
        assert_eq!(
            fs::read_to_string(snapshot.directory.join("tests/producer-inventory.json"))?,
            "reviewed fixture resource\n"
        );
        assert_eq!(snapshot.read_sources(&shell)?, expected);
        assert_ne!(project.read_sources(&shell)?, expected);
        scratch.close()?;
        Ok(())
    }
}
