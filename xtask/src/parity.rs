//! Differential fixtures execute the pinned original source and the production
//! replacement. Every invocation retains both inputs and outputs for inspection.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use xshell::{Shell, cmd};

use crate::{managed, render, sha256_file, workspace_root};

const BASELINE: &str = "c928c477852e75ffcc35f8cd16c7ed03caad7c7f";

pub struct Reference {
    directory: PathBuf,
    original: PathBuf,
}

impl Reference {
    #[allow(clippy::too_many_lines)] // One isolated source export owns all baseline executions and evidence.
    pub fn run(shell: &Shell, render: bool) -> Result<()> {
        let root = workspace_root();
        let scratch = root.join("tmp/parity-tests");
        fs::create_dir_all(&scratch)?;
        let mut serial = 0_u32;
        let directory = loop {
            let candidate = scratch.join(format!("run-{}-{serial}", std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    serial = serial
                        .checked_add(1)
                        .context("parity project serial exhausted")?;
                }
                Err(error) => return Err(error.into()),
            }
        };
        println!("parity-test: evidence retained at {}", directory.display());
        let archive = directory.join("baseline.tar");
        cmd!(
            shell,
            "git archive --format=tar --output={archive} {BASELINE}"
        )
        .run()?;
        let original = directory.join("original");
        fs::create_dir(&original)?;
        cmd!(
            shell,
            "tar --extract --file {archive} --directory {original}"
        )
        .run()?;
        let reference = Self {
            directory,
            original,
        };
        let support = root.join("test-support/parity");
        let fingerprints_path = support.join("fingerprints.json");
        let fingerprints: serde_json::Value =
            serde_json::from_slice(&fs::read(&fingerprints_path)?)?;
        ensure!(
            fingerprints["baseline"].as_str() == Some(BASELINE),
            "{} must pin baseline {BASELINE}",
            fingerprints_path.display()
        );
        let source = reference.original.join("profiler-core/src");
        let mut lib = fs::read_to_string(source.join("lib.rs"))?;
        for name in ["ui", "session"] {
            fs::copy(
                support.join(format!("{name}_oracle.rs")),
                source.join(format!("{name}_oracle.rs")),
            )?;
            lib.push_str(&format!("\n#[cfg(test)]\nmod {name}_oracle;\n"));
        }
        fs::write(source.join("lib.rs"), lib)?;
        let baseline_manifest = reference.original.join("Cargo.toml");
        let ui = reference.directory.join("ui_reference.json");
        let session = reference.directory.join("session_reference.json");
        {
            let _target = shell.push_env("CARGO_TARGET_DIR", scratch.join("target"));
            let _ui = shell.push_env("PARITY_OUTPUT", &ui);
            let _session = shell.push_env("PARITY_SESSION_OUTPUT", &session);
            cmd!(shell, "cargo test --manifest-path {baseline_manifest} --package profiler_core --lib oracle::export_reference --locked --offline").run()?;
        }
        for (name, output) in [
            ("ui_reference.json", &ui),
            ("session_reference.json", &session),
        ] {
            let expected = fingerprints["sha256"][name].as_str().with_context(|| {
                format!(
                    "missing SHA-256 for {name} in {}",
                    fingerprints_path.display()
                )
            })?;
            let actual = sha256_file(output)?;
            ensure!(
                actual == expected,
                "reference fingerprint mismatch for {name}: expected {expected}, generated {actual}; \
                 inspect {} and the original-source generator before manually changing {}; \
                 expected hashes are never updated automatically",
                output.display(),
                fingerprints_path.display()
            );
        }
        let driver = root.join("profiler-core/tests/support/attribution_parity.rs");
        let mut ledgers = Vec::new();
        for (label, code, features) in [
            (
                "baseline",
                reference.original.as_path(),
                &["--features", "baseline"][..],
            ),
            ("current", root, &[][..]),
        ] {
            let project = reference.directory.join(label);
            fs::create_dir(&project)?;
            let manifest = project.join("Cargo.toml");
            let core = serde_json::to_string(&code.join("profiler-core"))?;
            let driver_path = serde_json::to_string(&driver)?;
            fs::write(
                &manifest,
                format!(
                    "[package]\nname = \"attribution-parity-{label}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[features]\nbaseline = []\n[dependencies]\nprofiler_core = {{ path = {core}, features = [\"test-support\"] }}\nserde_json = \"=1.0.151\"\n[[bin]]\nname = \"parity\"\npath = {driver_path}\n"
                ),
            )?;
            let _target = shell.push_env("CARGO_TARGET_DIR", scratch.join("target"));
            cmd!(
                shell,
                "cargo generate-lockfile --manifest-path {manifest} --offline"
            )
            .run()?;
            let result = cmd!(
                shell,
                "cargo run --manifest-path {manifest} --locked --offline {features...}"
            )
            .output()?;
            fs::write(
                reference.directory.join(format!("{label}-ledger.jsonl")),
                &result.stdout,
            )?;
            fs::write(
                reference.directory.join(format!("{label}-ledger.stderr")),
                &result.stderr,
            )?;
            ledgers.push(result.stdout);
        }
        ensure!(
            ledgers[0] == ledgers[1],
            "original/current attribution ledgers differ; inspect retained JSONL outputs"
        );
        println!(
            "attribution parity: {} identical snapshots",
            ledgers[0]
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .count()
        );
        managed::run(shell, Some(&reference))?;
        if render {
            render::run(shell, &reference.directory)?;
        }
        println!("parity-test: PASS baseline={BASELINE} rendered={render}");
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // Extracted original methods, compile inputs, and their digests form one project transaction.
    pub fn prepare_managed(&self, project: &Path, csproj: &mut String) -> Result<()> {
        let support = workspace_root().join("test-support/parity");
        let destination = project.join("parity");
        fs::create_dir(&destination)?;
        let mut inputs = Vec::new();
        for name in [
            "UiParityFixtures.cs",
            "AttributionParityFixtures.cs",
            "SessionParityFixtures.cs",
            "TemporalParityFixtures.cs",
        ] {
            fs::copy(support.join(name), destination.join(name))?;
            inputs.push(format!("parity/{name}"));
        }
        fs::copy(support.join("Program.cs"), project.join("tests/Program.cs"))?;
        for name in ["ui_reference.json", "session_reference.json"] {
            fs::copy(self.directory.join(name), destination.join(name))?;
        }
        let mut imports = BTreeSet::new();
        let mut methods = String::new();
        for (file, start, end) in [
            (
                "DamageCapture.cs",
                "    internal static void Decompose(",
                "    internal static ResultKind Classify(",
            ),
            (
                "CommandCapture.cs",
                "    internal static void DecomposeBlock(",
                "    internal static void BuffPrefix(",
            ),
        ] {
            let original = fs::read_to_string(self.original.join("shim/attribution").join(file))?;
            imports.extend(
                original
                    .lines()
                    .filter(|line| line.starts_with("using "))
                    .map(str::to_owned),
            );
            let (before_end, _) = original
                .split_once(end)
                .with_context(|| format!("original {file} end marker missing"))?;
            let start_index = before_end
                .find(start)
                .with_context(|| format!("original {file} start marker missing"))?;
            methods.push_str(&before_end[start_index..]);
        }
        fs::write(
            destination.join("BaselineAttribution.g.cs"),
            format!(
                "{}\nnamespace SpireProfiler;\ninternal static class BaselineAttribution\n{{\n{methods}}}\n",
                imports.into_iter().collect::<Vec<_>>().join("\n")
            ),
        )?;
        inputs.push("parity/BaselineAttribution.g.cs".into());
        let original =
            fs::read_to_string(self.original.join("shim/attribution/SourceSnapshot.cs"))?;
        ensure!(
            original.matches("namespace SpireProfiler;").count() == 1,
            "original source namespace must be unique"
        );
        fs::write(
            destination.join("BaselineSources.g.cs"),
            original.replace(
                "namespace SpireProfiler;",
                "namespace SpireProfiler.OriginalSources;",
            ),
        )?;
        let temporal = fs::read_to_string(
            self.original
                .join("shim/attribution/TemporalPowerCapture.cs"),
        )?;
        let imports = temporal
            .lines()
            .filter(|line| line.starts_with("using "))
            .collect::<Vec<_>>()
            .join("\n");
        let (accumulator, _) = temporal
            .split_once("    internal static SourceSnapshot Combine(")
            .context("original accumulator end marker missing")?;
        let start = accumulator
            .find("    internal static void Accumulate(")
            .context("original accumulator start marker missing")?;
        fs::write(
            destination.join("BaselineTemporalAccumulator.g.cs"),
            format!(
                "{imports}\nnamespace SpireProfiler;\ninternal static partial class BaselineTemporalAccumulator\n{{\n{}}}\n",
                &accumulator[start..]
            ),
        )?;
        inputs.push("parity/BaselineTemporalAccumulator.g.cs".into());
        let (prefix, _) = temporal
            .split_once("    internal static void SetTurnAmount(")
            .context("original temporal end marker missing")?;
        let start = prefix
            .find("    internal static SourceSnapshot Combine(")
            .context("original temporal start marker missing")?;
        fs::write(
            destination.join("BaselineTemporal.g.cs"),
            format!(
                "{imports}\nnamespace SpireProfiler.OriginalSources;\ninternal static class BaselineTemporal\n{{\n{}}}\n",
                &prefix[start..]
            ),
        )?;
        inputs.extend([
            "parity/BaselineSources.g.cs".into(),
            "parity/BaselineTemporal.g.cs".into(),
        ]);
        let mut additions = String::new();
        for source in &inputs {
            additions.push_str(&format!("    <Compile Include=\"{source}\" />\n"));
        }
        *csproj = csproj.replacen("  </ItemGroup>", &format!("{additions}  </ItemGroup>"), 1);
        let mut digests = String::new();
        for line in fs::read_to_string(project.join("source-digests.txt"))?.lines() {
            let (_, source) = line
                .split_once("  ")
                .context("managed source digest missing separator")?;
            digests.push_str(&format!(
                "{}  {source}\n",
                sha256_file(&project.join(source))?
            ));
        }
        for source in inputs {
            digests.push_str(&format!(
                "{}  {source}\n",
                sha256_file(&project.join(&source))?
            ));
        }
        fs::write(project.join("source-digests.txt"), digests)?;
        Ok(())
    }
}
