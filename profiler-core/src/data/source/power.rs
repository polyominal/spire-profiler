//! Accepted mutations retain supplier grants in application order. Drained
//! attachments keep their last captured source; every grant owns its snapshot.

use super::*;

impl PowerProvenance {
    fn matches_owner(&self, id: &str, owner: u64, kind: CreatureKind) -> bool {
        self.id == id && self.owner == owner && self.owner_kind == kind
    }
}

impl Provenance {
    pub(super) fn reserve_tracking(&mut self) -> Result<(), std::collections::TryReserveError> {
        self.powers.try_init(caps::POWER_INSTANCES, || {
            Ok(PowerProvenance {
                instance: 0,
                id: Default::default(),
                owner: 0,
                owner_kind: CreatureKind::Other,
                owner_slot: TEAM_SLOT,
                observed: 0,
                source: SourceSnapshot::try_new()?,
                trusted: false,
            })
        })?;
        self.grants.try_init(caps::POWER_GRANTS_TOTAL, || {
            Ok(FlatGrant {
                power: 0,
                grant: PowerGrant::try_new()?,
            })
        })?;
        self.generated.try_init(caps::GENERATED_INSTANCES, || {
            Ok(GeneratedSource {
                instance: 0,
                source: SourceSnapshot::try_new()?,
                producer_role: ProducerRole::Unknown,
            })
        })?;
        self.orbs.try_init(caps::ORB_SOURCES, || {
            Ok(InstanceSource {
                instance: 0,
                source: SourceSnapshot::try_new()?,
            })
        })?;
        self.plays.try_init(caps::ACTIVE_PLAYS, || {
            Ok(ActiveSourcePlay {
                serial: 0,
                execution: 0,
                card_instance: 0,
                owner_slot: TEAM_SLOT,
                source: SourceSnapshot::try_new()?,
                first_orb_used: false,
            })
        })?;
        self.reductions.try_init(caps::STR_REDUCTIONS, || {
            Ok(StrengthReduction {
                power_instance: 0,
                creature: 0,
                amount: 0,
                source: SourceSnapshot::try_new()?,
            })
        })?;
        self.incoming = SourceSnapshot::try_new()?;
        self.mixture_scratch = snapshot::SnapshotScratch::try_new()?;
        Ok(())
    }

    pub(super) fn clear_tracking(&mut self) {
        self.powers.clear();
        self.grants.clear();
        self.generated.clear();
        self.orbs.clear();
        self.plays.clear();
        self.reductions.clear();
        self.play_serial = 0;
        self.incoming.clear();
    }

    fn consume_power(&mut self, power: usize, mut remaining: u32) {
        let mut index = 0;
        while remaining > 0 && index < self.grants.len() {
            if self.grants[index].power != power {
                index += 1;
                continue;
            }
            remaining -= self.grants[index].grant.consume(remaining);
            if self.grants[index].grant.remaining() == 0 {
                self.grants.remove(index);
            } else {
                index += 1;
            }
        }
    }

    fn reset_power_grants(&mut self, power: usize, amount: u32, epoch: CombatEpoch) {
        self.grants.retain(|entry| entry.power != power);
        self.powers[power].source.set_unknown(epoch);
        if amount > 0 {
            let entry = self
                .grants
                .vacant_mut()
                .expect("the global grant reserve admits one Unknown for every power");
            entry.power = power;
            entry.grant.reset_from(amount, &self.powers[power].source);
            self.grants.activate();
        }
    }

    fn refresh_power_source(
        &mut self,
        power: usize,
        epoch: CombatEpoch,
        rows: usize,
        diagnostics: &mut SourceDiagnostics,
    ) {
        if !self.grants.iter().any(|entry| entry.power == power) {
            return;
        }
        self.powers[power].source.mixture_into(
            epoch,
            rows,
            self.grants
                .iter()
                .filter(|entry| entry.power == power)
                .map(|entry| (entry.grant.remaining(), entry.grant.source())),
            &mut self.mixture_scratch,
            diagnostics,
        );
        if self.powers[power].source.is_unknown()
            && self
                .grants
                .iter()
                .any(|entry| entry.power == power && !entry.grant.source().is_unknown())
        {
            let remaining = self
                .grants
                .iter()
                .filter(|entry| entry.power == power)
                .map(|entry| entry.grant.remaining())
                .sum();
            self.reset_power_grants(power, remaining, epoch);
        }
    }
}

