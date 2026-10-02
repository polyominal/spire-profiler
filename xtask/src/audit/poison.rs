//! Expected suppliers come exclusively from raw source frames and accepted game
//! mutations. Native snapshots enter only comparisons, never the reconstructed
//! queue. Unsupported provenance remains unknown until that attachment drains.

use super::event::{
    Attempt, CommandEnd, Envenom, EnvenomEnd, Mutation, RawSource, SourceFrame, Tick,
};
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
            epoch: trace.events.first().and_then(Event::epoch).unwrap_or(0),
        };
        let mut sequence = 0_u64;
        let indexed: BTreeMap<_, _> = trace
            .events
            .iter()
            .take_while(|event| {
                let continuous = sequence.checked_add(1) == Some(event.seq)
                    && !matches!(event.name(), "diagnostic" | "trace_truncated");
                sequence = event.seq;
                continuous
            })
            .map(|event| (event.seq, event))
            .collect();
        let mut previous = 0_u64;
        let mut interrupted = result.epoch == 0;
        for event in &trace.events {
            if previous.checked_add(1) != Some(event.seq)
                || matches!(event.name(), "diagnostic" | "trace_truncated")
            {
                interrupted = true;
            }
            previous = event.seq;
            if interrupted {
                result.complete = false;
                result.add(event.seq, Check::Unverified("Raw event continuity is unavailable; subsequent independent reconstruction is withheld.".into()));
                continue;
            }
            let outcome = match &event.data {
                Data::Frame(data) => result.frame(event, data),
                Data::Change(data) => result.change(event, data, &indexed),
                Data::Attempt(data) => result.command(event, data, &indexed),
                Data::Envenom(data) => result.envenom(event, data, &indexed),
                Data::Tick(data) => result.tick(event, data, &indexed),
                Data::PoisonDamage(data) => {
                    if indexed.get(&data.tick_seq).is_some_and(|tick| {
                        matches!(tick.data, Data::Tick(_)) && tick.seq < event.seq
                    }) {
                        Ok(())
                    } else {
                        Err(anyhow!("Physical Poison result lacks a preceding raw tick"))
                    }
                }
                _ => Ok(()),
            };
            if let Err(error) = outcome {
                result.complete = false;
                if event.name() == "power_change" {
                    for power in result.powers.values_mut() {
                        power.grants = None;
                    }
                }
                result.add(event.seq, Check::Unverified(error.to_string()));
            }
            if event.native.is_some() {
                match event
                    .checkpoint()
                    .and_then(|native| result.checkpoint(native))
                {
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

    fn frame(&mut self, event: &Event, data: &SourceFrame) -> Result<()> {
        let raw = &data.source;
        let identity = raw.identity;
        let model = raw.model.clone();
        let amount = data.amount.nonnegative()?;
        let context = data.context.clone();
        let source = match raw.role.as_str() {
            "card" => Self::card(raw),
            "power" => (|| {
                ensure!(
                    matches!(model.as_str(), "POISON_POWER" | "ENVENOM_POWER"),
                    "Unsupported source power {model}"
                );
                ensure!(
                    data.power_identity == identity && identity != 0,
                    "Source frame power identity disagrees with raw source"
                );
                let power = self.powers.get(&identity).ok_or_else(|| {
                    anyhow!("No independently observed attachment for source power {identity}")
                })?;
                ensure!(
                    power.model == model && Some(power.owner) == Creature::instance(&data.target),
                    "Source frame power ownership differs from its observed attachment"
                );
                ensure!(
                    power.amount == amount,
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
                owner: Creature::instance(&data.target).unwrap_or(0),
                amount,
                context,
                source,
            },
        );
        Ok(())
    }

    fn card(raw: &RawSource) -> Result<Weights> {
        ensure!(
            raw.origin == "ordinary",
            "Generated or unproven card origin is outside independent reconstruction"
        );
        ensure!(raw.identity != 0, "Card identity is missing");
        let player = raw.player;
        ensure!(player < 4, "Ordinary card owner is not a player");
        let id = raw.model.clone();
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

    #[allow(
        clippy::too_many_lines,
        reason = "Command, frame and Envenom identity checks establish one causal source."
    )]
    fn source(
        &self,
        event: &Event,
        data: &Mutation,
        indexed: &BTreeMap<u64, &Event>,
    ) -> Result<Weights> {
        let action = data
            .action
            .ok_or_else(|| anyhow!("Missing action identity"))?;
        let (attempt, attempt_data) = indexed
            .get(&action)
            .and_then(|event| {
                if let Data::Attempt(data) = &event.data {
                    Some((*event, data))
                } else {
                    None
                }
            })
            .filter(|(attempt, _)| attempt.seq < event.seq)
            .ok_or_else(|| anyhow!("Positive mutation has no preceding raw command attempt"))?;
        ensure!(
            data.power.as_deref() == Some(attempt_data.power.as_str())
                && Creature::instance(&attempt_data.target).is_some()
                && Creature::instance(&attempt_data.target) == Creature::instance(&data.target),
            "Mutation and attempt raw power model or target differ"
        );
        let (end, completion) = Self::command_end(attempt, attempt_data, indexed)?;
        ensure!(
            end.seq > event.seq && Some(completion.power_identity) == data.power_identity,
            "Command completion does not bind the actual mutated power identity"
        );
        let frame_id = data
            .source_frame
            .ok_or_else(|| anyhow!("Missing source frame identity"))?;
        ensure!(
            frame_id != 0 && frame_id < attempt.seq && attempt_data.source_frame == frame_id,
            "Mutation source does not match its command's frozen raw frame"
        );
        let frame = self
            .frames
            .get(&frame_id)
            .ok_or_else(|| anyhow!("Missing frozen source frame #{frame_id}"))?;
        ensure!(
            attempt_data.source.identity == frame.identity
                && attempt_data.source.model == frame.model,
            "Command's raw source differs from the frozen source frame"
        );
        if frame.model == "ENVENOM_POWER" {
            ensure!(
                frame.context == "producer",
                "Envenom supplier frame was not sampled at producer entry"
            );
            let (trigger, trigger_data) = indexed
                .get(&attempt_data.cause)
                .and_then(|event| {
                    if let Data::Envenom(data) = &event.data {
                        Some((*event, data))
                    } else {
                        None
                    }
                })
                .filter(|(trigger, _)| trigger.seq < attempt.seq)
                .ok_or_else(|| anyhow!("Envenom application lacks a preceding trigger"))?;
            ensure!(
                trigger_data.source_frame == frame_id
                    && trigger_data.power_identity == frame.identity
                    && Creature::instance(&trigger_data.target) == Creature::instance(&data.target),
                "Envenom trigger does not bind this source frame and receiver"
            );
            ensure!(
                Self::eligible(trigger, trigger_data, indexed)?,
                "Raw Envenom callback is not eligible to apply Poison"
            );
            ensure!(
                Creature::instance(&trigger_data.owner) == Some(frame.owner),
                "Envenom callback owner differs from its frozen power owner"
            );
            let amount = trigger_data.amount.nonnegative()?;
            ensure!(
                attempt_data.amount.nonnegative()? == amount,
                "Envenom command request differs from its observed trigger amount"
            );
            ensure!(
                amount == frame.amount && data.power.as_deref() == Some("POISON_POWER"),
                "Envenom child amount or power differs from the independently observed producer"
            );
            ensure!(
                Self::trigger_end(trigger, indexed)?.0.seq > event.seq,
                "Envenom child mutation is outside its callback lifetime"
            );
        }
        frame.source.clone().map_err(|reason| anyhow!(reason))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Physical attachment transitions and provenance update form one transaction."
    )]
    fn change(
        &mut self,
        event: &Event,
        data: &Mutation,
        indexed: &BTreeMap<u64, &Event>,
    ) -> Result<()> {
        let model = data
            .power
            .clone()
            .ok_or_else(|| anyhow!("Missing power model"))?;
        ensure!(
            matches!(model.as_str(), "POISON_POWER" | "ENVENOM_POWER"),
            "Unsupported power mutation {model}"
        );
        let identity = data
            .power_identity
            .ok_or_else(|| anyhow!("Raw mutation identity missing"))?;
        let owner = Creature::instance(&data.target)
            .ok_or_else(|| anyhow!("Raw mutation owner missing"))?;
        ensure!(identity != 0 && owner != 0, "Raw mutation identity missing");
        let before_attached = data
            .before_attached
            .ok_or_else(|| anyhow!("Missing prior attachment state"))?;
        let attached = data.after_attached;
        let before = if before_attached {
            data.before.nonnegative()?
        } else {
            0
        };
        let after = if attached {
            data.after.nonnegative()?
        } else {
            0
        };
        let source = if after > before {
            Some(self.source(event, data, indexed))
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
        power.native_id = data.power_instance.unwrap_or(0);
        power.native_owner = Creature::native(&data.target).unwrap_or(0);
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

    fn command_end<'a>(
        attempt: &Event,
        data: &Attempt,
        indexed: &BTreeMap<u64, &'a Event>,
    ) -> Result<(&'a Event, &'a CommandEnd)> {
        let mut ends = indexed.values().filter_map(|event| {
            if let Data::CommandEnd(end) = &event.data
                && end.action == attempt.seq
            {
                Some((*event, end))
            } else {
                None
            }
        });
        let (end, completion) = ends
            .next()
            .ok_or_else(|| anyhow!("Command #{} has no recorded completion", attempt.seq))?;
        ensure!(
            ends.next().is_none() && end.seq > attempt.seq,
            "Command completion is duplicated or out of order"
        );
        ensure!(
            completion.command == data.command
                && Creature::instance(&data.target).is_some()
                && Creature::instance(&completion.target) == Creature::instance(&data.target)
                && completion.requested_power_identity == data.power_identity,
            "Command completion has a different raw command, requested power or target"
        );
        ensure!(
            matches!(
                completion.outcome.as_str(),
                "completed" | "faulted" | "cancelled"
            ),
            "Missing command outcome"
        );
        Ok((end, completion))
    }

    fn command(
        &mut self,
        attempt: &Event,
        data: &Attempt,
        indexed: &BTreeMap<u64, &Event>,
    ) -> Result<()> {
        let (end, completion) = Self::command_end(attempt, data, indexed)?;
        let count = indexed.values().filter(|event| {
            matches!(&event.data, Data::Change(data) if data.action == Some(attempt.seq))
        }).count();
        self.add(attempt.seq, Check::Match(format!("Raw command ends {} at #{} with {count} observed mutations. Completion alone does not establish acceptance or explain a no-op.", completion.outcome, end.seq)));
        Ok(())
    }

    fn eligible(trigger: &Event, data: &Envenom, indexed: &BTreeMap<u64, &Event>) -> Result<bool> {
        let result_id = data.result_identity;
        ensure!(
            result_id != 0,
            "Envenom trigger lacks a raw damage result identity"
        );
        let mut candidates = indexed.values().filter_map(|event| {
            event
                .damage_data()
                .filter(|result| result.result_identity == Some(result_id))
                .map(|result| (*event, result))
        });
        let (result, result_data) = candidates
            .next()
            .ok_or_else(|| anyhow!("No raw damage result for Envenom trigger"))?;
        ensure!(
            candidates.next().is_none() && result.seq < trigger.seq,
            "Envenom raw result identity is duplicated or ordered incorrectly"
        );
        ensure!(
            Creature::instance(&data.target).is_some()
                && Creature::instance(&result_data.target) == Creature::instance(&data.target)
                && result_data.unblocked.nonnegative()? == data.unblocked.nonnegative()?,
            "Envenom receiver or damage differs from its raw result"
        );
        let begin = indexed
            .get(&result_data.tick_seq)
            .ok_or_else(|| anyhow!("No raw damage begin for Envenom result"))?;
        ensure!(
            begin.seq < result.seq,
            "Envenom lacks a completed raw damage observation"
        );
        let (dealer, props) = match &begin.data {
            Data::DamageBegin(data) => (&data.dealer, Some(data.props)),
            Data::Tick(data) => (&data.dealer, data.props),
            _ => return Err(anyhow!("Envenom lacks a completed raw damage observation")),
        };
        let owner =
            Creature::instance(&data.owner).ok_or_else(|| anyhow!("Envenom owner is missing"))?;
        ensure!(
            Creature::instance(&data.dealer) == Creature::instance(dealer),
            "Envenom owner is missing or damage dealer differs from raw begin"
        );
        ensure!(
            props == Some(data.props),
            "Envenom damage properties differ from the raw begin"
        );
        Ok(Creature::instance(&data.dealer) == Some(owner)
            && data.props & 8 != 0
            && data.props & 4 == 0
            && data.unblocked.nonnegative()? > 0)
    }

    fn envenom(
        &mut self,
        trigger: &Event,
        data: &Envenom,
        indexed: &BTreeMap<u64, &Event>,
    ) -> Result<()> {
        let eligible = Self::eligible(trigger, data, indexed)?;
        let frame = self
            .frames
            .get(&data.source_frame)
            .ok_or_else(|| anyhow!("Envenom trigger lacks its earlier raw source frame"))?;
        ensure!(
            frame.model == "ENVENOM_POWER"
                && frame.context == "producer"
                && data.power_identity == frame.identity
                && Creature::instance(&data.owner) == Some(frame.owner)
                && data.amount.nonnegative()? == frame.amount,
            "Envenom callback identity, owner or amount differs from its frozen raw power"
        );
        let (end, completion) = Self::trigger_end(trigger, indexed)?;
        let child = indexed.values().any(|event| {
            matches!(&event.data, Data::Attempt(data) if data.power == "POISON_POWER" && data.cause == trigger.seq)
                && event.seq > trigger.seq && event.seq < end.seq
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
            completion.outcome == "completed",
            "Envenom callback did not complete normally"
        );
        self.add(trigger.seq, Check::Match(format!("Envenom raw damage predicate is {eligible}; canonical application child observed={child}.")));
        Ok(())
    }

    fn trigger_end<'a>(
        trigger: &Event,
        indexed: &BTreeMap<u64, &'a Event>,
    ) -> Result<(&'a Event, &'a EnvenomEnd)> {
        let mut ends = indexed.values().filter_map(|event| {
            if let Data::EnvenomEnd(data) = &event.data
                && data.trigger == trigger.seq
            {
                Some((*event, data))
            } else {
                None
            }
        });
        let (end, data) = ends
            .next()
            .ok_or_else(|| anyhow!("Envenom callback has no recorded completion"))?;
        ensure!(
            ends.next().is_none() && end.seq > trigger.seq,
            "Envenom completion is duplicated or out of order"
        );
        Ok((end, data))
    }

    fn checkpoint(&self, native: &Native) -> Result<Vec<Check>> {
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
    fn tick(&mut self, tick: &Event, data: &Tick, indexed: &BTreeMap<u64, &Event>) -> Result<()> {
        let results: Vec<_> = indexed
            .values()
            .filter_map(|event| {
                if let Data::PoisonDamage(data) = &event.data
                    && data.tick_seq == tick.seq
                {
                    Some((*event, data))
                } else {
                    None
                }
            })
            .collect();
        ensure!(
            !results.is_empty(),
            "Poison tick has no completed physical result"
        );
        self.checks
            .entry(tick.seq)
            .or_default()
            .extend(tick.physical(data, &results));
        let frame_id = data
            .source_frame
            .ok_or_else(|| anyhow!("Missing source frame identity"))?;
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
                && data.power_identity == Some(frame.identity)
                && frame.owner != 0
                && Creature::instance(&data.target) == Some(frame.owner),
            "Poison tick and frozen source frame power or owner identities disagree"
        );
        let weights = frame
            .source
            .as_ref()
            .map_err(|reason| anyhow!(reason.clone()))?;
        let mut damage = 0_u64;
        for (index, (result, data)) in results.iter().enumerate() {
            ensure!(
                Creature::instance(&data.target) == Some(frame.owner),
                "Redirected or unidentified Poison result is outside independent reconstruction"
            );
            ensure!(
                result.seq > tick.seq
                    && data.group_index == Some(index as u64)
                    && data.group_count == Some(results.len() as u64),
                "Incomplete or reordered Poison result group"
            );
            ensure!(
                data.receiver_side.as_deref() == Some("enemy")
                    && data.receiver_kind.as_deref() == Some("monster"),
                "Incoming or pet Poison damage is outside outgoing credit reconstruction"
            );
            damage = damage
                .checked_add(data.unblocked.nonnegative()?)
                .and_then(|sum| sum.checked_add(data.blocked.nonnegative().ok()?))
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
        let before = tick.checkpoint()?;
        let after = results
            .last()
            .expect("nonempty result group")
            .0
            .checkpoint()?;
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
