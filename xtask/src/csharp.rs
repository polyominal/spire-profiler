//! Syntax queries use the compiler shipped in the pinned SDK, without NuGet.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use xshell::{Shell, cmd};

#[derive(Serialize)]
struct Request<'a> {
    mode: &'a str,
    sources: &'a [(&'a str, &'a str)],
}

#[derive(Deserialize)]
struct Response<T> {
    data: T,
    errors: Vec<String>,
}

pub(crate) fn parse<T: DeserializeOwned>(mode: &str, sources: &[(&str, &str)]) -> Result<T> {
    static TOOL: OnceLock<Result<(PathBuf, PathBuf), String>> = OnceLock::new();
    let (binary, assembly) = TOOL
        .get_or_init(|| {
            let build = || -> Result<_> {
                let shell = Shell::new()?;
                let binary = crate::dotnet::resolve_dotnet(&shell)?;
                let project = crate::workspace_root().join("xtask/syntax/Syntax.csproj");
                let artifacts = crate::workspace_root().join("target/syntax");
                std::fs::create_dir_all(&artifacts)?;
                // Nextest runs fixtures in separate processes sharing these outputs.
                let lock = std::fs::File::create(artifacts.join(".build.lock"))?;
                lock.lock()?;
                let output = artifacts.join("bin");
                cmd!(shell, "{binary} build {project} --configuration Release --artifacts-path {artifacts} --output {output} --nologo --verbosity quiet")
                    .env("DOTNET_ROOT", crate::dotnet::bootstrap_dir())
                    .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
                    .env("DOTNET_NOLOGO", "1")
                    .run()?;
                Ok((binary, output.join("Syntax.dll")))
            };
            build().map_err(|error| format!("building the C# syntax tool: {error:#}"))
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let input = serde_json::to_vec(&Request { mode, sources })?;
    let mut child = Command::new(binary)
        .arg(assembly)
        .env("DOTNET_ROOT", crate::dotnet::bootstrap_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting the C# syntax tool")?;
    child
        .stdin
        .take()
        .expect("syntax tool stdin was piped")
        .write_all(&input)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "C# syntax tool failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let response: Response<T> =
        serde_json::from_slice(&output.stdout).context("reading the C# syntax tool response")?;
    if !response.errors.is_empty() {
        bail!("{}", response.errors.join("\n"));
    }
    Ok(response.data)
}
