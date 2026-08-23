//! Generation of the C# host sources from the committed template and the
//! attribution catalog. The template's C# semantics are never edited here;
//! only data (the catalog, the native lib selector) is injected.

use std::path::Path;

use crate::catalog;
use crate::cross::MATRIX;

pub const SHIM_TEMPLATE: &str = include_str!("../../shim/shim.cs.template");

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
    build_shim_cs_with(&catalog::RELICS, &catalog::POWERS)
}

pub fn build_shim_cs_with(relics: &[(&str, &str)], powers: &[(&str, &str)]) -> String {
    SHIM_TEMPLATE
        .replace("@RELIC_ENTRIES@", &catalog_literal(relics))
        .replace("@POWER_ENTRIES@", &catalog_literal(powers))
        .replace("@NATIVE_LIB_SELECTOR@", &native_lib_selector())
}

/// 8-space indent, "class|method",.
fn catalog_literal(entries: &[(&str, &str)]) -> String {
    let mut output = String::new();
    for (class_name, method_name) in entries {
        output.push_str("        \"");
        output.push_str(class_name);
        output.push('|');
        output.push_str(method_name);
        output.push_str("\",\n");
    }
    output
}

pub fn build_csproj(sts2_dll: &Path, harmony_dll: &Path, godot_sharp_dll: &Path) -> String {
    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net9.0</TargetFramework>
    <AssemblyName>SpireProfiler</AssemblyName>
    <RootNamespace>SpireProfiler</RootNamespace>
    <Nullable>disable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
    <OutputPath>bin/</OutputPath>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <AppendRuntimeIdentifierToOutputPath>false</AppendRuntimeIdentifierToOutputPath>
    <!-- The game scans every *.json under mods/ as a mod manifest. -->
    <GenerateDependencyFile>false</GenerateDependencyFile>
    <!-- Only the generated shim.cs is compiled. -->
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <EnableDefaultEmbeddedResourceItems>false</EnableDefaultEmbeddedResourceItems>
    <DebugType>none</DebugType>
    <DebugSymbols>false</DebugSymbols>
    <Deterministic>true</Deterministic>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="shim.cs" />
  </ItemGroup>
  <ItemGroup>
    <Reference Include="sts2"><HintPath>{}</HintPath><Private>false</Private></Reference>
    <Reference Include="0Harmony"><HintPath>{}</HintPath><Private>false</Private></Reference>
    <Reference Include="GodotSharp"><HintPath>{}</HintPath><Private>false</Private></Reference>
  </ItemGroup>
</Project>
"#,
        sts2_dll.display(),
        harmony_dll.display(),
        godot_sharp_dll.display(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn shim_substitution_replaces_all_placeholders() {
        let output = build_shim_cs_with(
            &[("RelicA", "AfterHook"), ("RelicB", "BeforeHook")],
            &[("PowerC", "OnTrigger")],
        );
        assert!(
            output
                .contains("        \"RelicA|AfterHook\",\n        \"RelicB|BeforeHook\",\n    };")
        );
        assert!(output.contains("        \"PowerC|OnTrigger\",\n    };"));
        assert!(!output.contains("@RELIC_ENTRIES@"));
        assert!(!output.contains("@POWER_ENTRIES@"));
        assert!(!output.contains("@NATIVE_LIB_SELECTOR@"));
        assert!(
            output.contains("PlatformNotSupportedException"),
            "the selector must fail loudly on platforms the bundle does not ship"
        );
    }

    #[test]
    fn csproj_matches_the_exact_format() {
        let output = build_csproj(
            &PathBuf::from("/STS2/sts2.dll"),
            &PathBuf::from("/STS2/0Harmony.dll"),
            &PathBuf::from("/STS2/GodotSharp.dll"),
        );
        let expected = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net9.0</TargetFramework>
    <AssemblyName>SpireProfiler</AssemblyName>
    <RootNamespace>SpireProfiler</RootNamespace>
    <Nullable>disable</Nullable>
    <ImplicitUsings>disable</ImplicitUsings>
    <OutputPath>bin/</OutputPath>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <AppendRuntimeIdentifierToOutputPath>false</AppendRuntimeIdentifierToOutputPath>
    <!-- The game scans every *.json under mods/ as a mod manifest. -->
    <GenerateDependencyFile>false</GenerateDependencyFile>
    <!-- Only the generated shim.cs is compiled. -->
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <EnableDefaultEmbeddedResourceItems>false</EnableDefaultEmbeddedResourceItems>
    <DebugType>none</DebugType>
    <DebugSymbols>false</DebugSymbols>
    <Deterministic>true</Deterministic>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="shim.cs" />
  </ItemGroup>
  <ItemGroup>
    <Reference Include="sts2"><HintPath>/STS2/sts2.dll</HintPath><Private>false</Private></Reference>
    <Reference Include="0Harmony"><HintPath>/STS2/0Harmony.dll</HintPath><Private>false</Private></Reference>
    <Reference Include="GodotSharp"><HintPath>/STS2/GodotSharp.dll</HintPath><Private>false</Private></Reference>
  </ItemGroup>
</Project>
"#;
        assert_eq!(output, expected);
    }
}
