//! Offline poison evidence checks use exact integer arithmetic and never load
//! the native engine. A matched checkpoint proves consistency of recorded
//! evidence, not that the recorder observed every game action.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use serde_json::Value;

mod poison;

const MAX_LINE: u64 = 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_FILES: usize = 1000;

struct Event {
    seq: u64,
    turn: u64,
    name: String,
    data: Value,
}

struct Trace {
    events: Vec<Event>,
    incomplete: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Source {
    id: String,
    kind: u64,
    player: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Weights(Vec<(Source, u128)>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Grant {
    remaining: u64,
    source: Weights,
}

#[derive(Clone)]
struct Poison {
    instance: u64,
    owner: u64,
    amount: u64,
    grants: Vec<Grant>,
    source: Weights,
}

struct Native {
    combat_id: u64,
    poison: BTreeMap<u64, Poison>,
    cards: BTreeMap<Source, i64>,
}

#[derive(Clone)]
enum Check {
    Match(String),
    Discrepancy(String),
    Unverified(String),
}

impl Check {
    fn text(&self) -> (&str, &str) {
        match self {
            Self::Match(text) => ("MATCH", text),
            Self::Discrepancy(text) => ("DISCREPANCY", text),
            Self::Unverified(text) => ("UNVERIFIABLE", text),
        }
    }
}

pub(crate) fn run(input: &Path, output: Option<&Path>) -> Result<()> {
    let files = Trace::files(input)?;
    if let Some(output) = output
        && output.exists()
    {
        let output = output.canonicalize()?;
        ensure!(
            !files
                .iter()
                .any(|input| input.canonicalize().ok().as_ref() == Some(&output)),
            "output must not overwrite an input trace"
        );
    }
    // Validate every envelope before opening the output, so unsupported versions
    // do not leave a plausible-looking partial report behind.
    for path in &files {
        Trace::read(BufReader::new(File::open(path)?))
            .with_context(|| format!("reading {}", path.display()))?;
    }
    let mut output: Box<dyn Write> = match output {
        Some(path) => Box::new(BufWriter::new(File::create(path)?)),
        None => Box::new(BufWriter::new(std::io::stdout().lock())),
    };
    writeln!(output, "# Poison audit\n")?;
    writeln!(
        output,
        "Recorded events combine game values with capture metadata: sequence, execution, native identities and status are recorder claims. Native checkpoints are attribution claims. Version 2 reconstructs supported Poison suppliers from raw game evidence; version 1 checks native consistency. Neither proves capture completeness or whether FIFO is the desired policy. Checkpoint discrepancies may include interleaved effects and require review.\n"
    )?;
    for path in files {
        let trace = Trace::read(BufReader::new(File::open(&path)?))?;
        output.write_all(trace.report(&path)?.as_bytes())?;
    }
    output.flush()?;
    Ok(())
}

impl Trace {
    fn files(input: &Path) -> Result<Vec<PathBuf>> {
        if input.is_file() {
            ensure!(
                fs::metadata(input)?.len() <= MAX_FILE,
                "audit exceeds 64 MiB"
            );
            return Ok(vec![input.to_owned()]);
        }
        ensure!(
            input.is_dir(),
            "audit input does not exist: {}",
            input.display()
        );
        let mut pending = vec![(input.to_owned(), 0)];
        let mut files = Vec::new();
        let mut entries = 0;
        while let Some((directory, depth)) = pending.pop() {
            ensure!(depth <= 32, "audit directory nesting exceeds 32 levels");
            for entry in fs::read_dir(directory)? {
                entries += 1;
                ensure!(
                    entries <= 100_000,
                    "audit directory exceeds 100,000 entries"
                );
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    pending.push((entry.path(), depth + 1));
                } else if kind.is_file()
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".audit.jsonl")
                {
                    ensure!(entry.metadata()?.len() <= MAX_FILE, "audit exceeds 64 MiB");
                    files.push(entry.path());
                    ensure!(files.len() <= MAX_FILES, "audit input exceeds 1000 files");
                }
            }
        }
        files.sort();
        ensure!(!files.is_empty(), "no .audit.jsonl files found");
        Ok(files)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Framing, sequence and version checks share one bounded stream."
    )]
    fn read(mut reader: impl BufRead) -> Result<Self> {
        let mut trace = Self {
            events: Vec::new(),
            incomplete: Vec::new(),
        };
        let mut total = 0;
        let mut previous = 0_u64;
        loop {
            let mut line = Vec::new();
            let count = reader
                .by_ref()
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            ensure!(total <= MAX_FILE, "audit exceeds 64 MiB");
            ensure!(count as u64 <= MAX_LINE, "audit event exceeds 1 MiB");
            ensure!(
                trace.events.len() < MAX_EVENTS,
                "audit exceeds 100,000 events"
            );
            if !line.ends_with(b"\n") {
                trace
                    .incomplete
                    .push("Unterminated final line; its event is not trusted.".into());
                break;
            }
            let value: Value = serde_json::from_slice(&line)
                .with_context(|| format!("invalid JSON at line {}", trace.events.len() + 1))?;
            let seq = number(&value, "seq")?;
            let turn = number(&value, "turn")?;
            ensure!(turn <= u64::from(u32::MAX), "turn does not fit u32");
            let name = string(&value, "event")?.to_owned();
            let data = value
                .get("data")
                .filter(|data| data.is_object())
                .ok_or_else(|| anyhow!("event data must be an object"))?
                .clone();
            if previous.checked_add(1) != Some(seq) {
                trace.incomplete.push(format!(
                    "Sequence gap or reordering: #{previous} to #{seq}."
                ));
            }
            previous = seq;
            if name == "combat_start" {
                Self::versions(&data, true)?;
            }
            if let Some(native) = data.get("native").filter(|native| !native.is_null()) {
                Self::versions(native, false)?;
                if let Some(epoch) = trace
                    .events
                    .first()
                    .and_then(|event| event.data["epoch"].as_u64())
                    && native["combat_id"].as_u64() != Some(epoch)
                {
                    trace.incomplete.push(format!("Native checkpoint #{seq} has a missing or foreign combat identity; expected epoch {epoch}."));
                }
            }
            trace.events.push(Event {
                seq,
                turn,
                name,
                data,
            });
        }
        for (event, name) in [
            (trace.events.first(), "combat_start"),
            (trace.events.last(), "combat_end"),
        ] {
            if event.is_none_or(|event| event.name != name) {
                trace.incomplete.push(format!(
                    "Missing {name} boundary; capture may be interrupted."
                ));
            }
            if trace
                .events
                .iter()
                .filter(|event| event.name == name)
                .count()
                > 1
            {
                trace
                    .incomplete
                    .push(format!("Multiple {name} boundaries in one file."));
            }
            if name == "combat_end"
                && event.is_some_and(|event| {
                    event.data["complete"].as_bool() != Some(true)
                        || event.data["native"]["coverage"]["complete"].as_bool() != Some(true)
                })
            {
                trace
                    .incomplete
                    .push("Footer does not affirm complete journal and native coverage.".into());
            }
        }
        Ok(trace)
    }

