//! Accepted mutations of actual power instances retain complete supplier grants.
//! Decreases consume FIFO; a drained attachment keeps its last captured source.

use super::*;

impl PowerProvenance {
    fn matches_owner(&self, id: &str, owner: u64, kind: CreatureKind) -> bool {
        self.id == id && self.owner == owner && self.owner_kind == kind
    }

    fn consume(&mut self, amount: u32) {
        let mut remaining = amount;
        while remaining > 0 && !self.grants.is_empty() {
            let take = self.grants[0].consume(remaining);
            remaining -= take;
            if self.grants[0].remaining() == 0 {
                self.grants.remove(0);
            }
        }
    }
    fn refresh_source(
        &mut self,
        epoch: CombatEpoch,
        rows: usize,
        diagnostics: &mut SourceDiagnostics,
    ) {
        if self.grants.is_empty() {
            return;
        }
        let mixed = SourceSnapshot::mixture(epoch, rows, &self.grants, diagnostics);
        if mixed == SourceSnapshot::unknown(epoch)
            && self.grants.iter().any(|grant| grant.source() != &mixed)
        {
            let remaining = self.grants.iter().map(|grant| grant.remaining()).sum();
            self.grants = vec![PowerGrant::new(remaining, mixed.clone())];
        }
        self.source = mixed;
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

    #[allow(clippy::too_many_arguments)]
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
        let incoming = self.source_snapshot(combat_seq, transfer)?;
        let powers = &self.provenance.powers;
        let index = powers.iter().position(|power| power.instance == instance);
        let mut power = if let Some(index) = index {
            let power = &powers[index];
            if old.is_none() || !power.matches_owner(id, owner, kind) || power.owner_slot != slot {
                return Err(SourceFailure::Packet);
            }
            power.clone()
        } else {
            if powers.len() == caps::POWER_INSTANCES {
                return Err(SourceFailure::Capacity);
            }
            PowerProvenance {
                instance,
                id: bounded_id,
                owner,
                owner_kind: kind,
                owner_slot: slot,
                observed: old.unwrap_or(0),
                grants: Vec::new(),
                source: incoming.clone(),
                trusted: old.is_none(),
            }
        };
        let before = old.unwrap_or(0);
        if !power.trusted || power.observed != before {
            power.grants.clear();
            power.source = SourceSnapshot::unknown(epoch);
            if before > 0 {
                power
                    .grants
                    .push(PowerGrant::new(before as u32, power.source.clone()));
            }
            self.source_transfers
                .diagnostics
                .report(SourceFailure::Packet);
        }
        self.update_power_grants(&mut power, before, new, incoming.clone(), epoch)?;
        let mut reductions = self.provenance.reductions.clone();
        if id == "STRENGTH_POWER" && kind == CreatureKind::Enemy {
            if !power.trusted || power.observed != before {
                reductions.retain(|entry| entry.power_instance != instance);
            }
            Self::strength_transition(
                &mut reductions,
                instance,
                owner,
                i64::from(new) - i64::from(before),
                incoming,
            )?;
        }
        power.observed = new;
        power.trusted = true;
        if let Some(index) = index {
            self.provenance.powers[index] = power;
        } else {
            self.provenance.powers.push(power);
        }
        self.provenance.reductions = reductions;
        Ok(())
    }

    fn update_power_grants(
        &mut self,
        power: &mut PowerProvenance,
        before: i32,
        after: i32,
        incoming: SourceSnapshot,
        epoch: CombatEpoch,
    ) -> Result<(), SourceFailure> {
        let previous = before.max(0) as u32;
        let current = after.max(0) as u32;
        if current < previous {
            power.consume(previous - current);
        } else if current > previous {
            let amount = current - previous;
            if let Some(last) = power
                .grants
                .last_mut()
                .filter(|last| last.source() == &incoming)
            {
                last.add(amount)?;
            } else {
                power.grants.push(PowerGrant::new(amount, incoming));
            }
        }
        let others = self
            .provenance
            .powers
            .iter()
            .filter(|entry| entry.instance != power.instance)
            .map(|entry| entry.grants.len().saturating_sub(1))
            .sum::<usize>();
        if power.grants.len() > caps::POWER_GRANTS_PER_INSTANCE
            || others + power.grants.len().saturating_sub(1)
                > caps::POWER_GRANTS_TOTAL - caps::POWER_INSTANCES
        {
            power.grants = vec![PowerGrant::new(current, SourceSnapshot::unknown(epoch))];
            self.source_transfers
                .diagnostics
                .report(SourceFailure::Capacity);
        }
        let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
        power.refresh_source(epoch, rows, &mut self.source_transfers.diagnostics);
        Ok(())
    }

    fn strength_transition(
        reductions: &mut Vec<StrengthReduction>,
        instance: u64,
        creature: u64,
        delta: i64,
        source: SourceSnapshot,
    ) -> Result<(), SourceFailure> {
        if delta < 0 {
            let amount = delta.unsigned_abs();
            if let Some(last) = reductions.iter_mut().find(|entry| {
                entry.power_instance == instance
                    && entry.creature == creature
                    && entry.source == source
            }) {
                last.amount = last
                    .amount
                    .checked_add(amount)
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                if reductions.len() == caps::STR_REDUCTIONS {
                    return Err(SourceFailure::Capacity);
                }
                reductions.push(StrengthReduction {
                    power_instance: instance,
                    creature,
                    amount,
                    source,
                });
            }
        } else {
            let mut remaining = delta as u64;
            let mut index = reductions.len();
            while remaining > 0 && index > 0 {
                index -= 1;
                if reductions[index].creature != creature {
                    continue;
                }
                let take = remaining.min(reductions[index].amount);
                reductions[index].amount -= take;
                remaining -= take;
                if reductions[index].amount == 0 {
                    reductions.remove(index);
                }
            }
        }
        Ok(())
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
            self.provenance
                .powers
                .retain(|power| power.instance != instance);
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
            if let Some(power) = self.provenance.powers.iter().find(|power| {
                power.instance == instance
                    && power.matches_owner("DOOM_POWER", creature, CreatureKind::Enemy)
                    && power.trusted
            }) {
                for grant in &power.grants {
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
                if let Some(power) = self.provenance.powers.iter_mut().find(|power| {
                    power.instance == target.power_instance && power.owner == target.creature
                }) {
                    power.consume(target.debit);
                    power.refresh_source(token.epoch, rows, &mut self.source_transfers.diagnostics);
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