impl State {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn power_attached(
        &mut self,
        combat_seq: u64,
        instance: u64,
        id: &str,
        owner: u64,
        owner_kind: i32,
        owner_slot: i32,
        amount: i32,
        transfer: u64,
    ) -> i32 {
        let result = self.power_observed(
            combat_seq, instance, id, owner, owner_kind, owner_slot, None, amount, transfer,
        );
        if result.is_err() {
            self.power_provenance_invalidate(combat_seq, instance);
        }
        self.source_status(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn power_amount_changed(
        &mut self,
        combat_seq: u64,
        instance: u64,
        id: &str,
        owner: u64,
        owner_kind: i32,
        owner_slot: i32,
        old_amount: i32,
        new_amount: i32,
        transfer: u64,
    ) -> i32 {
        let result = self.power_observed(
            combat_seq,
            instance,
            id,
            owner,
            owner_kind,
            owner_slot,
            Some(old_amount),
            new_amount,
            transfer,
        );
        if result.is_err() {
            self.power_provenance_invalidate(combat_seq, instance);
        }
        self.source_status(result)
    }

    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "keep all rejection preflight before publishing grants and reduction balances"
    )]
    fn power_observed(
        &mut self,
        combat_seq: u64,
        instance: u64,
        id: &str,
        owner: u64,
        owner_kind: i32,
        owner_slot: i32,
        old: Option<i32>,
        new: i32,
        transfer: u64,
    ) -> Result<(), SourceFailure> {
        let epoch = self.provenance_epoch(combat_seq)?;
        if instance == 0 || owner == 0 || id.is_empty() {
            return Err(SourceFailure::Packet);
        }
        let bounded_id =
            super::super::state::ModelId::try_from(id).map_err(|_| SourceFailure::Packet)?;
        let kind = CreatureKind::decode(owner_kind, &mut self.source_transfers.diagnostics);
        let slot = super::super::state::clamp_source_slot(owner_slot);
        let index = self
            .provenance
            .powers
            .iter()
            .position(|power| power.instance == instance);
        let before = old.unwrap_or(0);
        let reset = if let Some(index) = index {
            let power = &self.provenance.powers[index];
            if old.is_none() || !power.matches_owner(id, owner, kind) || power.owner_slot != slot {
                return Err(SourceFailure::Packet);
            }
            !power.trusted || power.observed != before
        } else {
            if self.provenance.powers.len() == caps::POWER_INSTANCES {
                return Err(SourceFailure::Capacity);
            }
            old.is_some()
        };
        self.capture_source_snapshot(combat_seq, transfer)?;
        let strength = id == "STRENGTH_POWER" && kind == CreatureKind::Enemy;
        let delta = i64::from(new) - i64::from(before);
        let reduction_amount = if strength && delta < 0 {
            let reductions = &self.provenance.reductions;
            let matched = reductions.iter().find(|entry| {
                !reset
                    && entry.power_instance == instance
                    && entry.creature == owner
                    && entry.source == self.provenance.incoming
            });
            if let Some(entry) = matched {
                Some(
                    entry
                        .amount
                        .checked_add(delta.unsigned_abs())
                        .ok_or(SourceFailure::Arithmetic)?,
                )
            } else {
                let retained = reductions
                    .iter()
                    .filter(|entry| !reset || entry.power_instance != instance)
                    .count();
                if retained == caps::STR_REDUCTIONS {
                    return Err(SourceFailure::Capacity);
                }
                Some(delta.unsigned_abs())
            }
        } else {
            None
        };

        // Every fallible admission is settled before changing the accepted balance.
        let power = if let Some(index) = index {
            index
        } else {
            let index = self.provenance.powers.len();
            let power = self
                .provenance
                .powers
                .vacant_mut()
                .expect("power admission leaves one initialized slot");
            power.instance = instance;
            power.id = bounded_id;
            power.owner = owner;
            power.owner_kind = kind;
            power.owner_slot = slot;
            power.observed = before;
            power.trusted = old.is_none();
            power.source.clone_from(&self.provenance.incoming);
            self.provenance.powers.activate();
            index
        };
        if reset {
            self.provenance
                .reset_power_grants(power, before.max(0) as u32, epoch);
            self.source_transfers
                .diagnostics
                .report(SourceFailure::Packet);
        }
        self.update_power_grants(power, before, new, epoch);
        if strength {
            if reset {
                self.provenance
                    .reductions
                    .retain(|entry| entry.power_instance != instance);
            }
            if let Some(amount) = reduction_amount {
                if let Some(entry) = self.provenance.reductions.iter_mut().find(|entry| {
                    entry.power_instance == instance
                        && entry.creature == owner
                        && entry.source == self.provenance.incoming
                }) {
                    entry.amount = amount;
                } else {
                    let entry = self
                        .provenance
                        .reductions
                        .vacant_mut()
                        .expect("reduction admission is validated before mutation");
                    entry.power_instance = instance;
                    entry.creature = owner;
                    entry.amount = amount;
                    entry.source.clone_from(&self.provenance.incoming);
                    self.provenance.reductions.activate();
                }
            } else {
                let mut remaining = delta as u64;
                let mut index = self.provenance.reductions.len();
                while remaining > 0 && index > 0 {
                    index -= 1;
                    if self.provenance.reductions[index].creature != owner {
                        continue;
                    }
                    let take = remaining.min(self.provenance.reductions[index].amount);
                    self.provenance.reductions[index].amount -= take;
                    remaining -= take;
                    if self.provenance.reductions[index].amount == 0 {
                        self.provenance.reductions.remove(index);
                    }
                }
            }
        }
        self.provenance.powers[power].observed = new;
        self.provenance.powers[power].trusted = true;
        Ok(())
    }