    fn versions(value: &Value, journal: bool) -> Result<()> {
        ensure!(
            matches!(
                (number(value, "audit_version")?, journal),
                (1, _) | (2, true)
            ),
            "unsupported audit version"
        );
        ensure!(
            number(value, "policy_version")? == 3,
            "unsupported attribution policy version"
        );
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Correlation needs both earlier checkpoints and later outcomes."
    )]
    fn checks(&self) -> BTreeMap<u64, Vec<Check>> {
        let mut checks: BTreeMap<u64, Vec<Check>> = BTreeMap::new();
        let header = self.events.first();
        let Some(epoch) = header
            .filter(|event| event.name == "combat_start")
            .and_then(|event| event.data["epoch"].as_u64())
            .filter(|epoch| *epoch != 0)
        else {
            checks
                .entry(header.map_or(0, |event| event.seq))
                .or_default()
                .push(Check::Unverified(
                    "No supported combat header; attribution checks disabled.".into(),
                ));
            return checks;
        };
        let mut previous = BTreeMap::new();
        let mut mutations: BTreeMap<u64, Vec<&Event>> = BTreeMap::new();
        let ticks: BTreeMap<_, _> = self
            .events
            .iter()
            .filter(|event| event.name == "poison_tick")
            .map(|event| (event.seq, event))
            .collect();
        let mut damages: BTreeMap<u64, Vec<&Event>> = BTreeMap::new();
        for event in &self.events {
            let output = checks.entry(event.seq).or_default();
            if matches!(event.name.as_str(), "damage_begin" | "damage_result") {
                output.push(Check::Unverified("Non-Poison damage is game evidence for trigger review; this reporter does not check its attribution.".into()));
            }
            if event.name == "poison_change" {
                if let Some(action) = event.data["action"].as_u64().filter(|action| *action != 0) {
                    mutations.entry(action).or_default().push(event);
                }
                match event.change(&previous, epoch) {
                    Ok(found) => output.extend(found),
                    Err(error) => output.push(Check::Unverified(error.to_string())),
                }
            }
            if event.name == "poison_damage" {
                match number(&event.data, "tick_seq") {
                    Ok(tick) => damages.entry(tick).or_default().push(event),
                    Err(error) => output.push(Check::Unverified(error.to_string())),
                }
            }
            if !event.data["native"].is_null() {
                previous = Native::read(&event.data["native"])
                    .ok()
                    .filter(|native| native.combat_id == epoch)
                    .map_or_else(BTreeMap::new, |native| native.poison);
            }
        }
        for (seq, tick) in ticks {
            let output = checks.entry(seq).or_default();
            match damages.remove(&seq) {
                Some(damage) => match tick.damage(&damage, epoch) {
                    Ok(found) => output.extend(found),
                    Err(error) => output.push(Check::Unverified(error.to_string())),
                },
                None => output.push(Check::Unverified(
                    "No completed damage result for this tick.".into(),
                )),
            }
        }
        for (tick, events) in damages {
            for event in events {
                checks
                    .entry(event.seq)
                    .or_default()
                    .push(Check::Unverified(format!("No poison_tick #{tick}.")));
            }
        }
        for attempt in self
            .events
            .iter()
            .filter(|event| event.name == "poison_attempt")
        {
            let check = match mutations.remove(&attempt.seq) {
                Some(changes) => {
                    let power = attempt.data["power_identity"].as_u64().filter(|id| *id != 0);
                    let target = attempt.data["target"]["instance"].as_u64().filter(|id| *id != 0);
                    let linked = power.is_some() && target.is_some() && changes.iter().all(|change|
                        change.seq > attempt.seq && change.data["power_identity"].as_u64() == power
                        && change.data["target"]["instance"].as_u64() == target);
                    let sequences: Vec<_> = changes.iter().map(|change| change.seq).collect();
                    if linked { Check::Match(format!("Attempt #{} links to observed Poison mutations {sequences:?} by raw power and target identities; linkage does not establish final command success.", attempt.seq)) }
                    else { Check::Unverified(format!("Recorder links attempt #{} to mutations {sequences:?}, but raw identities or ordering do not independently confirm the link; applying to an existing power may change the instance.", attempt.seq)) }
                },
                None => Check::Unverified("No linked Poison mutation or command completion outcome. The attempt may be rejected, a no-op, interrupted, or incompletely captured; its outcome cannot be inferred.".into()),
            };
            checks.entry(attempt.seq).or_default().push(check);
        }
        for (action, changes) in mutations {
            for change in changes {
                checks
                    .entry(change.seq)
                    .or_default()
                    .push(Check::Unverified(format!(
                        "Mutation references missing poison_attempt #{action}."
                    )));
            }
        }
        checks
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Both evidence modes share the same bounded report layout."
    )]
    fn report(&self, path: &Path) -> Result<String> {
        let independent = self
            .events
            .first()
            .filter(|event| event.data["audit_version"] == 2)
            .map(|_| poison::Reconstruction::run(self));
        let checks = independent
            .as_ref()
            .map_or_else(|| self.checks(), |result| result.checks.clone());
        let mut text = format!(
            "## Combat file {}\n\n",
            markdown(&path.display().to_string())
        );
        let mut incomplete = self.incomplete.clone();
        for event in &self.events {
            if matches!(event.name.as_str(), "trace_truncated" | "diagnostic") {
                incomplete.push(format!("#{} {}: {}", event.seq, event.name, event.facts()));
            }
        }
        let (matched, discrepancies, unverifiable) = checks.values().flatten().fold(
            (0_usize, 0_usize, 0_usize),
            |(matched, discrepancies, unverifiable), check| match check {
                Check::Match(_) => (matched + 1, discrepancies, unverifiable),
                Check::Discrepancy(_) => (matched, discrepancies + 1, unverifiable),
                Check::Unverified(_) => (matched, discrepancies, unverifiable + 1),
            },
        );
        writeln!(
            text,
            "Evidence: **{}**. Checks: {matched} matched, {discrepancies} discrepancies, {unverifiable} unverifiable.\n",
            if incomplete.is_empty() {
                "complete journal boundaries"
            } else {
                "INCOMPLETE"
            }
        )?;
        for issue in incomplete {
            writeln!(text, "- {}", markdown(&issue))?;
        }
        text.push('\n');
        if let Some(independent) = &independent {
            independent.summary(&mut text)?;
        } else {
            text.push_str("Version 1: consistency checks only. Supplier identities are native claims, not independently reconstructed.\n\n");
        }
        let mut turn = None;
        for event in &self.events {
            if turn != Some(event.turn) {
                writeln!(text, "### Turn {}\n", event.turn)?;
                turn = Some(event.turn);
            }
            writeln!(
                text,
                "**#{} {}**\n\nRecorded event: {}\n",
                event.seq,
                markdown(&event.name),
                event.facts()
            )?;
            if let Some(found) = checks.get(&event.seq) {
                for check in found {
                    let (status, message) = check.text();
                    writeln!(text, "- **{status}**: {}", markdown(message))?;
                }
                if !found.is_empty() {
                    text.push('\n');
                }
            }
            if event.name == "combat_end" {
                event.totals(
                    &mut text,
                    self.events
                        .first()
                        .and_then(|event| event.data["epoch"].as_u64()),
                )?;
            }
        }
        Ok(text)
    }
}

