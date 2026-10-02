//! Review freshness covers tokenized declarations and the scanner's one level
//! of same-class helper matches, not semantic equivalence or runtime coverage.
//! Each declaration hashes Roslyn token kind/text pairs without trivia. The
//! review hashes that digest with sorted, distinct helper declaration digests.
//! `fingerprints.json` contains accepted reviews. Every check writes a proposed
//! replacement and diagnostics under `tmp/catalog-review`; after reading changed
//! bodies, update the curated decisions and explicitly copy the reviewed proposal.
//! Bump the format when normalization or helper selection changes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::de::{Error, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Candidate, ClassFile, Method, Review};
use crate::{catalog, game_version, workspace_root};

const FORMAT_VERSION: u32 = 1;

#[cfg(test)]
mod tests;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fingerprints {
    format_version: u32,
    game_version: String,
    #[serde(deserialize_with = "Fingerprints::unique_methods")]
    methods: BTreeMap<String, String>,
}

impl Review {
    pub(super) fn check_fingerprints(
        &mut self,
        files: [(&str, &BTreeMap<String, ClassFile>); 2],
        candidates: &[Candidate<'_>],
    ) -> Result<()> {
        let reviewed: BTreeSet<_> = catalog::RELICS
            .iter()
            .map(|(class, method)| ("Relics", *class, *method))
            .chain(
                catalog::POWERS
                    .iter()
                    .map(|(class, method)| ("Powers", *class, *method)),
            )
            .chain(catalog::REVIEWED_CANDIDATES.iter().copied())
            .collect();
        let requested = reviewed
            .iter()
            .copied()
            .chain(
                candidates
                    .iter()
                    .map(|candidate| (candidate.namespace, candidate.class, candidate.method)),
            )
            .collect();
        let (candidate, failures) = Fingerprints::capture(files, &requested);
        self.failures.extend(failures);
        let expected = reviewed
            .iter()
            .map(|(namespace, class, method)| format!("{namespace}.{class}.{method}"))
            .collect();
        let root = workspace_root();
        candidate.write_review(
            &root.join("xtask/src/check_catalog/fingerprints.json"),
            &root.join("tmp/catalog-review"),
            &expected,
            &mut self.failures,
        )
    }
}

impl Fingerprints {
    fn unique_methods<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<BTreeMap<String, String>, D::Error> {
        struct UniqueMethods;
        impl<'de> Visitor<'de> for UniqueMethods {
            type Value = BTreeMap<String, String>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a map of unique method identities to fingerprints")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut methods = BTreeMap::new();
                while let Some((identity, fingerprint)) = entries.next_entry::<String, String>()? {
                    if methods.insert(identity.clone(), fingerprint).is_some() {
                        return Err(A::Error::custom(format!(
                            "duplicate method identity {identity}"
                        )));
                    }
                }
                Ok(methods)
            }
        }
        deserializer.deserialize_map(UniqueMethods)
    }

    fn capture(
        files: [(&str, &BTreeMap<String, ClassFile>); 2],
        requested: &BTreeSet<(&str, &str, &str)>,
    ) -> (Self, Vec<String>) {
        let mut methods = BTreeMap::new();
        let mut failures = Vec::new();
        for (namespace, class, name) in requested {
            let identity = format!("{namespace}.{class}.{name}");
            let file = files
                .iter()
                .find(|(label, _)| label == namespace)
                .and_then(|(_, classes)| classes.get(*class));
            let Some(file) = file else {
                failures.push(format!("fingerprint source {identity}: class is missing"));
                continue;
            };
            let mut declarations = file.methods.iter().filter(|method| method.name == *name);
            let (Some(method), None) = (declarations.next(), declarations.next()) else {
                failures.push(format!(
                    "fingerprint source {identity}: method is missing or overloaded"
                ));
                continue;
            };
            methods.insert(identity, method.review_fingerprint(&file.methods));
        }
        (
            Self {
                format_version: FORMAT_VERSION,
                game_version: game_version::PIN.to_owned(),
                methods,
            },
            failures,
        )
    }

    fn compare(&self, accepted: &Self, expected: &BTreeSet<String>) -> Vec<String> {
        let mut failures = Vec::new();
        if accepted.format_version != self.format_version {
            failures.push(format!(
                "fingerprint format {} is not supported; expected {}",
                accepted.format_version, self.format_version
            ));
        }
        if accepted.game_version != self.game_version {
            failures.push(format!(
                "fingerprint game {} does not match pinned {}",
                accepted.game_version, self.game_version
            ));
        }
        for identity in expected {
            match (accepted.methods.get(identity), self.methods.get(identity)) {
                (None, _) => failures.push(format!("missing reviewed fingerprint: {identity}")),
                (Some(before), Some(after)) if before != after => {
                    failures.push(format!("stale reviewed fingerprint: {identity}"));
                }
                _ => {}
            }
        }
        failures.extend(
            accepted
                .methods
                .keys()
                .filter(|identity| !expected.contains(*identity))
                .map(|identity| format!("orphaned reviewed fingerprint: {identity}")),
        );
        failures
    }

    fn write_review(
        &self,
        accepted: &Path,
        output: &Path,
        expected: &BTreeSet<String>,
        failures: &mut Vec<String>,
    ) -> Result<()> {
        fs::create_dir_all(output)?;
        let candidate = output.join("candidate.json");
        fs::write(
            &candidate,
            format!("{}\n", serde_json::to_string_pretty(self)?),
        )?;
        let baseline = fs::read_to_string(accepted)
            .with_context(|| format!("reading reviewed fingerprints {}", accepted.display()))
            .and_then(|text| {
                serde_json::from_str::<Self>(&text).context("parsing reviewed fingerprints")
            });
        match baseline {
            Ok(baseline) => failures.extend(self.compare(&baseline, expected)),
            Err(error) => failures.push(format!("{error:#}")),
        }
        failures.sort();
        failures.dedup();
        let report = output.join("changes.txt");
        let mut changes = format!(
            "Catalog review for {} (fingerprint format {})\nCandidate: {}\nAccepted: {}\n",
            self.game_version,
            self.format_version,
            candidate.display(),
            accepted.display(),
        );
        if failures.is_empty() {
            changes.push_str("No review changes.\n");
        } else {
            changes.push_str(&failures.join("\n"));
            changes.push('\n');
        }
        fs::write(&report, changes)?;
        println!("catalog review: {}", report.display());
        Ok(())
    }
}

impl Method {
    fn review_fingerprint(&self, methods: &[Self]) -> String {
        let mut helpers: Vec<_> = self
            .reviewed_methods(methods)
            .skip(1)
            .map(|method| &method.fingerprint)
            .collect();
        helpers.sort();
        helpers.dedup();
        let input = serde_json::to_vec(&(&self.fingerprint, helpers))
            .expect("fingerprint tuples contain only JSON strings");
        format!("{:x}", Sha256::digest(input))
    }
}
