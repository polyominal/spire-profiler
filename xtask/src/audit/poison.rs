//! Expected suppliers come exclusively from raw source frames and accepted game
//! mutations. Native snapshots enter only comparisons, never the reconstructed
//! queue. Unsupported provenance remains unknown until that attachment drains.

use super::*;

#[cfg(test)]
mod tests;

type Evidence<T> = std::result::Result<T, String>;

struct Power {
    model: String,
    owner: u64,
    native_id: u64,
    native_owner: u64,
    amount: u64,
    grants: Option<Vec<Grant>>,
}

struct Frame {
    identity: u64,
    model: String,
    owner: u64,
    amount: u64,
    context: String,
    source: Evidence<Weights>,
}

pub(super) struct Reconstruction {
    pub(super) checks: BTreeMap<u64, Vec<Check>>,
    powers: BTreeMap<u64, Power>,
    frames: BTreeMap<u64, Frame>,
    credits: BTreeMap<Source, u64>,
    complete: bool,
    epoch: u64,
}

impl Reconstruction {
    #[allow(
        clippy::too_many_lines,
        reason = "Prefix-bounded causal joins and ordered replay share one evidence boundary."
    )]
    pub(super) fn run(trace: &Trace) -> Self {
        let mut result = Self {
            checks: BTreeMap::new(),
            powers: BTreeMap::new(),
            frames: BTreeMap::new(),
            credits: BTreeMap::new(),
            complete: true,
            epoch: trace
                .events
                .first()
                .and_then(|event| event.data["epoch"].as_u64())
                .unwrap_or(0),
        };
        let mut sequence = 0_u64;
        let indexed: BTreeMap<_, _> = trace
            .events
            .iter()
            .take_while(|event| {
                let continuous = sequence.checked_add(1) == Some(event.seq)
                    && !matches!(event.name.as_str(), "diagnostic" | "trace_truncated");
                sequence = event.seq;
                continuous
            })
            .map(|event| (event.seq, event))
            .collect();
        let mut previous = 0_u64;
        let mut interrupted = result.epoch == 0;
        for event in &trace.events {
            if previous.checked_add(1) != Some(event.seq)
                || matches!(event.name.as_str(), "diagnostic" | "trace_truncated")
            {
                interrupted = true;
            }
            previous = event.seq;
            if interrupted {
                result.complete = false;
                result.add(event.seq, Check::Unverified("Raw event continuity is unavailable; subsequent independent reconstruction is withheld.".into()));
                continue;
            }
            let outcome = match event.name.as_str() {
                "source_frame" => result.frame(event),
                "power_change" => result.change(event, &indexed),
                "power_attempt" => result.command(event, &indexed),
                "envenom_trigger" => result.envenom(event, &indexed),
                "poison_tick" => result.tick(event, &indexed),
                "poison_damage" => {
                    if event.data["tick_seq"]
                        .as_u64()
                        .and_then(|seq| indexed.get(&seq))
                        .is_some_and(|tick| tick.name == "poison_tick" && tick.seq < event.seq)
                    {
                        Ok(())
                    } else {
                        Err(anyhow!("Physical Poison result lacks a preceding raw tick"))
                    }
                }
                _ => Ok(()),
            };
            if let Err(error) = outcome {
                result.complete = false;
                if event.name == "power_change" {
                    for power in result.powers.values_mut() {
                        power.grants = None;
                    }
                }
                result.add(event.seq, Check::Unverified(error.to_string()));
            }
            if !event.data["native"].is_null() {
                match result.checkpoint(&event.data["native"]) {
                    Ok(checks) => result.checks.entry(event.seq).or_default().extend(checks),
                    Err(error) => result.add(event.seq, Check::Unverified(error.to_string())),
                }
            }
        }
        if !trace.incomplete.is_empty() {
            result.complete = false;
        }
        result
    }

    fn add(&mut self, seq: u64, check: Check) {
        self.checks.entry(seq).or_default().push(check);
    }

    fn frame(&mut self, event: &Event) -> Result<()> {
        let raw = &event.data["source"];
        let identity = number(raw, "identity")?;
        let model = string(raw, "model")?.to_owned();
        let amount = Self::amount(&event.data["amount"])?;
        let context = string(&event.data, "context")?.to_owned();
        let source = match string(raw, "role")? {
            "card" => Self::card(raw),
            "power" => (|| {
                ensure!(
                    matches!(model.as_str(), "POISON_POWER" | "ENVENOM_POWER"),
                    "Unsupported source power {model}"
                );
                ensure!(
                    number(&event.data, "power_identity")? == identity && identity != 0,
                    "Source frame power identity disagrees with raw source"
                );
                let power = self.powers.get(&identity).ok_or_else(|| {
                    anyhow!("No independently observed attachment for source power {identity}")
                })?;
                ensure!(
                    power.model == model
                        && power.owner == number(&event.data["target"], "instance")?,
                    "Source frame power ownership differs from its observed attachment"
                );
                ensure!(
                    power.amount == Self::amount(&event.data["amount"])?,
                    "Source frame amount differs from independently observed mutations"
                );
                power.weights()
            })(),
            _ => Err(anyhow!("Unsupported or ambiguous raw source provenance")),
        }
        .map_err(|error| error.to_string());
        let check = match &source {
            Ok(weights) => Check::Match(format!(
                "Independent source frame frozen from raw evidence: [{weights}]."
            )),
            Err(reason) => {
                Check::Unverified(format!("Source frame cannot be reconstructed: {reason}."))
            }
        };
        self.add(event.seq, check);
        self.frames.insert(
            event.seq,
            Frame {
                identity,
                model,
                owner: event.data["target"]["instance"].as_u64().unwrap_or(0),
                amount,
                context,
                source,
            },
        );
        Ok(())
    }

    fn card(raw: &Value) -> Result<Weights> {
        ensure!(
            raw["origin"].as_str() == Some("ordinary"),
            "Generated or unproven card origin is outside independent reconstruction"
        );
        ensure!(number(raw, "identity")? != 0, "Card identity is missing");
        let player = number(raw, "player")?;
        ensure!(player < 4, "Ordinary card owner is not a player");
        let id = string(raw, "model")?.to_owned();
        ensure!(!id.is_empty(), "Ordinary card model is missing");
        Ok(Weights(vec![(
            Source {
                id,
                kind: 0,
                player,
            },
            1,
        )]))
    }

    fn amount(value: &Value) -> Result<u64> {
        let value = match value.as_u64() {
            Some(value) => value,
            None => value
                .as_str()
                .ok_or_else(|| anyhow!("Missing nonnegative integral power amount"))?
                .parse()?,
        };
        ensure!(
            value <= i32::MAX as u64,
            "Power amount is outside the supported game domain"
        );
        Ok(value)
    }

    fn source(&self, event: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<Weights> {
        let action = number(&event.data, "action")?;
        let attempt = indexed
            .get(&action)
            .filter(|attempt| attempt.name == "power_attempt" && attempt.seq < event.seq)
            .ok_or_else(|| anyhow!("Positive mutation has no preceding raw command attempt"))?;
        ensure!(
            attempt.data["power"] == event.data["power"]
                && attempt.data["target"]["instance"] == event.data["target"]["instance"],
            "Mutation and attempt raw power model or target differ"
        );
        let end = Self::command_end(attempt, indexed)?;
        ensure!(
            end.seq > event.seq && end.data["power_identity"] == event.data["power_identity"],
            "Command completion does not bind the actual mutated power identity"
        );
        let frame_id = number(&event.data, "source_frame")?;
        ensure!(
            frame_id != 0
                && frame_id < attempt.seq
                && attempt.data["source_frame"].as_u64() == Some(frame_id),
            "Mutation source does not match its command's frozen raw frame"
        );
        let frame = self
            .frames
            .get(&frame_id)
            .ok_or_else(|| anyhow!("Missing frozen source frame #{frame_id}"))?;
        ensure!(
            attempt.data["source"]["identity"].as_u64() == Some(frame.identity)
                && attempt.data["source"]["model"].as_str() == Some(frame.model.as_str()),
            "Command's raw source differs from the frozen source frame"
        );
        if frame.model == "ENVENOM_POWER" {
            ensure!(
                frame.context == "producer",
                "Envenom supplier frame was not sampled at producer entry"
            );
            let cause = number(&attempt.data, "cause")?;
            let trigger = indexed
                .get(&cause)
                .filter(|cause| cause.name == "envenom_trigger" && cause.seq < attempt.seq)
                .ok_or_else(|| anyhow!("Envenom application lacks a preceding trigger"))?;
            ensure!(
                trigger.data["source_frame"].as_u64() == Some(frame_id)
                    && trigger.data["power_identity"].as_u64() == Some(frame.identity)
                    && trigger.data["target"]["instance"] == event.data["target"]["instance"],
                "Envenom trigger does not bind this source frame and receiver"
            );
            ensure!(
                Self::eligible(trigger, indexed)?,
                "Raw Envenom callback is not eligible to apply Poison"
            );
            ensure!(
                trigger.data["owner"]["instance"].as_u64() == Some(frame.owner),
                "Envenom callback owner differs from its frozen power owner"
            );
            ensure!(
                Self::amount(&attempt.data["amount"])? == Self::amount(&trigger.data["amount"])?,
                "Envenom command request differs from its observed trigger amount"
            );
            ensure!(
                Self::amount(&trigger.data["amount"])? == frame.amount
                    && event.data["power"].as_str() == Some("POISON_POWER"),
                "Envenom child amount or power differs from the independently observed producer"
            );
            ensure!(
                Self::trigger_end(trigger, indexed)?.seq > event.seq,
                "Envenom child mutation is outside its callback lifetime"
            );
        }
        frame.source.clone().map_err(|reason| anyhow!(reason))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Physical attachment transitions and provenance update form one transaction."
    )]
    fn change(&mut self, event: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<()> {
        let model = string(&event.data, "power")?.to_owned();
        ensure!(
            matches!(model.as_str(), "POISON_POWER" | "ENVENOM_POWER"),
            "Unsupported power mutation {model}"
        );
        let identity = number(&event.data, "power_identity")?;
        let owner = number(&event.data["target"], "instance")?;
        ensure!(identity != 0 && owner != 0, "Raw mutation identity missing");
        let before_attached = event.data["before_attached"]
            .as_bool()
            .ok_or_else(|| anyhow!("Missing prior attachment state"))?;
        let attached = event.data["after_attached"]
            .as_bool()
            .ok_or_else(|| anyhow!("Missing resulting attachment state"))?;
        let before = if before_attached {
            Self::amount(&event.data["before"])?
        } else {
            0
        };
        let after = if attached {
            Self::amount(&event.data["after"])?
        } else {
            0
        };
        let source = if after > before {
            Some(self.source(event, indexed))
        } else {
            None
        };
        let power = self.powers.entry(identity).or_insert_with(|| Power {
            model: model.clone(),
            owner,
            native_id: 0,
            native_owner: 0,
            amount: before,
            grants: if before == 0 { Some(Vec::new()) } else { None },
        });
        if power.model != model || power.owner != owner || power.amount != before {
            power.grants = None;
        }
        power.native_id = event.data["power_instance"].as_u64().unwrap_or(0);
        power.native_owner = event.data["target"]["native_instance"]
            .as_u64()
            .unwrap_or(0);
        let mut unknown = None;
        if let Some(source) = source {
            match (source, power.grants.as_mut()) {
                (Ok(source), Some(grants)) => {
                    if let Some(last) = grants.last_mut().filter(|last| last.source == source) {
                        last.remaining += after - before;
                    } else {
                        grants.push(Grant {
                            remaining: after - before,
                            source,
                        });
                    }
                }
                (Err(error), _) => {
                    power.grants = None;
                    unknown = Some(error.to_string());
                }
                (Ok(_), None) => {}
            }
        } else if let Some(grants) = &mut power.grants {
            let mut removed = before - after;
            for grant in grants.iter_mut() {
                let take = removed.min(grant.remaining);
                grant.remaining -= take;
                removed -= take;
            }
            grants.retain(|grant| grant.remaining != 0);
        }
        power.amount = after;
        if after == 0 {
            power.grants = Some(Vec::new());
        }
        let check = match &power.grants {
            Some(grants) => Check::Match(format!(
                "Independent {model} #{identity}: {before} -> {after} stacks; FIFO has {} grants from accepted raw mutations.",
                grants.len()
            )),
            None => {
                self.complete = false;
                Check::Unverified(format!(
                    "{model} #{identity} supplier queue is tainted: {}. Native suppliers are not used to fill the gap.",
                    unknown
                        .as_deref()
                        .unwrap_or("missing or inconsistent earlier raw evidence")
                ))
            }
        };
        self.add(event.seq, check);
        Ok(())
    }

    fn command_end<'a>(attempt: &Event, indexed: &BTreeMap<u64, &'a Event>) -> Result<&'a Event> {
        let mut ends = indexed.values().filter(|event| {
            event.name == "command_end" && event.data["action"].as_u64() == Some(attempt.seq)
        });
        let end = ends
            .next()
            .ok_or_else(|| anyhow!("Command #{} has no recorded completion", attempt.seq))?;
        ensure!(
            ends.next().is_none() && end.seq > attempt.seq,
            "Command completion is duplicated or out of order"
        );
        ensure!(
            end.data["command"] == attempt.data["command"]
                && end.data["target"]["instance"] == attempt.data["target"]["instance"]
                && end.data["requested_power_identity"].as_u64()
                    == Some(number(&attempt.data, "power_identity")?),
            "Command completion has a different raw command, requested power or target"
        );
        ensure!(
            matches!(
                end.data["outcome"].as_str(),
                Some("completed" | "faulted" | "cancelled")
            ),
            "Missing command outcome"
        );
        Ok(end)
    }

    fn command(&mut self, attempt: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<()> {
        let end = Self::command_end(attempt, indexed)?;
        let count = indexed
            .values()
            .filter(|event| {
                event.name == "power_change" && event.data["action"].as_u64() == Some(attempt.seq)
            })
            .count();
        self.add(attempt.seq, Check::Match(format!("Raw command ends {} at #{} with {count} observed mutations. Completion alone does not establish acceptance or explain a no-op.", string(&end.data, "outcome")?, end.seq)));
        Ok(())
    }

    fn eligible(trigger: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<bool> {
        let result_id = number(&trigger.data, "result_identity")?;
        ensure!(
            result_id != 0,
            "Envenom trigger lacks a raw damage result identity"
        );
        let mut candidates = indexed.values().filter(|event| {
            matches!(event.name.as_str(), "damage_result" | "poison_damage")
                && event.data["result_identity"].as_u64() == Some(result_id)
        });
        let result = candidates
            .next()
            .ok_or_else(|| anyhow!("No raw damage result for Envenom trigger"))?;
        ensure!(
            candidates.next().is_none() && result.seq < trigger.seq,
            "Envenom raw result identity is duplicated or ordered incorrectly"
        );
        ensure!(
            result.data["target"]["instance"] == trigger.data["target"]["instance"]
                && result.data["unblocked"] == trigger.data["unblocked"],
            "Envenom receiver or damage differs from its raw result"
        );
        let begin = indexed
            .get(&number(&result.data, "tick_seq")?)
            .ok_or_else(|| anyhow!("No raw damage begin for Envenom result"))?;
        ensure!(
            begin.seq < result.seq && matches!(begin.name.as_str(), "damage_begin" | "poison_tick"),
            "Envenom lacks a completed raw damage observation"
        );
        let owner = number(&trigger.data["owner"], "instance")?;
        ensure!(
            owner != 0 && trigger.data["dealer"]["instance"] == begin.data["dealer"]["instance"],
            "Envenom owner is missing or damage dealer differs from raw begin"
        );
        let props = number(&trigger.data, "props")?;
        ensure!(
            begin.data["props"].as_u64() == Some(props),
            "Envenom damage properties differ from the raw begin"
        );
        Ok(trigger.data["dealer"]["instance"].as_u64() == Some(owner)
            && props & 8 != 0
            && props & 4 == 0
            && Self::amount(&trigger.data["unblocked"])? > 0)
    }

    fn envenom(&mut self, trigger: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<()> {
        let eligible = Self::eligible(trigger, indexed)?;
        let frame = self
            .frames
            .get(&number(&trigger.data, "source_frame")?)
            .ok_or_else(|| anyhow!("Envenom trigger lacks its earlier raw source frame"))?;
        ensure!(
            frame.model == "ENVENOM_POWER"
                && frame.context == "producer"
                && trigger.data["power_identity"].as_u64() == Some(frame.identity)
                && trigger.data["owner"]["instance"].as_u64() == Some(frame.owner)
                && Self::amount(&trigger.data["amount"])? == frame.amount,
            "Envenom callback identity, owner or amount differs from its frozen raw power"
        );
        let end = Self::trigger_end(trigger, indexed)?;
        let child = indexed.values().any(|event| {
            event.name == "power_attempt"
                && event.data["power"].as_str() == Some("POISON_POWER")
                && event.seq > trigger.seq
                && event.seq < end.seq
                && event.data["cause"].as_u64() == Some(trigger.seq)
        });
        ensure!(
            !eligible || child,
            "Eligible Envenom callback has no observable canonical application child; rejection, no-op and missed capture cannot be distinguished"
        );
        ensure!(
            eligible || !child,
            "Ineligible Envenom callback unexpectedly has an application child"
        );
        ensure!(
            end.data["outcome"].as_str() == Some("completed"),
            "Envenom callback did not complete normally"
        );
        self.add(trigger.seq, Check::Match(format!("Envenom raw damage predicate is {eligible}; canonical application child observed={child}.")));
        Ok(())
    }

    fn trigger_end<'a>(trigger: &Event, indexed: &BTreeMap<u64, &'a Event>) -> Result<&'a Event> {
        let mut ends = indexed.values().filter(|event| {
            event.name == "envenom_end" && event.data["trigger"].as_u64() == Some(trigger.seq)
        });
        let end = ends
            .next()
            .ok_or_else(|| anyhow!("Envenom callback has no recorded completion"))?;
        ensure!(
            ends.next().is_none() && end.seq > trigger.seq,
            "Envenom completion is duplicated or out of order"
        );
        Ok(end)
    }

    fn checkpoint(&self, value: &Value) -> Result<Vec<Check>> {
        let native = Native::read(value)?;
        ensure!(
            native.combat_id == self.epoch,
            "Foreign native combat checkpoint"
        );
        let mut checks = Vec::new();
        for (identity, power) in self
            .powers
            .iter()
            .filter(|(_, power)| power.model == "POISON_POWER")
        {
            let Some(grants) = &power.grants else {
                continue;
            };
            let current = native.poison.get(&power.native_id);
            if power.native_id == 0 || power.native_owner == 0 {
                checks.push(Check::Unverified(format!(
                    "Raw Poison #{identity} lacks comparison identity metadata."
                )));
                continue;
            }
            checks.push(compare(
                current.map_or(0, |value| value.amount) == power.amount,
                format!(
                    "Raw Poison #{identity}: independently reconstructed amount {}, native {}.",
                    power.amount,
                    current.map_or(0, |value| value.amount)
                ),
            ));
            if let Some(current) = current {
                checks.push(compare(current.owner == power.native_owner && current.grants == *grants,
                    format!("Raw Poison #{identity}: expected owner {} and FIFO {:?}; native owner {} and FIFO {:?}.", power.native_owner, grants, current.owner, current.grants)));
                if power.amount > 0 {
                    let expected = power.weights()?;
                    checks.push(compare(current.source == expected, format!("Raw Poison #{identity}: expected supplier weights [{expected}], native [{}].", current.source)));
                }
            } else if power.amount > 0 {
                checks.push(Check::Discrepancy(format!(
                    "Raw Poison #{identity} has {} stacks but no native attachment.",
                    power.amount
                )));
            }
        }
        for instance in native.poison.keys() {
            if !self
                .powers
                .values()
                .any(|power| power.native_id == *instance)
            {
                checks.push(Check::Unverified(format!(
                    "Native Poison #{instance} has no independently observed raw attachment."
                )));
            }
        }
        Ok(checks)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Frozen source, grouped physical damage and measured counter delta are one check."
    )]
    fn tick(&mut self, tick: &Event, indexed: &BTreeMap<u64, &Event>) -> Result<()> {
        let results: Vec<_> = indexed
            .values()
            .filter(|event| {
                event.name == "poison_damage" && event.data["tick_seq"].as_u64() == Some(tick.seq)
            })
            .copied()
            .collect();
        ensure!(
            !results.is_empty(),
            "Poison tick has no completed physical result"
        );
        self.checks
            .entry(tick.seq)
            .or_default()
            .extend(tick.physical(&results));
        let frame_id = number(&tick.data, "source_frame")?;
        ensure!(
            frame_id < tick.seq,
            "Poison tick source frame is not earlier than its use"
        );
        let frame = self
            .frames
            .get(&frame_id)
            .ok_or_else(|| anyhow!("Missing independent Poison source frame"))?;
        ensure!(
            frame.model == "POISON_POWER"
                && frame.context == "damage"
                && tick.data["power_identity"].as_u64() == Some(frame.identity)
                && frame.owner != 0
                && tick.data["target"]["instance"].as_u64() == Some(frame.owner),
            "Poison tick and frozen source frame power or owner identities disagree"
        );
        let weights = frame
            .source
            .as_ref()
            .map_err(|reason| anyhow!(reason.clone()))?;
        let mut damage = 0_u64;
        for (index, result) in results.iter().enumerate() {
            ensure!(
                result.data["target"]["instance"].as_u64() == Some(frame.owner),
                "Redirected or unidentified Poison result is outside independent reconstruction"
            );
            ensure!(
                result.seq > tick.seq
                    && result.data["group_index"].as_u64() == Some(index as u64)
                    && result.data["group_count"].as_u64() == Some(results.len() as u64),
                "Incomplete or reordered Poison result group"
            );
            ensure!(
                result.data["receiver_side"].as_str() == Some("enemy")
                    && result.data["receiver_kind"].as_str() == Some("monster"),
                "Incoming or pet Poison damage is outside outgoing credit reconstruction"
            );
            damage = damage
                .checked_add(Self::amount(&result.data["unblocked"])?)
                .and_then(|sum| sum.checked_add(Self::amount(&result.data["blocked"]).ok()?))
                .ok_or_else(|| anyhow!("Physical Poison damage exceeds supported arithmetic"))?;
        }
        let expected = weights.allocate(damage)?;
        let description = weights.to_string();
        for (source, amount) in &expected {
            let total = self.credits.entry(source.clone()).or_default();
            *total = total
                .checked_add(*amount)
                .ok_or_else(|| anyhow!("Independent Poison subtotal overflow"))?;
        }
        self.add(tick.seq, Check::Match(format!("Independently reconstructed Poison damage {damage}, frozen weights [{description}]; FIFO suppliers earn credit, including Accelerant-triggered ticks.")));
        let before = Native::read(&tick.data["native"])?;
        let after = Native::read(&results.last().expect("nonempty result group").data["native"])?;
        ensure!(
            before.combat_id == self.epoch && after.combat_id == self.epoch,
            "Foreign native tick checkpoint"
        );
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
            let amount = *expected.get(&source).unwrap_or(&0);
            if delta != 0 || amount != 0 {
                self.add(tick.seq, compare(delta == i128::from(amount), format!("Independent tick #{}: {} (kind {}, player {}) expects +{amount}; native observed delta {delta:+}. Interleaved credits require manual review.", tick.seq, source.id, source.kind, source.player)));
            }
        }
        Ok(())
    }

    pub(super) fn summary(&self, text: &mut String) -> Result<()> {
        writeln!(
            text,
            "Version 2: independent Poison reconstruction from raw accepted mutations and frozen source frames. Native supplier queues are comparison claims, never expected-state inputs. Ordinary cards and evidenced Envenom chains are supported; generated or ambiguous origins remain unverifiable. Trigger scheduling and unobserved applications are not proven.\n"
        )?;
        writeln!(
            text,
            "Independent Poison subtotal: **{}**. Full native indirect totals below may also contain other effects and are not an equality target.\n",
            if self.complete {
                "all recorded ticks reconstructed"
            } else {
                "PARTIAL; only reconstructable ticks included"
            }
        )?;
        text.push_str("| Source | Player | Reconstructed Poison damage |\n| --- | ---: | ---: |\n");
        for (source, amount) in &self.credits {
            writeln!(
                text,
                "| {} (kind {}) | {} | {amount} |",
                markdown(&source.id),
                source.kind,
                source.player
            )?;
        }
        text.push('\n');
        Ok(())
    }
}