impl Event {
    fn facts(&self) -> String {
        self.data
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| key.as_str() != "native")
            .map(|(key, value)| format!("{}={}", markdown(key), markdown(&value.to_string())))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn totals(&self, text: &mut String, epoch: Option<u64>) -> Result<()> {
        match Native::read(&self.data["native"]).and_then(|native| {
            ensure!(
                Some(native.combat_id) == epoch,
                "Final native totals belong to another combat"
            );
            Ok(native)
        }) {
            Ok(native) => {
                text.push_str("Native final indirect credits (not independently established game totals):\n\n| Source | Player | Indirect damage |\n| --- | ---: | ---: |\n");
                for (source, damage) in native.cards.iter().filter(|(_, damage)| **damage != 0) {
                    writeln!(
                        text,
                        "| {} (kind {}) | {} | {} |",
                        markdown(&source.id),
                        source.kind,
                        source.player,
                        damage
                    )?;
                }
                text.push('\n');
            }
            Err(error) => writeln!(
                text,
                "Native final totals unavailable: {}.\n",
                markdown(&error.to_string())
            )?,
        }
        Ok(())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Owner, quantity and FIFO checks share the same mutation evidence."
    )]
    fn change(&self, previous: &BTreeMap<u64, Poison>, epoch: u64) -> Result<Vec<Check>> {
        let before = number(&self.data, "before")?;
        let observed_after = number(&self.data, "after")?;
        let attached = self.data["after_attached"]
            .as_bool()
            .ok_or_else(|| anyhow!("Missing attachment state after Poison mutation"))?;
        let after = if attached { observed_after } else { 0 };
        let instance = number(&self.data, "power_instance")?;
        let native = Native::read(&self.data["native"])?;
        ensure!(
            native.combat_id == epoch,
            "Mutation checkpoint belongs to another combat"
        );
        let mut checks = Vec::new();
        let current = native.poison.get(&instance);
        let target = number(&self.data["target"], "native_instance")?;
        ensure!(
            target != 0,
            "Mutation target has no assigned native identity"
        );
        if current
            .into_iter()
            .chain(previous.get(&instance))
            .any(|power| power.owner != target)
        {
            return Ok(vec![Check::Discrepancy(format!(
                "Poison mutation target {target} disagrees with native owner for instance {instance}; FIFO checks withheld."
            ))]);
        }
        let amount = current.map_or(0, |power| power.amount);
        checks.push(compare(amount == after, format!("Observed Poison amount {before} -> {observed_after}, attached={attached}; native instance {instance} has {amount} live stacks.")));
        if let Some(current) = current {
            checks.extend(current.checks()?);
        }
        let Some(old) = previous.get(&instance) else {
            checks.push(Check::Unverified(format!("No preceding native checkpoint for Poison instance {instance}; mutation source cannot be independently checked.")));
            return Ok(checks);
        };
        let prior = old.checks()?;
        if prior.iter().any(|check| !matches!(check, Check::Match(_))) {
            checks.extend(prior);
            checks.push(Check::Unverified(
                "Prior Poison grants are inconsistent; FIFO check withheld.".into(),
            ));
            return Ok(checks);
        }
        if old.amount != before {
            checks.push(Check::Discrepancy(format!(
                "Game mutation starts at {before}, preceding native checkpoint held {}.",
                old.amount
            )));
            return Ok(checks);
        }
        if after <= before {
            let mut expected = old.grants.clone();
            let mut consume = before - after;
            for grant in &mut expected {
                let take = consume.min(grant.remaining);
                grant.remaining -= take;
                consume -= take;
            }
            expected.retain(|grant| grant.remaining != 0);
            let observed = current.map_or(&[][..], |power| power.grants.as_slice());
            checks.push(compare(
                expected == observed,
                format!(
                    "FIFO decrease consumes {} oldest stacks on instance {instance}.",
                    before - after
                ),
            ));
        } else {
            checks.push(Check::Unverified("Positive grant supplier identity is a native claim; game evidence does not include an independently resolved supplier mixture.".into()));
        }
        Ok(checks)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Group identity and physical results bound one allocation comparison."
    )]
    fn damage(&self, results: &[&Event], epoch: u64) -> Result<Vec<Check>> {
        for (index, result) in results.iter().enumerate() {
            ensure!(
                result.data["receiver_side"].as_str() == Some("enemy")
                    && result.data["receiver_kind"].as_str() == Some("monster"),
                "Incoming, pet, or unclassified Poison damage requires a different ledger check"
            );
            ensure!(
                number(&result.data, "group_count")? == results.len() as u64
                    && number(&result.data, "group_index")? == index as u64,
                "Incomplete or reordered Poison damage result group"
            );
        }
        let before = Native::read(&self.data["native"])?;
        let last = results.last().expect("grouping produces nonempty results");
        let after = Native::read(&last.data["native"])?;
        ensure!(
            before.combat_id == epoch && after.combat_id == epoch,
            "Tick checkpoint belongs to another combat"
        );
        let instance = number(&self.data, "power_instance")?;
        let power = before
            .poison
            .get(&instance)
            .ok_or_else(|| anyhow!("No native Poison instance {instance} before tick."))?;
        ensure!(
            power.grants.iter().any(|grant| grant.remaining > 0),
            "No live Poison grants; retained historical source cannot establish tick provenance"
        );
        let mut checks = power.checks()?;
        let target = number(&self.data["target"], "native_instance")?;
        ensure!(target != 0, "Game target has no assigned native identity");
        let owner = power.owner;
        checks.push(compare(
            owner == target,
            format!("Poison native owner {owner} matches game target native identity {target}."),
        ));
        let observed_amount = number(&self.data, "poison_before")?;
        let native_amount = power.amount;
        checks.push(compare(
            native_amount == observed_amount,
            format!(
                "Game Poison before tick is {observed_amount}; native amount is {native_amount}."
            ),
        ));
        let mut damage = 0_u64;
        for event in results {
            damage = damage
                .checked_add(number(&event.data, "unblocked")?)
                .and_then(|total| total.checked_add(event.data.get("blocked")?.as_u64()?))
                .ok_or_else(|| anyhow!("Missing, negative, or overflowing physical damage"))?;
        }
        let expected = power.source.allocate(damage)?;
        let keys: BTreeSet<_> = before
            .cards
            .keys()
            .chain(after.cards.keys())
            .chain(expected.keys())
            .cloned()
            .collect();
        for source in keys {
            let delta = i128::from(*after.cards.get(&source).unwrap_or(&0))
                - i128::from(*before.cards.get(&source).unwrap_or(&0));
            let expected = *expected.get(&source).unwrap_or(&0);
            if delta != 0 || expected != 0 {
                checks.push(compare(delta == i128::from(expected), format!("Tick #{}: {} (kind {}, player {}) expected indirect +{expected}, native delta {delta:+} from {damage} observed damage.", self.seq, source.id, source.kind, source.player)));
            }
        }
        checks.extend(self.physical(results));
        Ok(checks)
    }

    fn physical(&self, results: &[&Event]) -> Vec<Check> {
        let mut checks = Vec::new();
        let Some(last) = results.last() else {
            return checks;
        };
        match (self.data["requested"].as_str(), self.data["poison_before"].as_u64()) {
            (Some(requested), Some(amount)) => checks.push(compare(requested.parse::<u64>().ok() == Some(amount),
                format!("Requested Poison damage {requested}; observed stacks at modifier entry {amount}. Equality assumes no intervening power mutation."))),
            _ => checks.push(Check::Unverified("Requested Poison damage or modifier-entry stack count unavailable.".into())),
        }
        if results.len() == 1
            && self.data["target"]["instance"]
                .as_u64()
                .is_some_and(|target| {
                    target != 0 && Some(target) == last.data["target"]["instance"].as_u64()
                })
        {
            match (self.data["hp_before"].as_u64(), last.data["hp_after"].as_u64(), last.data["unblocked"].as_u64()) {
                (Some(before), Some(after), Some(unblocked)) => checks.push(compare(before.checked_sub(after) == Some(unblocked), format!("HP before modifiers {before} -> after results {after}; raw unblocked damage {unblocked}. Equality assumes no intervening HP changes; overkill is excluded."))),
                _ => checks.push(Check::Unverified("HP checkpoint conservation unavailable.".into())),
            }
        } else {
            checks.push(Check::Unverified(
                "HP conservation unavailable for redirected or unidentified receivers.".into(),
            ));
        }
        checks
    }
}

