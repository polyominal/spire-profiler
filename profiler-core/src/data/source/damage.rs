//! Complete target-local result groups stage every ledger and defensive-pool
//! delta before publication. Capture failures invalidate the whole calculation.

use allocation::{DamageAllocation, ModifierContribution};

use super::*;

impl DamageCalculation {
    fn try_new() -> Result<Self, std::collections::TryReserveError> {
        let mut modifiers = storage::Slots::default();
        modifiers.try_init(caps::DAMAGE_MODIFIERS, || {
            Ok(ModifierContribution {
                source: SourceSnapshot::try_new()?,
                amount: 0,
            })
        })?;
        let mut strength = storage::Slots::default();
        strength.try_init(caps::STR_REDUCTIONS, || Ok((SourceSnapshot::try_new()?, 0)))?;
        let mut results = Vec::new();
        results.try_reserve_exact(caps::DAMAGE_RESULTS)?;
        Ok(Self {
            serial: 0,
            source: SourceSnapshot::try_new()?,
            producer_role: ProducerRole::Unknown,
            segment: ProducerSegment::Attributed,
            original_target: 0,
            modifiers,
            results,
            weak: SourceSnapshot::try_new()?,
            strength,
            complete: false,
        })
    }

    fn reset(&mut self) {
        self.serial = 0;
        self.modifiers.clear();
        self.results.clear();
        self.strength.clear();
        self.complete = false;
    }
}

impl Provenance {
    pub(super) fn reserve_calculations_and_pools(
        &mut self,
    ) -> Result<(), std::collections::TryReserveError> {
        self.calculations
            .try_init(caps::DAMAGE_CALCULATIONS, DamageCalculation::try_new)?;
        self.pools
            .try_reserve_exact(caps::MAX_PLAYER_SLOTS.saturating_sub(self.pools.len()))?;
        while self.pools.len() < caps::MAX_PLAYER_SLOTS {
            self.pools.push(SourcePool::try_new()?);
        }
        Ok(())
    }

    pub(super) fn clear_calculations_and_pools(&mut self) {
        for calculation in self.calculations.iter_mut() {
            calculation.reset();
        }
        self.calculations.clear();
        for pool in &mut self.pools {
            pool.clear();
        }
    }
}

impl ObservedDamage {
    #[allow(clippy::too_many_arguments)]
    fn from_wire(
        total: i32,
        unblocked: i32,
        blocked: i32,
        kind: i32,
        receiver: i32,
        weak: i32,
        diagnostics: &mut SourceDiagnostics,
    ) -> Result<Self, SourceFailure> {
        if total < 0
            || unblocked < 0
            || blocked < 0
            || weak < 0
            || unblocked.checked_add(blocked) != Some(total)
        {
            return Err(SourceFailure::Packet);
        }
        Ok(Self {
            total: total as u64,
            unblocked: unblocked as u64,
            blocked: blocked as u64,
            kind: ResultKind::decode(kind, diagnostics),
            receiver: super::super::state::clamp_source_slot(receiver),
            weak_prevented: weak as u64,
        })
    }
}

impl LedgerStage {
    fn received(
        &mut self,
        result: &ObservedDamage,
        source: &SourceSnapshot,
        weak: &SourceSnapshot,
        strength: &[(SourceSnapshot, u64)],
    ) -> Result<(), SourceFailure> {
        self.combat.damage_received = self
            .combat
            .damage_received
            .checked_add(i64::try_from(result.total).map_err(|_| SourceFailure::Arithmetic)?)
            .ok_or(SourceFailure::Arithmetic)?;
        if result.blocked > 0 {
            self.consume_block(result.receiver, result.blocked)?;
        }
        self.source_credit(weak, CreditField::MitigateWeak, result.weak_prevented)?;
        if result.kind == ResultKind::SelfDamage {
            self.source_credit(source, CreditField::SelfDamage, result.unblocked)?;
        } else {
            for (source, amount) in strength {
                self.source_credit(source, CreditField::MitigateStrength, *amount)?;
            }
        }
        Ok(())
    }

