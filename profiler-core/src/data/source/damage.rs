//! Complete target-local result groups stage every ledger and defensive-pool
//! delta before publication. Capture failures invalidate the whole calculation.

use allocation::{DamageAllocation, ModifierContribution};

use super::*;

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

    fn apply_group(&mut self, calculation: &DamageCalculation) -> Result<(), SourceFailure> {
        let outgoing: Vec<_> = calculation
            .results
            .iter()
            .filter(|result| result.kind == ResultKind::Outgoing)
            .map(|result| (result.total, result.blocked))
            .collect();
        let allocated = DamageAllocation::build(
            &calculation.source,
            calculation.segment as i32,
            &calculation.modifiers,
            &outgoing,
        )?;
        let mut outgoing_credits = allocated.into_iter();
        let epoch = CombatEpoch::from_wire(u64::from(self.combat.seq))?;
        let unknown = SourceSnapshot::unknown(epoch);
        let weak = calculation.weak.as_ref().unwrap_or(&unknown);
        let mut strength = calculation.strength.as_slice();
        for result in &calculation.results {
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
                    self.received(result, &calculation.source, weak, strength)?;
                    if result.kind == ResultKind::Incoming {
                        strength = &[];
                    }
                }
                ResultKind::OstyDealt => {
                    let credits = DamageAllocation::build(
                        &calculation.source,
                        0,
                        &[],
                        &[(result.total, result.blocked)],
                    )?;
                    for result in credits {
                        for credit in result {
                            self.damage(credit.destination, 0, credit.damage, credit.blocked)?;
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
                DamageSegment::decode(segment, &mut self.source_transfers.diagnostics);
            if segment == DamageSegment::Modifier || original_target == 0 {
                return Err(SourceFailure::Packet);
            }
            let mut source = self.source_snapshot(combat_seq, transfer)?;
            if role == ProducerRole::Unknown {
                source = SourceSnapshot::unknown(epoch);
                segment = DamageSegment::Attributed;
            }
            if role == ProducerRole::Power && segment != DamageSegment::Attributed
                || matches!(
                    role,
                    ProducerRole::Card | ProducerRole::Relic | ProducerRole::Potion
                ) && segment != DamageSegment::Direct
            {
                return Err(SourceFailure::Packet);
            }
            if self.provenance.calculations.len() == caps::DAMAGE_CALCULATIONS
                || self.provenance.calculation_serial == PAYLOAD_MAX
            {
                return Err(SourceFailure::Capacity);
            }
            let serial = self.provenance.calculation_serial + 1;
            self.provenance.calculations.push(DamageCalculation {
                serial,
                source,
                producer_role: role,
                segment,
                original_target,
                modifiers: Vec::new(),
                results: Vec::new(),
                weak: None,
                strength: Vec::new(),
                complete: true,
            });
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
            let source = state.source_snapshot(u64::from(epoch), transfer)?;
            let calculation = &mut state.provenance.calculations[index];
            if calculation.modifiers.len() == caps::DAMAGE_MODIFIERS {
                return Err(SourceFailure::Capacity);
            }
            if amount > 0 {
                calculation.modifiers.push(ModifierContribution {
                    source,
                    amount: amount as u64,
                });
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
            let source = state.source_snapshot(u64::from(epoch), transfer)?;
            state.provenance.calculations[index].weak = Some(source);
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
            let reductions: Vec<_> = state
                .provenance
                .reductions
                .iter()
                .filter(|entry| entry.creature == dealer)
                .collect();
            let total = reductions
                .iter()
                .try_fold(0_u64, |sum, entry| sum.checked_add(entry.amount))
                .ok_or(SourceFailure::Arithmetic)?;
            let hypothetical = (i128::from(base) + i128::from(strength) + i128::from(total)).max(0);
            let effective = u64::try_from(hypothetical.min(i128::from(total)))
                .map_err(|_| SourceFailure::Arithmetic)?;
            let mut allocated = 0_u64;
            let mut shares = Vec::new();
            if total > 0 {
                for (index, entry) in reductions.iter().enumerate() {
                    let amount = if index + 1 == reductions.len() {
                        effective - allocated
                    } else {
                        (u128::from(effective) * u128::from(entry.amount) / u128::from(total))
                            as u64
                    };
                    allocated += amount;
                    if amount > 0 {
                        shares.push((entry.source.clone(), amount));
                    }
                }
            }
            state.provenance.calculations[index].strength = shares;
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
            let calculation = self.provenance.calculations.remove(index);
            if !calculation.complete {
                return Err(SourceFailure::Packet);
            }
            crate::data::persistence::event_log!(
                "  damage group: target {}, producer role {:?}",
                calculation.original_target,
                calculation.producer_role
            );
            let mut stage = LedgerStage::new(self)?;
            stage.apply_group(&calculation)?;
            stage.commit(self)?;
            for result in calculation.results {
                if matches!(
                    result.kind,
                    ResultKind::Incoming | ResultKind::SelfDamage | ResultKind::OstyAbsorbed
                ) {
                    self.slot_index(i32::from(result.receiver));
                }
            }
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
            let calculation = DamageCalculation {
                serial: 0,
                source: SourceSnapshot::unknown(epoch),
                producer_role: ProducerRole::Unknown,
                segment: DamageSegment::Attributed,
                original_target: 0,
                modifiers: Vec::new(),
                results: vec![result],
                weak: None,
                strength: Vec::new(),
                complete: true,
            };
            let mut stage = LedgerStage::new(self)?;
            stage.apply_group(&calculation)?;
            stage.commit(self)?;
            if received {
                self.slot_index(i32::from(receiver));
            }
            Ok(())
        })();
        self.source_status(result)
    }
}