impl Source {
    fn read(value: &Value) -> Result<Self> {
        let kind = number(value, "kind")?;
        let player = number(value, "player")?;
        ensure!(
            kind <= 5 && player <= 4,
            "Invalid source kind or player slot"
        );
        Ok(Self {
            id: string(value, "id")?.to_owned(),
            kind,
            player,
        })
    }
}

impl Weights {
    fn read(value: &Value) -> Result<Self> {
        let values = value
            .as_array()
            .ok_or_else(|| anyhow!("Missing supplier weights"))?;
        ensure!(
            !values.is_empty() && values.len() <= 128,
            "invalid supplier count"
        );
        let mut entries = Vec::new();
        for value in values {
            let weight = number(value, "weight")?;
            ensure!(weight > 0, "supplier weight must be positive");
            entries.push((Source::read(value)?, u128::from(weight)));
        }
        Self::normalize(entries)
    }

    fn normalize(entries: Vec<(Source, u128)>) -> Result<Self> {
        let mut merged: Vec<(Source, u128)> = Vec::new();
        for (source, weight) in entries {
            ensure!(weight > 0, "supplier weight must be positive");
            if let Some((_, previous)) = merged.iter_mut().find(|(key, _)| key == &source) {
                *previous = previous
                    .checked_add(weight)
                    .ok_or_else(|| anyhow!("supplier weight overflow"))?;
            } else {
                merged.push((source, weight));
                ensure!(merged.len() <= 128, "Supplier mixture exceeds 128 sources");
            }
        }
        let common = merged
            .iter()
            .fold(0, |common, (_, weight)| gcd(common, *weight));
        ensure!(common > 0, "empty supplier mixture");
        for (_, weight) in &mut merged {
            *weight /= common;
        }
        Ok(Self(merged))
    }

