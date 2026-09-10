//! One production source inventory feeds copying, compilation, digests, and
//! ABI checks. Managed tests append fixtures; only the native library selector
//! is generated from the platform matrix.

use std::path::Path;

use anyhow::{Context, Result};

use crate::cross::MATRIX;
use crate::{sha256_file, workspace_root};

const PRODUCTION_SOURCES: [&str; 21] = [
    "SpireProfilerMod.cs",
    "NativeLibrarySelector.g.cs",
    "native/ProfilerNative.cs",
    "native/NativeAttributionBackend.cs",
    "run/RunContext.cs",
    "run/RunPatches.cs",
    "ui/ProfilerPanels.cs",
    "ui/RunHistoryPatches.cs",
    "attribution/SourceSnapshot.cs",
    "attribution/AttributionBackend.cs",
    "attribution/CaptureRuntime.cs",
    "attribution/IdentityCapture.cs",
    "attribution/FlowCapture.cs",
    "attribution/GameAttributionBackend.cs",
    "attribution/CapturePatches.cs",
    "attribution/ProvenanceCapture.cs",
    "attribution/DamageCapture.cs",
    "attribution/TemporalPowerCapture.cs",
    "attribution/PlayCapture.cs",
    "attribution/CommandCapture.cs",
    "attribution/DoomCapture.cs",
];

#[derive(Clone, Copy)]
pub enum ProjectKind {
    Mod,
    Tests,
}

impl ProjectKind {
    fn sources(self) -> impl Iterator<Item = &'static str> {
        let fixtures: &[&str] = match self {
            Self::Mod => &[],
            Self::Tests => &["tests/Program.cs", "tests/Fixtures.cs"],
        };
        PRODUCTION_SOURCES
            .into_iter()
            .chain(fixtures.iter().copied())
    }

    pub fn read_sources(self) -> Result<Vec<(&'static str, String)>> {
        self.sources()
            .map(|source| {
                let contents = if source == "NativeLibrarySelector.g.cs" {
                    native_library_selector()
                } else {
                    let input = workspace_root().join("shim").join(source);
                    std::fs::read_to_string(&input)
                        .with_context(|| format!("reading managed source {}", input.display()))?
                };
                Ok((source, contents))
            })
            .collect()
    }
}

pub fn write_sources(destination: &Path, kind: ProjectKind) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let mut digests = String::new();
    for (source, contents) in kind.read_sources()? {
        let output = destination.join(source);
        std::fs::create_dir_all(output.parent().expect("managed sources have a parent"))?;
        write_if_changed(&output, &contents)?;
        digests.push_str(&format!("{}  {source}\n", sha256_file(&output)?));
    }
    write_if_changed(&destination.join("source-digests.txt"), &digests)
}

pub fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
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

