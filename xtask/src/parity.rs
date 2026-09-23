//! Differential fixtures retain original behavior and explicit policy corrections
//! as separate references. Every invocation retains its inputs and outputs.

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
            let _directory = shell.push_dir(&reference.original);
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
        // The unchanged oracle remains fingerprinted. A separate original-source
        // export supplies the independently specified defense-scale correction.
        let approved = reference.directory.join("approved-ui");
        fs::create_dir(&approved)?;
        cmd!(
            shell,
            "tar --extract --file {archive} --directory {approved}"
        )
        .run()?;
        let patch = reference.directory.join("defense_scale.patch");
        fs::copy(support.join("defense_scale.patch"), &patch)?;
        cmd!(
            shell,
            "git apply --unsafe-paths --directory={approved} {patch}"
        )
        .run()?;
        let approved_source = approved.join("profiler-core/src");
        fs::copy(
            support.join("ui_oracle.rs"),
            approved_source.join("ui_oracle.rs"),
        )?;
        let mut approved_lib = fs::read_to_string(approved_source.join("lib.rs"))?;
        approved_lib.push_str("\n#[cfg(test)]\nmod ui_oracle;\n");
        fs::write(approved_source.join("lib.rs"), approved_lib)?;
        let corrected_ui = reference.directory.join("ui_approved.json");
        {
            let _directory = shell.push_dir(&approved);
            let _target = shell.push_env("CARGO_TARGET_DIR", scratch.join("approved-target"));
            let _ui = shell.push_env("PARITY_OUTPUT", &corrected_ui);
            cmd!(shell, "cargo test --package profiler_core --lib ui_oracle::export_reference --locked --offline").run()?;
        }
        let mut corrected: serde_json::Value = serde_json::from_slice(&fs::read(&corrected_ui)?)?;
        corrected["approved_changes"] = serde_json::json!(["displayed-defense-scale"]);
        fs::write(&corrected_ui, serde_json::to_vec_pretty(&corrected)?)?;
        println!(
            "UI reference: original fingerprint verified; approved defense patch SHA-256 {}",
            sha256_file(&patch)?
        );
        let current = reference.directory.join("current");
        fs::create_dir(&current)?;
        let files = cmd!(
            shell,
            "git -C {root} ls-files --cached --others --exclude-standard -z -- Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml profiler-core xtask"
        )
        .read()?;
        for relative in files.split('\0').filter(|path| !path.is_empty()) {
            let source = root.join(relative);
            if !source.exists() {
                continue;
            }
            let destination = current.join(relative);
            fs::create_dir_all(destination.parent().expect("exported files have a parent"))?;
            fs::copy(source, destination)?;
        }
        let driver = current.join("profiler-core/tests/support/attribution_parity.rs");
        println!(
            "attribution parity: driver SHA-256 {}",
            sha256_file(&driver)?
        );
        let driver_path = serde_json::to_string(&driver)?;
        let mut ledgers = Vec::new();
        for (label, code, features) in [
            ("baseline", &reference.original, "test-support,baseline"),
            ("current", &current, "test-support"),
        ] {
            let manifest = code.join("profiler-core/Cargo.toml");
            let original = fs::read_to_string(&manifest)?;
            ensure!(
                original.matches("[features]\n").count() == 1,
                "{label} crate must have one feature table for the parity adapter"
            );
            fs::write(
                &manifest,
                format!(
                    "{}\n[[example]]\nname = \"attribution-parity\"\npath = {driver_path}\n",
                    original.replace("[features]\n", "[features]\nbaseline = []\n")
                ),
            )?;
            let lock = code.join("Cargo.lock");
            let locked = sha256_file(&lock)?;
            println!("attribution parity: {label} Cargo.lock SHA-256 {locked}");
            let _directory = shell.push_dir(code);
            let _target = shell.push_env("CARGO_TARGET_DIR", scratch.join("target"));
            let result = cmd!(
                shell,
                "cargo run --manifest-path {manifest} --package profiler_core --example attribution-parity --features {features} --locked --offline"
            )
            .output()?;
            ensure!(
                sha256_file(&lock)? == locked,
                "{label} dependency lock changed"
            );
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
        let mut comparable = Vec::new();
        for (label, output) in ["baseline", "current"].into_iter().zip(&ledgers) {
            let mut snapshots = Vec::new();
            for line in output
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                snapshots.push(Self::preserved_ledger(serde_json::from_slice(line)?)?);
            }
            fs::write(
                reference.directory.join(format!("{label}-preserved.json")),
                serde_json::to_vec_pretty(&snapshots)?,
            )?;
            comparable.push(snapshots);
        }
        ensure!(
            comparable[0] == comparable[1],
            "unapproved attribution difference; inspect retained *-preserved.json and original JSONL outputs"
        );
        let corrected = comparable[0]
            .iter()
            .filter(|snapshot| snapshot["block_policy"] == true)
            .count();
        println!(
            "attribution reference: {} full snapshots, {corrected} snapshots compare all fields except block allocation and Unknown first-seen order (independent policy-3 model tests cover those)",
            comparable[0].len() - corrected
        );
        managed::run(shell, Some(&reference))?;
        if render {
            render::run(shell, &reference.directory)?;
        }
        println!("parity-test: PASS baseline={BASELINE} rendered={render}");
        Ok(())
    }

    fn preserved_ledger(mut snapshot: serde_json::Value) -> Result<serde_json::Value> {
        if snapshot["block_policy"] != true {
            return Ok(snapshot);
        }
        let rows = snapshot["rows"]
            .as_array_mut()
            .context("ledger rows must be an array")?;
        let mut unknown = Vec::new();
        let mut known = Vec::new();
        for mut row in rows.drain(..) {
            let values = row
                .as_object_mut()
                .context("ledger row must be an object")?;
            ensure!(values.len() == 17, "ledger row schema changed");
            // Only effective block and its modifier split use the new policy.
            for field in ["block_effective", "blk_modifier"] {
                *values
                    .get_mut(field)
                    .context("missing block credit field")? = 0.into();
            }
            if values["id"] == "UNATTRIBUTED" && values["kind"] == 5 {
                if values.iter().any(|(key, value)| {
                    !matches!(key.as_str(), "id" | "kind" | "player") && value != 0
                }) {
                    unknown.push(row);
                }
            } else {
                known.push(row);
            }
        }
        *rows = known;
        // Missing block provenance can create an Unknown row earlier than damage.
        // Its remaining counters still compare, independent of insertion order.
        unknown.sort_by_key(|row| row["player"].as_u64());
        snapshot["unknown_rows"] = serde_json::Value::Array(unknown);
        Ok(snapshot)
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
        for name in [
            "ui_reference.json",
            "ui_approved.json",
            "session_reference.json",
        ] {
            fs::copy(self.directory.join(name), destination.join(name))?;
        }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Reference;

    #[test]
    fn approved_block_projection_keeps_physical_and_other_credit_differences() {
        let original = json!({"block_policy":true, "rows":[{
            "player":0,"id":"A","kind":0,"plays":0,"damage_dealt":7,"damage_blocked":2,
            "block_gained":9,"block_effective":3,"dmg_direct":7,"dmg_attributed":0,
            "dmg_modifier":0,"blk_modifier":1,"mitigate_debuff":4,"mitigate_buff":0,
            "mitigate_str":0,"self_damage":0,"forge":0
        }]});
        let mut corrected = original.clone();
        corrected["rows"][0]["block_effective"] = 2.into();
        corrected["rows"][0]["blk_modifier"] = 3.into();
        assert_eq!(
            Reference::preserved_ledger(original.clone()).expect("valid ledger"),
            Reference::preserved_ledger(corrected.clone()).expect("valid ledger")
        );
        for field in [
            "player",
            "kind",
            "plays",
            "damage_dealt",
            "damage_blocked",
            "block_gained",
            "dmg_direct",
            "dmg_attributed",
            "dmg_modifier",
            "mitigate_debuff",
            "mitigate_buff",
            "mitigate_str",
            "self_damage",
            "forge",
        ] {
            let mut changed = corrected.clone();
            changed["rows"][0][field] = 99.into();
            assert_ne!(
                Reference::preserved_ledger(original.clone()).expect("valid ledger"),
                Reference::preserved_ledger(changed).expect("valid ledger"),
                "unapproved row field {field} must still fail"
            );
        }
        let mut strict = original.clone();
        strict["block_policy"] = false.into();
        corrected["block_policy"] = false.into();
        assert_ne!(
            Reference::preserved_ledger(strict).expect("valid ledger"),
            Reference::preserved_ledger(corrected).expect("valid ledger")
        );
    }
}