    fn allocate(&self, damage: u64) -> Result<BTreeMap<Source, u64>> {
        let total = self
            .0
            .iter()
            .try_fold(0_u128, |sum, (_, weight)| sum.checked_add(*weight))
            .ok_or_else(|| anyhow!("weight sum overflow"))?;
        let mut output = BTreeMap::new();
        let mut cumulative = 0;
        let mut allocated = 0_u64;
        for (source, weight) in &self.0 {
            cumulative += weight;
            let next = u128::from(damage)
                .checked_mul(cumulative)
                .ok_or_else(|| anyhow!("allocation product overflow"))?
                / total;
            let next = u64::try_from(next)?;
            output.insert(source.clone(), next - allocated);
            allocated = next;
        }
        Ok(output)
    }
}

impl std::fmt::Display for Weights {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, (source, weight)) in self.0.iter().enumerate() {
            if index != 0 {
                output.write_str(" + ")?;
            }
            write!(
                output,
                "{}[kind {}, player {}]:{weight}",
                source.id, source.kind, source.player
            )?;
        }
        Ok(())
    }
}

impl Poison {
    fn read(value: &Value) -> Result<Self> {
        ensure!(
            value["trusted"].as_bool() == Some(true),
            "Poison provenance is untrusted"
        );
        let amount = number(value, "amount")?;
        ensure!(
            amount <= i32::MAX as u64,
            "Poison amount exceeds native domain"
        );
        let mut grants = Vec::new();
        for grant in value["grants"]
            .as_array()
            .ok_or_else(|| anyhow!("Missing Poison grants"))?
        {
            grants.push(Grant {
                remaining: number(grant, "remaining")?,
                source: Weights::read(&grant["source"])?,
            });
            ensure!(grants.len() <= 64, "Poison instance exceeds 64 grants");
        }
        let instance = number(value, "instance")?;
        let owner = number(value, "owner")?;
        ensure!(
            instance != 0 && owner != 0,
            "Missing native Poison identity"
        );
        Ok(Self {
            instance,
            owner,
            amount,
            grants,
            source: Weights::read(&value["source"])?,
        })
    }

