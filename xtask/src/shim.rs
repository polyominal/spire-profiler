//! Generate the C# host and compile the same attribution sources in the
//! shipped mod and managed fixture runner. Only the native library selector
//! is substituted into the host template.

use std::path::Path;

use anyhow::{Context, Result};

use crate::cross::MATRIX;
use crate::{sha256_file, workspace_root};

pub const SHIM_TEMPLATE: &str = include_str!("../../shim/shim.cs.template");

const ATTRIBUTION_SOURCES: [&str; 8] = [
    "attribution/SourceSnapshot.cs",
    "attribution/FlowCapture.cs",
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
        ATTRIBUTION_SOURCES
            .into_iter()
            .chain(fixtures.iter().copied())
    }
}

pub fn write_sources(destination: &Path, kind: ProjectKind) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let template = destination.join("shim.cs");
    write_if_changed(&template, &build_shim_cs())?;
    let mut digests = format!("{}  shim.cs\n", sha256_file(&template)?);
    for source in kind.sources() {
        let input = workspace_root().join("shim").join(source);
        let contents = std::fs::read_to_string(&input)
            .with_context(|| format!("reading managed source {}", input.display()))?;
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

/// The four bundle file names come from the build matrix; any other
/// platform throws instead of loading a mismatched library.
pub fn native_lib_selector() -> String {
    let windows = lib_for("windows", "x86_64");
    let linux = lib_for("linux", "x86_64");
    let mac_x64 = lib_for("macos", "x86_64");
    let mac_arm64 = lib_for("macos", "arm64");
    format!(
        "OperatingSystem.IsWindows() ? \"{windows}\" : \
         OperatingSystem.IsLinux() ? \"{linux}\" : \
         RuntimeInformation.ProcessArchitecture == Architecture.X64 \
         ? \"{mac_x64}\" : \
         OperatingSystem.IsMacOS() ? \"{mac_arm64}\" : \
         throw new PlatformNotSupportedException(\"spire-profiler ships no native library for \
         this platform\")"
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

pub fn build_shim_cs() -> String {
    SHIM_TEMPLATE.replace("@NATIVE_LIB_SELECTOR@", &native_lib_selector())
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
    <Compile Include="shim.cs" />
{sources}  </ItemGroup>
  <ItemGroup>
{references}  </ItemGroup>
</Project>
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_substitution_replaces_all_placeholders() {
        let output = build_shim_cs();
        for row in MATRIX {
            assert!(
                output.contains(row.bundle_name),
                "missing shipped library {}",
                row.bundle_name
            );
        }
        assert!(!output.contains("@NATIVE_LIB_SELECTOR@"));
        assert!(
            output.contains("PlatformNotSupportedException"),
            "the selector must fail loudly on platforms the bundle does not ship"
        );
    }
}