    fn apply_group(
        &mut self,
        source: &SourceSnapshot,
        segment: ProducerSegment,
        modifiers: &[ModifierContribution],
        results: &[ObservedDamage],
        weak: &SourceSnapshot,
        strength: &[(SourceSnapshot, u64)],
    ) -> Result<(), SourceFailure> {
        let outgoing: Vec<_> = results
            .iter()
            .filter(|result| result.kind == ResultKind::Outgoing)
            .map(|result| (result.total, result.blocked))
            .collect();
        let allocated = DamageAllocation::build(source, segment, modifiers, &outgoing)?;
        let mut outgoing_credits = allocated.into_iter();
        let mut strength = strength;
        for result in results {
            match result.kind {
                ResultKind::Outgoing => {
                    for credit in outgoing_credits.next().ok_or(SourceFailure::Packet)? {
                        self.damage(
                            credit.destination,
                            credit.segment,
                            credit.damage,
                            credit.blocked,
                        )?;
                    }
                }
                ResultKind::Incoming | ResultKind::SelfDamage => {
                    self.received(result, source, weak, strength)?;
                    if result.kind == ResultKind::Incoming {
                        strength = &[];
                    }
                }
                ResultKind::OstyDealt => {
                    let credits = DamageAllocation::build(
                        source,
                        ProducerSegment::Direct,
                        &[],
                        &[(result.total, result.blocked)],
                    )?;
                    for result in credits {
                        for credit in result {
                            self.damage(
                                credit.destination,
                                credit.segment,
                                credit.damage,
                                credit.blocked,
                            )?;
                        }
                    }
                }
                ResultKind::OstyAbsorbed => self.absorb_osty(result.receiver, result.total)?,
            }
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn damage_calculation_begin(
        &mut self,
        combat_seq: u64,
        transfer: u64,
        producer_role: i32,
        segment: i32,
        original_target: u64,
    ) -> u64 {
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            let role = ProducerRole::decode(producer_role, &mut self.source_transfers.diagnostics);
            let mut segment =
                match DamageSegment::decode(segment, &mut self.source_transfers.diagnostics) {
                    DamageSegment::Direct => ProducerSegment::Direct,
                    DamageSegment::Attributed => ProducerSegment::Attributed,
                    DamageSegment::Modifier => return Err(SourceFailure::Packet),
                };
            if original_target == 0 {
                return Err(SourceFailure::Packet);
            }
            if role == ProducerRole::Unknown {
                segment = ProducerSegment::Attributed;
            }
            if role == ProducerRole::Power && segment != ProducerSegment::Attributed
                || matches!(
                    role,
                    ProducerRole::Card | ProducerRole::Relic | ProducerRole::Potion
                ) && segment != ProducerSegment::Direct
            {
                return Err(SourceFailure::Packet);
            }
            if self.provenance.calculations.len() == caps::DAMAGE_CALCULATIONS
                || self.provenance.calculation_serial == PAYLOAD_MAX
            {
                return Err(SourceFailure::Capacity);
            }
            let serial = self.provenance.calculation_serial + 1;
            let calculation = self
                .provenance
                .calculations
                .vacant_mut()
                .ok_or(SourceFailure::Capacity)?;
            calculation.reset();
            self.source_transfers
                .snapshot_into(epoch, transfer, &mut calculation.source)?;
            if role == ProducerRole::Unknown {
                calculation.source.set_unknown(epoch);
            }
            calculation.weak.set_unknown(epoch);
            calculation.serial = serial;
            calculation.producer_role = role;
            calculation.segment = segment;
            calculation.original_target = original_target;
            calculation.complete = true;
            self.provenance.calculations.activate();
            self.provenance.calculation_serial = serial;
            Ok(Token {
                epoch,
                kind: TokenKind::DamageCalculation,
                payload: serial,
            }
            .encode())
        })();
        match result {
            Ok(token) => token,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
                0
            }
        }
    }

    fn calculation_mutation(
        &mut self,
        calculation: u64,
        mutation: impl FnOnce(&mut State, usize) -> Result<(), SourceFailure>,
    ) -> i32 {
        let result = (|| {
            let token = self.provenance_token(calculation, TokenKind::DamageCalculation)?;
            let index = self
                .provenance
                .calculations
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            if !self.provenance.calculations[index].complete {
                return Err(SourceFailure::Packet);
            }
            self.provenance.calculations[index].complete = false;
            mutation(self, index)?;
            self.provenance.calculations[index].complete = true;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn damage_modifier_contribution(
        &mut self,
        calculation: u64,
        transfer: u64,
        amount: i32,
    ) -> i32 {
        self.calculation_mutation(calculation, |state, index| {
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let epoch = state.provenance.calculations[index].source.combat_seq();
            let epoch = CombatEpoch::from_wire(u64::from(epoch))?;
            let calculation = &mut state.provenance.calculations[index];
            if calculation.modifiers.len() == caps::DAMAGE_MODIFIERS {
                return Err(SourceFailure::Capacity);
            }
            let modifier = calculation
                .modifiers
                .vacant_mut()
                .ok_or(SourceFailure::Capacity)?;
            state
                .source_transfers
                .snapshot_into(epoch, transfer, &mut modifier.source)?;
            if amount > 0 {
                modifier.amount = amount as u64;
                calculation.modifiers.activate();
            }
            Ok(())
        })
    }

    pub(crate) fn damage_calculation_weak_source(
        &mut self,
        calculation: u64,
        transfer: u64,
    ) -> i32 {
        self.calculation_mutation(calculation, |state, index| {
            let epoch = state.provenance.calculations[index].source.combat_seq();
            let epoch = CombatEpoch::from_wire(u64::from(epoch))?;
            state.source_transfers.snapshot_into(
                epoch,
                transfer,
                &mut state.provenance.calculations[index].weak,
            )?;
            Ok(())
        })
    }

    pub(crate) fn damage_calculation_enemy_hit(
        &mut self,
        calculation: u64,
        dealer: u64,
        base: i32,
        strength: i32,
    ) -> i32 {
        self.calculation_mutation(calculation, |state, index| {
            if dealer == 0 {
                return Err(SourceFailure::Packet);
            }
            let reductions = &state.provenance.reductions;
            let mut count = 0;
            let total = reductions
                .iter()
                .filter(|entry| entry.creature == dealer)
                .try_fold(0_u64, |sum, entry| {
                    count += 1;
                    sum.checked_add(entry.amount)
                })
                .ok_or(SourceFailure::Arithmetic)?;
            let hypothetical = (i128::from(base) + i128::from(strength) + i128::from(total)).max(0);
            let effective = u64::try_from(hypothetical.min(i128::from(total)))
                .map_err(|_| SourceFailure::Arithmetic)?;
            let mut allocated = 0_u64;
            let shares = &mut state.provenance.calculations[index].strength;
            shares.clear();
            if total > 0 {
                for (position, entry) in reductions
                    .iter()
                    .filter(|entry| entry.creature == dealer)
                    .enumerate()
                {
                    let amount = if position + 1 == count {
                        effective - allocated
                    } else {
                        (u128::from(effective) * u128::from(entry.amount) / u128::from(total))
                            as u64
                    };
                    allocated += amount;
                    if amount > 0 {
                        let capture = shares.vacant_mut().ok_or(SourceFailure::Capacity)?;
                        capture.0.clone_from(&entry.source);
                        capture.1 = amount;
                        shares.activate();
                    }
                }
            }
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn damage_result_append(
        &mut self,
        calculation: u64,
        total: i32,
        unblocked: i32,
        blocked: i32,
        kind: i32,
        receiver: i32,
        weak_prevented: i32,
    ) -> i32 {
        self.calculation_mutation(calculation, |state, index| {
            let result = ObservedDamage::from_wire(
                total,
                unblocked,
                blocked,
                kind,
                receiver,
                weak_prevented,
                &mut state.source_transfers.diagnostics,
            )?;
            let calculation = &mut state.provenance.calculations[index];
            if calculation.results.len() == caps::DAMAGE_RESULTS {
                return Err(SourceFailure::Capacity);
            }
            calculation.results.push(result);
            Ok(())
        })
    }

    pub(crate) fn damage_calculation_commit(&mut self, calculation: u64) -> i32 {
        let result = (|| {
            let token = self.provenance_token(calculation, TokenKind::DamageCalculation)?;
            let index = self
                .provenance
                .calculations
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            let result = (|| {
                let calculation = &self.provenance.calculations[index];
                if !calculation.complete {
                    return Err(SourceFailure::Packet);
                }
                crate::data::persistence::event_log!(
                    "  damage group: target {}, producer role {:?}",
                    calculation.original_target,
                    calculation.producer_role
                );
                let mut stage = LedgerStage::new(self)?;
                stage.apply_group(
                    &calculation.source,
                    calculation.segment,
                    &calculation.modifiers,
                    &calculation.results,
                    &calculation.weak,
                    &calculation.strength,
                )?;
                stage.commit(self)?;
                for result_index in 0..self.provenance.calculations[index].results.len() {
                    let result = &self.provenance.calculations[index].results[result_index];
                    if matches!(
                        result.kind,
                        ResultKind::Incoming | ResultKind::SelfDamage | ResultKind::OstyAbsorbed
                    ) {
                        let receiver = result.receiver;
                        self.slot_index(i32::from(receiver));
                    }
                }
                Ok(())
            })();
            self.provenance.calculations[index].reset();
            self.provenance.calculations.remove(index);
            result?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn damage_calculation_abort(&mut self, calculation: u64) -> i32 {
        let result = (|| {
            let token = self.provenance_token(calculation, TokenKind::DamageCalculation)?;
            let index = self
                .provenance
                .calculations
                .iter()
                .position(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            self.provenance.calculations[index].reset();
            self.provenance.calculations.remove(index);
            Ok(())
        })();
        self.source_status(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn damage_unattributed(
        &mut self,
        combat_seq: u64,
        total: i32,
        unblocked: i32,
        blocked: i32,
        kind: i32,
        receiver: i32,
        weak_prevented: i32,
    ) -> i32 {
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            let result = ObservedDamage::from_wire(
                total,
                unblocked,
                blocked,
                kind,
                receiver,
                weak_prevented,
                &mut self.source_transfers.diagnostics,
            )?;
            let receiver = result.receiver;
            let received = matches!(
                result.kind,
                ResultKind::Incoming | ResultKind::SelfDamage | ResultKind::OstyAbsorbed
            );
            self.provenance.incoming.set_unknown(epoch);
            let source = &self.provenance.incoming;
            let mut stage = LedgerStage::new(self)?;
            stage.apply_group(
                source,
                ProducerSegment::Attributed,
                &[],
                &[result],
                source,
                &[],
            )?;
            stage.commit(self)?;
            if received {
                self.slot_index(i32::from(receiver));
            }
            Ok(())
        })();
        self.source_status(result)
    }
}