    fn checks(&self) -> Result<Vec<Check>> {
        let sum = self
            .grants
            .iter()
            .try_fold(0_u64, |sum, grant| sum.checked_add(grant.remaining))
            .ok_or_else(|| anyhow!("Poison grant sum overflow"))?;
        let mut checks = vec![compare(
            sum == self.amount,
            format!(
                "Poison instance {}: grant sum {sum}, native amount {}.",
                self.instance, self.amount
            ),
        )];
        if sum == 0 {
            return Ok(checks);
        }
        let mut mixture = Vec::new();
        let mut denominator = 1_u128;
        for grant in self.grants.iter().filter(|grant| grant.remaining != 0) {
            let total = grant
                .source
                .0
                .iter()
                .try_fold(0_u128, |sum, (_, weight)| sum.checked_add(*weight))
                .ok_or_else(|| anyhow!("supplier sum overflow"))?;
            let scale = total / gcd(denominator, total);
            for (_, weight) in &mut mixture {
                *weight = u128::checked_mul(*weight, scale)
                    .ok_or_else(|| anyhow!("mixture product overflow"))?;
            }
            let grant_scale = denominator / gcd(denominator, total);
            denominator = denominator
                .checked_mul(scale)
                .ok_or_else(|| anyhow!("mixture denominator overflow"))?;
            for (source, weight) in &grant.source.0 {
                let weight = weight
                    .checked_mul(u128::from(grant.remaining))
                    .and_then(|weight| weight.checked_mul(grant_scale))
                    .ok_or_else(|| anyhow!("grant product overflow"))?;
                mixture.push((source.clone(), weight));
            }
        }
        checks.push(compare(
            Weights::normalize(mixture)? == self.source,
            format!(
                "Poison instance {}: supplier mixture [{}] equals FIFO grants [{}] in oldest-first order.",
                self.instance, self.source,
                self.grants.iter().map(|grant| format!("{} stacks from ({})", grant.remaining, grant.source))
                    .collect::<Vec<_>>().join("; ")
            ),
        ));
        Ok(checks)
    }
}

impl Native {
    fn read(value: &Value) -> Result<Self> {
        ensure!(value.is_object(), "Native checkpoint unavailable");
        ensure!(
            value["coverage"]["complete"].as_bool() == Some(true)
                && value["coverage"]["failures"].as_u64() == Some(0)
                && value["coverage"]["reasons"]
                    .as_array()
                    .is_some_and(Vec::is_empty),
            "Native coverage is not complete"
        );
        let mut poison = BTreeMap::new();
        for value in value["poison"]
            .as_array()
            .ok_or_else(|| anyhow!("Missing native Poison inventory"))?
        {
            let power = Poison::read(value)?;
            ensure!(
                poison.insert(power.instance, power).is_none(),
                "duplicate native Poison identity"
            );
            ensure!(
                poison.len() <= 256,
                "Native Poison inventory exceeds 256 powers"
            );
        }
        let mut cards = BTreeMap::new();
        for value in value["cards"]
            .as_array()
            .ok_or_else(|| anyhow!("Missing native card rows"))?
        {
            let damage = value["dmg_attributed"]
                .as_i64()
                .ok_or_else(|| anyhow!("Missing indirect damage counter"))?;
            ensure!(damage >= 0, "negative indirect damage counter");
            ensure!(
                cards.insert(Source::read(value)?, damage).is_none(),
                "duplicate native source row"
            );
            ensure!(
                cards.len() <= 512,
                "Native checkpoint exceeds 512 source rows"
            );
        }
        Ok(Self {
            combat_id: number(value, "combat_id")?,
            poison,
            cards,
        })
    }
}

fn number(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Missing or invalid {field}"))
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing or invalid {field}"))
}

fn gcd(mut first: u128, mut second: u128) -> u128 {
    while second != 0 {
        (first, second) = (second, first % second);
    }
    first
}

fn compare(matched: bool, message: String) -> Check {
    if matched {
        Check::Match(message)
    } else {
        Check::Discrepancy(message)
    }
}