pub fn build_csproj(
    sts2_dll: &Path,
    harmony_dll: &Path,
    godot_sharp_dll: &Path,
    kind: ProjectKind,
) -> String {
    let (assembly, test_properties, dependencies) = match kind {
        ProjectKind::Mod => ("SpireProfiler", "", "false"),
        ProjectKind::Tests => (
            "SpireProfiler.ManagedTests",
            "    <OutputType>Exe</OutputType>\n",
            "true",
        ),
    };
    let mut sources = String::new();
    for source in kind.sources() {
        sources.push_str(&format!("    <Compile Include=\"{source}\" />\n"));
    }
    let mut references = String::new();
    for (name, path) in [
        ("sts2", sts2_dll),
        ("0Harmony", harmony_dll),
        ("GodotSharp", godot_sharp_dll),
    ] {
        let path = path
            .to_string_lossy()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\"', "&quot;")
            .replace('\'', "&apos;");
        references.push_str(&format!(
            "    <Reference Include=\"{name}\"><HintPath>{path}</HintPath><Private>false</Private></Reference>\n"
        ));
    }
    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net9.0</TargetFramework>
    <EnableNETAnalyzers>true</EnableNETAnalyzers>
    <AnalysisLevel>9.0-recommended</AnalysisLevel>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
    <AssemblyName>{assembly}</AssemblyName>
    <RootNamespace>SpireProfiler</RootNamespace>
{test_properties}    <Nullable>disable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
    <OutputPath>bin/</OutputPath>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <AppendRuntimeIdentifierToOutputPath>false</AppendRuntimeIdentifierToOutputPath>
    <!-- The game scans every *.json under mods/ as a mod manifest. -->
    <GenerateDependencyFile>{dependencies}</GenerateDependencyFile>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <EnableDefaultEmbeddedResourceItems>false</EnableDefaultEmbeddedResourceItems>
    <DebugType>none</DebugType>
    <DebugSymbols>false</DebugSymbols>
    <Deterministic>true</Deterministic>
  </PropertyGroup>
  <ItemGroup>
{sources}  </ItemGroup>
  <ItemGroup>
{references}  </ItemGroup>
</Project>
"#,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs::{self, File, FileTimes};
    use std::time::{Duration, UNIX_EPOCH};

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
        assert!(
            output.contains("PlatformNotSupportedException"),
            "the selector must fail loudly on platforms the bundle does not ship"
        );
    }

    fn compile_inputs(kind: ProjectKind) -> Vec<String> {
        let csproj = build_csproj(
            Path::new("sts2.dll"),
            Path::new("0Harmony.dll"),
            Path::new("GodotSharp.dll"),
            kind,
        );
        assert!(csproj.contains("<EnableDefaultCompileItems>false</EnableDefaultCompileItems>"));
        csproj
            .lines()
            .filter_map(|line| {
                Some(
                    line.trim()
                        .strip_prefix("<Compile Include=\"")?
                        .strip_suffix("\" />")?
                        .to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn projects_include_all_production_sources_and_only_tests_add_fixtures() -> Result<()> {
        let shim_root = workspace_root().join("shim");
        let mut directories = vec![shim_root.clone()];
        let mut handwritten = BTreeSet::new();
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                if path.is_dir() {
                    directories.push(path);
                } else if path.extension().is_some_and(|extension| extension == "cs") {
                    handwritten.insert(
                        path.strip_prefix(&shim_root)?
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        for kind in [ProjectKind::Mod, ProjectKind::Tests] {
            let compiled = compile_inputs(kind);
            let mut expected: BTreeSet<_> = handwritten
                .iter()
                .filter(|source| {
                    matches!(kind, ProjectKind::Tests) || !source.starts_with("tests/")
                })
                .map(String::as_str)
                .collect();
            expected.insert("NativeLibrarySelector.g.cs");
            assert_eq!(
                compiled.iter().map(String::as_str).collect::<BTreeSet<_>>(),
                expected
            );
            assert_eq!(
                compiled.len(),
                expected.len(),
                "compile inputs must be unique"
            );
        }
        Ok(())
    }

    #[test]
    fn incremental_writes_copy_and_hash_compile_inputs_ignoring_obsolete_shim() -> Result<()> {
        let root = workspace_root();
        let scratch_root = root.join("tmp/xtask-shim-tests");
        fs::create_dir_all(&scratch_root)?;
        let mut serial = 0_u32;
        let scratch = loop {
            let candidate = scratch_root.join(format!("{}-{serial}", std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    serial = serial
                        .checked_add(1)
                        .context("test directory serial exhausted")?;
                }
                Err(error) => return Err(error.into()),
            }
        };
        let old_timestamp = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for (label, kind) in [("mod", ProjectKind::Mod), ("tests", ProjectKind::Tests)] {
            let project = scratch.join(label);
            fs::create_dir(&project)?;
            let obsolete = project.join("shim.cs");
            fs::write(&obsolete, "#error obsolete host\n")?;
            write_sources(&project, kind)?;
            let compiled = compile_inputs(kind);
            assert!(!compiled.iter().any(|source| source == "shim.cs"));
            let digests = fs::read_to_string(project.join("source-digests.txt"))?;
            let hashed: Vec<_> = digests
                .lines()
                .map(|line| {
                    line.split_once("  ")
                        .expect("digest lines separate hash and filename")
                })
                .collect();
            assert_eq!(
                hashed.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
                compiled
            );
            for (hash, source) in &hashed {
                let path = project.join(source);
                assert_eq!(*hash, sha256_file(&path)?);
                if *source != "NativeLibrarySelector.g.cs" {
                    assert_eq!(fs::read(&path)?, fs::read(root.join("shim").join(source))?);
                }
            }
            let outputs = compiled
                .iter()
                .map(String::as_str)
                .chain(["source-digests.txt"]);
            for source in outputs.clone() {
                File::options()
                    .write(true)
                    .open(project.join(source))?
                    .set_times(FileTimes::new().set_modified(old_timestamp))?;
            }
            write_sources(&project, kind)?;
            for source in outputs {
                assert_eq!(
                    fs::metadata(project.join(source))?.modified()?,
                    old_timestamp,
                    "unchanged input {source} must preserve its timestamp"
                );
            }
            assert!(
                obsolete.is_file(),
                "explicit compile inputs must tolerate stale output"
            );
        }
        fs::remove_dir_all(scratch)?;
        Ok(())
    }
}