    fn update_power_grants(&mut self, power: usize, before: i32, after: i32, epoch: CombatEpoch) {
        let previous = before.max(0) as u32;
        let current = after.max(0) as u32;
        if current < previous {
            self.provenance.consume_power(power, previous - current);
        } else if current > previous {
            let amount = current - previous;
            let last = self
                .provenance
                .grants
                .iter()
                .rposition(|entry| entry.power == power);
            if let Some(last) = last.filter(|index| {
                self.provenance.grants[*index].grant.source() == &self.provenance.incoming
            }) {
                self.provenance.grants[last]
                    .grant
                    .add(amount)
                    .expect("one grant cannot exceed the positive observed i32 balance");
            } else {
                let count = self
                    .provenance
                    .grants
                    .iter()
                    .filter(|entry| entry.power == power)
                    .count();
                let occupied = self
                    .provenance
                    .powers
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        self.provenance
                            .grants
                            .iter()
                            .any(|entry| entry.power == *index)
                    })
                    .count();
                let extras = self.provenance.grants.len() - occupied;
                if count == caps::POWER_GRANTS_PER_INSTANCE
                    || (count > 0 && extras == caps::POWER_GRANTS_TOTAL - caps::POWER_INSTANCES)
                {
                    self.provenance.reset_power_grants(power, current, epoch);
                    self.source_transfers
                        .diagnostics
                        .report(SourceFailure::Capacity);
                } else {
                    let entry = self
                        .provenance
                        .grants
                        .vacant_mut()
                        .expect("global extra-grant admission preserves one slot per power");
                    entry.power = power;
                    entry.grant.reset_from(amount, &self.provenance.incoming);
                    self.provenance.grants.activate();
                }
            }
        }
        let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
        self.provenance.refresh_power_source(
            power,
            epoch,
            rows,
            &mut self.source_transfers.diagnostics,
        );
    }

    pub(crate) fn power_provenance_invalidate(&mut self, combat_seq: u64, instance: u64) -> i32 {
        let result = self.provenance_epoch(combat_seq).map(|_| {
            if let Some(power) = self
                .provenance
                .powers
                .iter_mut()
                .find(|power| power.instance == instance)
            {
                power.trusted = false;
            }
            self.provenance
                .reductions
                .retain(|entry| entry.power_instance != instance);
        });
        self.source_status(result)
    }

    pub(crate) fn power_removed(&mut self, combat_seq: u64, instance: u64) -> i32 {
        let result = self.provenance_epoch(combat_seq).map(|_| {
            if let Some(index) = self
                .provenance
                .powers
                .iter()
                .position(|power| power.instance == instance)
            {
                self.provenance.grants.retain(|entry| entry.power != index);
                for entry in self
                    .provenance
                    .grants
                    .iter_mut()
                    .filter(|entry| entry.power > index)
                {
                    entry.power -= 1;
                }
                self.provenance.powers.remove(index);
            }
            self.provenance
                .reductions
                .retain(|entry| entry.power_instance != instance);
        });
        self.source_status(result)
    }

    pub(crate) fn doom_batch_begin(&mut self, combat_seq: u64) -> u64 {
        let Ok(epoch) = self.provenance_epoch(combat_seq) else {
            return 0;
        };
        if self.provenance.doom_batches.len() == caps::DOOM_BATCHES
            || self.provenance.doom_serial == PAYLOAD_MAX
        {
            self.source_transfers
                .diagnostics
                .report(SourceFailure::Capacity);
            return 0;
        }
        let serial = self.provenance.doom_serial + 1;
        self.provenance.doom_batches.push(DoomBatch {
            serial,
            targets: Vec::new(),
            complete: true,
        });
        self.provenance.doom_serial = serial;
        Token {
            epoch,
            kind: TokenKind::DoomBatch,
            payload: serial,
        }
        .encode()
    }

    pub(crate) fn doom_target_capture(
        &mut self,
        batch: u64,
        creature: u64,
        instance: u64,
        hp: i32,
    ) -> i32 {
        let result = (|| {
            let token = self.provenance_token(batch, TokenKind::DoomBatch)?;
            let index = self
                .provenance
                .doom_batches
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            if !self.provenance.doom_batches[index].complete {
                return Err(SourceFailure::Packet);
            }
            self.provenance.doom_batches[index].complete = false;
            if creature == 0 || hp < 0 {
                return Err(SourceFailure::Packet);
            }
            let targets = &self.provenance.doom_batches[index].targets;
            if targets.len() == caps::DOOM_TARGETS
                || targets.iter().any(|target| target.creature == creature)
            {
                return Err(SourceFailure::Capacity);
            }
            let mut remaining = hp as u64;
            let mut allocations = Vec::new();
            if let Some(power) = self.provenance.powers.iter().position(|power| {
                power.instance == instance
                    && power.matches_owner("DOOM_POWER", creature, CreatureKind::Enemy)
                    && power.trusted
            }) {
                for entry in self
                    .provenance
                    .grants
                    .iter()
                    .filter(|entry| entry.power == power)
                {
                    let grant = &entry.grant;
                    let take = remaining.min(u64::from(grant.remaining()));
                    allocations.extend(grant.source().budgets(take).take(take)?);
                    remaining -= take;
                    if remaining == 0 {
                        break;
                    }
                }
            }
            let debit = (hp as u64 - remaining) as u32;
            if remaining > 0 {
                allocations.push((Destination::Unknown(TEAM_SLOT), remaining));
            }
            let batch = &mut self.provenance.doom_batches[index];
            batch.targets.push(DoomCapture {
                creature,
                power_instance: instance,
                debit,
                allocations,
            });
            batch.complete = true;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn doom_kills_completed(&mut self, batch: u64) -> i32 {
        let result = (|| {
            let token = self.provenance_token(batch, TokenKind::DoomBatch)?;
            let index = self
                .provenance
                .doom_batches
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            let batch = self.provenance.doom_batches.remove(index);
            if !batch.complete {
                return Err(SourceFailure::Packet);
            }
            let mut stage = LedgerStage::new(self)?;
            for target in &batch.targets {
                for (destination, amount) in &target.allocations {
                    stage.damage(*destination, DamageSegment::Attributed, *amount, 0)?;
                }
            }
            stage.commit(self)?;
            let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
            for target in batch.targets {
                if let Some(power) = self.provenance.powers.iter().position(|power| {
                    power.instance == target.power_instance && power.owner == target.creature
                }) {
                    self.provenance.consume_power(power, target.debit);
                    self.provenance.refresh_power_source(
                        power,
                        token.epoch,
                        rows,
                        &mut self.source_transfers.diagnostics,
                    );
                }
            }
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn doom_batch_abort(&mut self, batch: u64) -> i32 {
        let result = (|| {
            let token = self.provenance_token(batch, TokenKind::DoomBatch)?;
            let index = self
                .provenance
                .doom_batches
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            self.provenance.doom_batches.remove(index);
            Ok(())
        })();
        self.source_status(result)
    }
}

#[cfg(test)]
#[path = "tracking_tests.rs"]
mod tracking_tests;