fn markdown(text: &str) -> String {
    let mut output = String::new();
    for character in text.chars() {
        if "\\`*_{}[]<>()#+-.!|".contains(character) {
            output.push('\\');
        }
        match character {
            '\n' | '\r' => output.push(' '),
            _ => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Fixture {
        events: Vec<Value>,
        turn: u32,
        credits: [u64; 2],
    }

    impl Fixture {
        fn source(index: usize, weight: u64) -> Value {
            json!({"id": (["SNAKEBITE", "ENVENOM"][index]), "kind": 0, "player": 0, "weight": weight})
        }

        fn native(&self, stacks: [u64; 2]) -> Value {
            let source: Vec<_> = stacks
                .into_iter()
                .enumerate()
                .filter(|(_, weight)| *weight > 0)
                .map(|(index, weight)| Self::source(index, weight))
                .collect();
            let grants: Vec<_> = stacks.into_iter().enumerate().filter(|(_, weight)| *weight > 0)
                .map(|(index, weight)| json!({"remaining": weight, "source": [Self::source(index, 1)]})).collect();
            let cards: Vec<_> = self
                .credits
                .into_iter()
                .enumerate()
                .map(|(index, damage)| {
                    let mut source = Self::source(index, 1);
                    source["dmg_attributed"] = json!(damage);
                    source
                })
                .collect();
            json!({"audit_version":1,"policy_version":3,"combat_id":7,
                "poison":[{"instance":20,"owner":30,"owner_kind":1,"owner_slot":4,
                    "amount":stacks.iter().sum::<u64>(),"trusted":true,"grants":grants,"source":source}],
                "cards":cards,"coverage":{"complete":true,"failures":0,"reasons":[]}})
        }

        fn event(&mut self, name: &str, data: Value) -> u64 {
            let seq = self.events.len() as u64 + 1;
            self.events
                .push(json!({"seq":seq,"turn":self.turn,"event":name,"data":data}));
            seq
        }

        fn nine_turns() -> Self {
            let mut fixture = Self::default();
            fixture.event("combat_start", json!({"audit_version":1,"policy_version":3,"epoch":7,
                "encounter":"SYNTHETIC","run_id":"fixture","seed":"independent","attempt_id":"fixture",
                "game_version":"fixture","mod_version":"fixture","players":[]}));
            for turn in 1..=9 {
                fixture.turn = turn;
                fixture.event("turn", json!({"side":"enemy","creatures":[]}));
                let stacks = if turn == 9 { [22, 45] } else { [16, 54] };
                let damage = stacks.iter().sum::<u64>();
                let tick = fixture.event("poison_tick", json!({"power_instance":20,"target":{"instance":40,"native_instance":30},
                    "requested":damage.to_string(),"poison_before":damage,"hp_before":1000,"block_before":0,
                    "accelerants":[{"player":0,"amount":1}],"native":fixture.native(stacks)}));
                for (credit, added) in fixture.credits.iter_mut().zip(stacks) {
                    *credit += added;
                }
                fixture.event("poison_damage", json!({"tick_seq":tick,"target":{"instance":40,"native_instance":30},
                    "hp_after":1000-damage,"unblocked":damage,"blocked":0,"overkill":0,"receiver_side":"enemy",
                    "receiver_kind":"monster","group_index":0,"group_count":1,"native":fixture.native(stacks)}));
            }
            fixture.event(
                "combat_end",
                json!({"result":"completed","complete":true,"native":fixture.native([22,45])}),
            );
            fixture
        }

        fn bytes(&self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for value in &self.events {
                writeln!(bytes, "{value}").expect("Vec writes cannot fail");
            }
            bytes
        }

        fn report(&self) -> Result<String> {
            Trace::read(self.bytes().as_slice())?.report(Path::new("synthetic.audit.jsonl"))
        }
    }

    #[test]
    fn nine_turn_totals_are_checked_per_tick_and_wrong_credit_is_reported() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        let report = fixture.report()?;
        assert!(report.contains("| SNAKEBITE (kind 0) | 0 | 150 |"));
        assert!(report.contains("| ENVENOM (kind 0) | 0 | 477 |"));
        assert!(report.contains("0 discrepancies, 0 unverifiable"));
        assert_eq!(report.matches("expected indirect").count(), 18);
        let damaged = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_damage")
            .expect("fixture has nine damage events");
        damaged["data"]["native"]["cards"][0]["dmg_attributed"] = json!(17);
        let report = fixture.report()?;
        assert!(report.contains("**DISCREPANCY**"));
        assert!(report.contains("expected indirect \\+16, native delta \\+17"));
        Ok(())
    }

    #[test]
    fn missing_footer_gaps_and_truncated_final_line_are_explicit() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        fixture.events.remove(2);
        fixture.events.pop();
        let mut bytes = fixture.bytes();
        bytes.extend_from_slice(b"{\"seq\":999,");
        let report = Trace::read(bytes.as_slice())?.report(Path::new("broken.audit.jsonl"))?;
        for expected in [
            "**INCOMPLETE**",
            "Sequence gap",
            "Missing combat",
            "Unterminated final line",
            "No poison",
        ] {
            assert!(report.contains(expected), "missing {expected}");
        }
        Ok(())
    }

    #[test]
    fn unknown_versions_and_oversized_lines_are_rejected() {
        let mut fixture = Fixture::nine_turns();
        fixture.events[0]["data"]["audit_version"] = json!(3);
        assert!(Trace::read(fixture.bytes().as_slice()).is_err());
        fixture.events[0]["data"]["audit_version"] = json!(1);
        fixture.events[0]["data"]["policy_version"] = json!(4);
        assert!(Trace::read(fixture.bytes().as_slice()).is_err());
        let line = vec![b' '; MAX_LINE as usize + 1];
        assert!(Trace::read(line.as_slice()).is_err());
    }

    #[test]
    fn fifo_checks_reject_removing_the_newer_supplier() -> Result<()> {
        let fixture = Fixture::default();
        let before = Native::read(&fixture.native([7, 7]))?;
        let mut event = Event {
            seq: 2,
            turn: 1,
            name: "poison_change".into(),
            data: json!({"power_instance":20,
            "before":14,"after":13,"after_attached":true,"status":1,"target":{"native_instance":30},"native":fixture.native([6,7])}),
        };
        assert!(
            event
                .change(&before.poison, 7)?
                .iter()
                .all(|check| matches!(check, Check::Match(_)))
        );
        event.data["native"] = fixture.native([7, 6]);
        assert!(
            event.change(&before.poison, 7)?.iter().any(
                |check| matches!(check, Check::Discrepancy(message) if message.contains("FIFO"))
            )
        );
        event.data["after"] = json!(14);
        event.data["after_attached"] = json!(false);
        event.data["native"]["poison"] = json!([]);
        assert!(
            event
                .change(&before.poison, 7)?
                .iter()
                .all(|check| matches!(check, Check::Match(_)))
        );
        event.data["target"]["native_instance"] = json!(31);
        assert!(
            event
                .change(&before.poison, 7)?
                .iter()
                .all(|check| matches!(check, Check::Discrepancy(_)))
        );
        event.data["target"]["native_instance"] = json!(30);
        let mut malformed = before.poison.clone();
        malformed
            .get_mut(&20)
            .expect("fixture has Poison")
            .grants
            .clear();
        let checks = event.change(&malformed, 7)?;
        assert!(checks.iter().any(|check| matches!(check, Check::Unverified(message) if message.contains("Prior Poison grants"))));
        assert!(
            !checks
                .iter()
                .any(|check| matches!(check, Check::Match(message) if message.starts_with("FIFO")))
        );
        Ok(())
    }

    #[test]
    fn proportional_allocation_preserves_first_seen_rounding_and_mixed_grants() -> Result<()> {
        let weights = Weights::read(&json!([Fixture::source(0, 1), Fixture::source(1, 1)]))?;
        let credits = weights.allocate(1)?;
        assert_eq!(credits[&Source::read(&Fixture::source(0, 1))?], 0);
        assert_eq!(credits[&Source::read(&Fixture::source(1, 1))?], 1);
        let mut power = Fixture::default().native([3, 2])["poison"][0].clone();
        power["grants"] = json!([
            {"remaining":3,"source":[Fixture::source(0,1),Fixture::source(1,1)]},
            {"remaining":2,"source":[Fixture::source(1,1)]}]);
        power["source"] = json!([Fixture::source(0, 3), Fixture::source(1, 7)]);
        assert!(
            Poison::read(&power)?
                .checks()?
                .iter()
                .all(|check| matches!(check, Check::Match(_)))
        );
        Ok(())
    }

    #[test]
    fn absent_native_evidence_and_incoming_damage_cannot_pass() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        let tick = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_tick")
            .expect("fixture has ticks");
        tick["data"]["native"] = Value::Null;
        assert!(fixture.report()?.contains("Native checkpoint unavailable"));
        let mut fixture = Fixture::nine_turns();
        let damage = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_damage")
            .expect("fixture has damage");
        damage["data"]["receiver_side"] = json!("player");
        assert!(fixture.report()?.contains("Incoming, pet, or unclassified"));
        Ok(())
    }

    #[test]
    fn incomplete_groups_and_partial_footers_remain_unverifiable() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        let damage = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_damage")
            .expect("fixture has damage");
        damage["data"]["group_count"] = json!(2);
        assert!(fixture.report()?.contains("Incomplete or reordered"));
        fixture.events.last_mut().expect("fixture has footer")["data"]["native"]["coverage"]["complete"] =
            json!(false);
        assert!(fixture.report()?.contains("**INCOMPLETE**"));
        let mut fixture = Fixture::nine_turns();
        fixture.events.remove(0);
        let report = fixture.report()?;
        assert!(report.contains("attribution checks disabled"));
        assert!(!report.contains("expected indirect"));
        let mut fixture = Fixture::nine_turns();
        fixture.events.last_mut().expect("fixture has footer")["data"]["native"]["combat_id"] =
            json!(8);
        let report = fixture.report()?;
        assert!(
            report.contains("**INCOMPLETE**")
                && report.contains("Final native totals belong to another combat")
        );
        assert!(!report.contains("| SNAKEBITE"));
        Ok(())
    }

    #[test]
    fn unmatched_attempts_do_not_invent_rejection_or_success() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        fixture.events.pop();
        let attempt = fixture.event("poison_attempt", json!({"command":"Apply","amount":"7.0","source":"SNAKEBITE","power_identity":41,"power_instance":0,"target":{"instance":40}}));
        fixture.event(
            "combat_end",
            json!({"complete":true,"native":fixture.native([22,45])}),
        );
        let report = fixture.report()?;
        assert!(report.contains("No linked Poison mutation"));
        assert!(report.contains("outcome cannot be inferred"));
        fixture.events.pop();
        let mutation = fixture.event("poison_change", json!({"action":attempt,"before":60,"after":67,"after_attached":true,"power_instance":20,"power_identity":41,"target":{"instance":40,"native_instance":30},"native":fixture.native([22,45])}));
        fixture.event(
            "combat_end",
            json!({"complete":true,"native":fixture.native([22,45])}),
        );
        let report = fixture.report()?;
        assert!(report.contains("links to observed Poison mutations"));
        assert!(!report.contains("No linked Poison mutation"));
        fixture.events[mutation as usize - 1]["data"]["power_identity"] = json!(42);
        assert!(
            fixture
                .report()?
                .contains("do not independently confirm the link")
        );
        Ok(())
    }

    #[test]
    fn empty_grants_and_redirected_hp_cannot_claim_causal_matches() -> Result<()> {
        let mut fixture = Fixture::nine_turns();
        let tick = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_tick")
            .expect("fixture has ticks");
        tick["data"]["native"]["poison"][0]["grants"] = json!([]);
        tick["data"]["native"]["poison"][0]["amount"] = json!(0);
        assert!(fixture.report()?.contains("No live Poison grants"));
        let mut fixture = Fixture::nine_turns();
        let damage = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "poison_damage")
            .expect("fixture has damage");
        damage["data"]["target"]["instance"] = json!(41);
        let report = fixture.report()?;
        assert!(report.contains("HP conservation unavailable for redirected"));
        assert_eq!(report.matches("HP before modifiers").count(), 8);
        Ok(())
    }
}