impl Power {
    fn weights(&self) -> Result<Weights> {
        let grants = self
            .grants
            .as_ref()
            .ok_or_else(|| anyhow!("Power supplier history is tainted"))?;
        ensure!(
            self.amount > 0 && !grants.is_empty(),
            "Drained power has no supported live supplier source"
        );
        let denominators: Vec<_> = grants
            .iter()
            .map(|grant| {
                grant
                    .source
                    .0
                    .iter()
                    .try_fold(0_u128, |total, (_, weight)| total.checked_add(*weight))
                    .ok_or_else(|| anyhow!("Independent supplier denominator overflow"))
            })
            .collect::<Result<_>>()?;
        let common = denominators.iter().try_fold(1_u128, |common, total| {
            (common / gcd(common, *total))
                .checked_mul(*total)
                .ok_or_else(|| anyhow!("Independent mixture denominator overflow"))
        })?;
        let mut entries = Vec::new();
        for (grant, total) in grants.iter().zip(denominators) {
            for (source, weight) in &grant.source.0 {
                let weight = weight
                    .checked_mul(u128::from(grant.remaining))
                    .and_then(|weight| weight.checked_mul(common / total))
                    .ok_or_else(|| anyhow!("Independent mixture weight overflow"))?;
                entries.push((source.clone(), weight));
            }
        }
        Weights::normalize(entries)
    }
}
