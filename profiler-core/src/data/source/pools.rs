//! Existing FIFO block and LIFO Osty slices keep their outer arithmetic.
//! Positive credits advance source prefixes separately from base residue.

use super::*;

impl Clone for SourcePool {
    fn clone(&self) -> Self {
        Self {
            blocks: self.blocks.clone(),
            pending: self.pending.clone(),
            osty: self.osty.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.blocks.clone_from(&source.blocks);
        if self.pending.all_slots().len() < source.pending.all_slots().len() {
            self.pending = source.pending.clone();
        }
        self.pending.clear();
        // Tuple clone_from replaces the source owner, so copy its fields separately.
        for (snapshot, amount) in &source.pending {
            let pending = self
                .pending
                .vacant_mut()
                .expect("pool publication retains the initialized pending reserve");
            pending.0.clone_from(snapshot);
            pending.1 = *amount;
            self.pending.activate();
        }
        self.osty.clone_from(&source.osty);
    }
}

impl SourcePool {
    pub(super) fn try_new() -> Result<Self, std::collections::TryReserveError> {
        let mut pool = Self::default();
        pool.blocks
            .try_init(caps::BLOCK_POOL, SourceBlock::try_new)?;
        pool.pending.try_init(caps::PENDING_BLOCK_CONTRIBS, || {
            Ok((SourceSnapshot::try_new()?, 0))
        })?;
        pool.osty.try_init(caps::OSTY_STACK, || {
            Ok(SourceOsty {
                source: SourcePrefix::try_new()?,
                remaining: 0,
            })
        })?;
        Ok(pool)
    }

    pub(super) fn clear(&mut self) {
        self.blocks.clear();
        self.pending.clear();
        self.osty.clear();
    }
}

impl Clone for SourceBlock {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            base_original: self.base_original,
            base_consumed: self.base_consumed,
            remaining: self.remaining,
            mods: self.mods.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.base.clone_from(&source.base);
        self.base_original = source.base_original;
        self.base_consumed = source.base_consumed;
        self.remaining = source.remaining;
        self.mods.clone_from(&source.mods);
    }
}

impl Clone for SourceBlockMod {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            original: self.original,
            consumed: self.consumed,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.source.clone_from(&source.source);
        self.original = source.original;
        self.consumed = source.consumed;
    }
}

impl Clone for SourceOsty {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            remaining: self.remaining,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.source.clone_from(&source.source);
        self.remaining = source.remaining;
    }
}

impl SourceBlock {
    fn try_new() -> Result<Self, std::collections::TryReserveError> {
        let mut mods = storage::Slots::default();
        mods.try_init(Self::MAX_MODS, || {
            Ok(SourceBlockMod {
                source: SourcePrefix::try_new()?,
                original: 0,
                consumed: 0,
            })
        })?;
        Ok(Self {
            base: SourcePrefix::try_new()?,
            mods,
            ..Self::default()
        })
    }

    fn consume(
        &mut self,
        take: u64,
    ) -> Result<Vec<(Destination, CreditField, u64)>, SourceFailure> {
        let total = self
            .mods
            .iter()
            .try_fold(self.base_original, |sum, entry| {
                sum.checked_add(entry.original)
            })
            .ok_or(SourceFailure::Arithmetic)?;
        let consumed_after = total
            .checked_sub(self.remaining)
            .and_then(|value| value.checked_add(take))
            .ok_or(SourceFailure::Arithmetic)?;
        let base_floor = if total == 0 {
            0
        } else {
            (u128::from(self.base_original) * u128::from(consumed_after) / u128::from(total)) as u64
        };
        let mut base_delta = i128::from(base_floor) - i128::from(self.base_consumed);
        let mut allocated = base_delta;
        let mut mod_deltas = Vec::with_capacity(self.mods.len());
        for entry in &self.mods {
            let after = if total == 0 {
                0
            } else {
                (u128::from(entry.original) * u128::from(consumed_after) / u128::from(total)) as u64
            };
            let delta = after
                .checked_sub(entry.consumed)
                .ok_or(SourceFailure::Arithmetic)?;
            mod_deltas.push(delta);
            allocated += i128::from(delta);
        }
        base_delta += i128::from(take) - allocated;
        let mut credits = Vec::new();
        if base_delta > 0 {
            let amount = u64::try_from(base_delta).map_err(|_| SourceFailure::Arithmetic)?;
            credits.extend(
                self.base
                    .credit(amount)?
                    .into_iter()
                    .map(|(destination, amount)| {
                        (destination, CreditField::BlockEffective, amount)
                    }),
            );
        }
        for (entry, delta) in self.mods.iter_mut().zip(mod_deltas) {
            entry.consumed = entry
                .consumed
                .checked_add(delta)
                .ok_or(SourceFailure::Arithmetic)?;
            debug_assert!(
                entry.consumed <= entry.original,
                "modifier consumption cannot exceed its fixed slice"
            );
            credits.extend(
                entry
                    .source
                    .credit(delta)?
                    .into_iter()
                    .map(|(destination, amount)| (destination, CreditField::BlockModifier, amount)),
            );
        }
        self.base_consumed = i64::try_from(i128::from(self.base_consumed) + base_delta)
            .map_err(|_| SourceFailure::Arithmetic)?;
        self.remaining = self
            .remaining
            .checked_sub(take)
            .ok_or(SourceFailure::Arithmetic)?;
        Ok(credits)
    }
}

impl LedgerStage {
    fn push_block(
        &mut self,
        slot: SourceSlot,
        source: &SourceSnapshot,
        amount: u64,
    ) -> Result<(), SourceFailure> {
        let pool = self.pool(slot)?;
        let pending = &pool.pending;
        let modifiers = pending
            .iter()
            .try_fold(0_u64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(SourceFailure::Arithmetic)?;
        let base = amount.saturating_sub(modifiers);
        if pending.is_empty()
            && let Some(entry) = pool
                .blocks
                .iter_mut()
                .find(|entry| entry.mods.is_empty() && entry.base.source() == source)
        {
            entry.remaining = entry
                .remaining
                .checked_add(base)
                .ok_or(SourceFailure::Arithmetic)?;
            entry.base_original = entry
                .base_original
                .checked_add(base)
                .ok_or(SourceFailure::Arithmetic)?;
            return Ok(());
        }
        if pool.blocks.len() == caps::BLOCK_POOL {
            pool.pending.clear();
            self.capacity_lost = true;
            return Ok(());
        }
        let lost = pending.len() > SourceBlock::MAX_MODS;
        let entry = pool.blocks.vacant_mut().ok_or(SourceFailure::Capacity)?;
        entry.base.reset_from(source);
        entry.base_original = base;
        entry.base_consumed = 0;
        entry.remaining = base;
        entry.mods.clear();
        entry
            .mods
            .try_init(SourceBlock::MAX_MODS, || Ok(SourceBlockMod::default()))
            .map_err(|_| SourceFailure::Capacity)?;
        for (source, amount) in pending.iter().take(SourceBlock::MAX_MODS) {
            entry.remaining = entry
                .remaining
                .checked_add(*amount)
                .ok_or(SourceFailure::Arithmetic)?;
            let modifier = entry.mods.vacant_mut().ok_or(SourceFailure::Capacity)?;
            modifier.source.reset_from(source);
            modifier.original = *amount;
            modifier.consumed = 0;
            entry.mods.activate();
        }
        if entry.remaining > 0 {
            pool.blocks.activate();
        }
        pool.pending.clear();
        self.capacity_lost |= lost;
        Ok(())
    }

    pub(super) fn consume_block(
        &mut self,
        slot: SourceSlot,
        blocked: u64,
    ) -> Result<(), SourceFailure> {
        let mut remaining = blocked;
        let mut index = 0;
        while remaining > 0 && index < self.pool(slot)?.blocks.len() {
            let block = &mut self.pool(slot)?.blocks[index];
            let take = remaining.min(block.remaining);
            let credits = block.consume(take)?;
            remaining -= take;
            if block.remaining == 0 {
                self.pool(slot)?.blocks.remove(index);
            } else {
                index += 1;
            }
            for (destination, field, amount) in credits {
                self.credit(
                    destination,
                    field,
                    i64::try_from(amount).map_err(|_| SourceFailure::Arithmetic)?,
                )?;
            }
        }
        self.pool(slot)?.pending.clear();
        Ok(())
    }

    pub(super) fn absorb_osty(
        &mut self,
        slot: SourceSlot,
        amount: u64,
    ) -> Result<(), SourceFailure> {
        let mut remaining = amount;
        while remaining > 0 && !self.pool(slot)?.osty.is_empty() {
            let entry = self
                .pool(slot)?
                .osty
                .last_mut()
                .ok_or(SourceFailure::Packet)?;
            let take = remaining.min(entry.remaining);
            let credits = entry.source.credit(take)?;
            entry.remaining -= take;
            remaining -= take;
            if entry.remaining == 0 {
                let pool = self.pool(slot)?;
                pool.osty.remove(pool.osty.len() - 1);
            }
            for (destination, share) in credits {
                self.credit(
                    destination,
                    CreditField::BlockEffective,
                    i64::try_from(share).map_err(|_| SourceFailure::Arithmetic)?,
                )?;
            }
        }
        if remaining > 0 {
            let row = crate::data::ledger::get_or_create_card_kind(
                &mut self.combat,
                TEAM_SLOT,
                "OSTY",
                crate::source_kind::SourceKind::Osty,
            )
            .ok_or(SourceFailure::Capacity)?;
            self.credit(
                Destination::Row(row),
                CreditField::BlockEffective,
                i64::try_from(remaining).map_err(|_| SourceFailure::Arithmetic)?,
            )?;
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn block_modifier_contribution(
        &mut self,
        combat_seq: u64,
        transfer: u64,
        amount: i32,
        receiver_slot: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            self.capture_source_snapshot(combat_seq, transfer)?;
            let source = &self.provenance.incoming;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let slot = super::super::state::clamp_source_slot(receiver_slot);
            let mut stage = LedgerStage::new(self)?;
            let pool = stage.pool(slot)?;
            if pool.pending.len() == caps::PENDING_BLOCK_CONTRIBS {
                return Err(SourceFailure::Capacity);
            }
            if amount > 0 {
                let pending = pool.pending.vacant_mut().ok_or(SourceFailure::Capacity)?;
                pending.0.clone_from(source);
                pending.1 = amount as u64;
                pool.pending.activate();
            }
            stage.commit(self)?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn block_gained(
        &mut self,
        combat_seq: u64,
        amount: i32,
        transfer: u64,
        receiver_slot: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            self.capture_source_snapshot(combat_seq, transfer)?;
            let source = &self.provenance.incoming;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            if amount == 0 {
                return Ok(());
            }
            let slot = super::super::state::clamp_source_slot(receiver_slot);
            let mut stage = LedgerStage::new(self)?;
            stage.combat.block_total = stage
                .combat
                .block_total
                .checked_add(i64::from(amount))
                .ok_or(SourceFailure::Arithmetic)?;
            stage.source_credit(source, CreditField::BlockGained, amount as u64)?;
            stage.push_block(slot, source, amount as u64)?;
            stage.commit(self)?;
            self.slot_index(i32::from(slot));
            Ok(())
        })();
        self.source_status(result)
    }

    fn source_event(
        &mut self,
        combat_seq: u64,
        transfer: u64,
        amount: i32,
        field: CreditField,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            self.capture_source_snapshot(combat_seq, transfer)?;
            let source = &self.provenance.incoming;
            let mut stage = LedgerStage::new(self)?;
            stage.source_credit(source, field, amount as u64)?;
            stage.commit(self)?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn forge(&mut self, combat_seq: u64, transfer: u64, amount: i32) -> i32 {
        self.source_event(combat_seq, transfer, amount, CreditField::Forge)
    }

    pub(crate) fn buff_mitigation(
        &mut self,
        combat_seq: u64,
        transfer: u64,
        prevented: i32,
    ) -> i32 {
        self.source_event(combat_seq, transfer, prevented, CreditField::MitigateBuff)
    }

    pub(crate) fn osty_summoned(
        &mut self,
        combat_seq: u64,
        transfer: u64,
        amount: i32,
        owner_slot: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            self.capture_source_snapshot(combat_seq, transfer)?;
            let source = &self.provenance.incoming;
            let slot = super::super::state::clamp_source_slot(owner_slot);
            let mut stage = LedgerStage::new(self)?;
            let pool = stage.pool(slot)?;
            if pool.osty.len() == caps::OSTY_STACK {
                return Err(SourceFailure::Capacity);
            }
            if amount > 0 {
                let entry = pool.osty.vacant_mut().ok_or(SourceFailure::Capacity)?;
                entry.source.reset_from(source);
                entry.remaining = amount as u64;
                pool.osty.activate();
            }
            stage.commit(self)?;
            self.slot_index(i32::from(slot));
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn osty_killed(&mut self, combat_seq: u64, owner_slot: i32, play: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let owner = super::super::state::clamp_source_slot(owner_slot);
            if play != 0 {
                let token = self.provenance_token(play, TokenKind::CardPlay)?;
                let source = &self
                    .provenance
                    .plays
                    .iter()
                    .find(|play| play.serial == token.payload && play.owner_slot == owner)
                    .ok_or(SourceFailure::Token)?
                    .source;
                self.provenance.incoming.clone_from(source);
            }
            let mut stage = LedgerStage::new(self)?;
            let remaining = stage
                .pool(owner)?
                .osty
                .iter()
                .try_fold(0_u64, |sum, entry| sum.checked_add(entry.remaining))
                .ok_or(SourceFailure::Arithmetic)?;
            if play != 0 {
                for (destination, amount) in self
                    .provenance
                    .incoming
                    .budgets(remaining)
                    .take(remaining)?
                {
                    stage.credit(
                        destination,
                        CreditField::BlockEffective,
                        -i64::try_from(amount).map_err(|_| SourceFailure::Arithmetic)?,
                    )?;
                }
            }
            stage.pool(owner)?.osty.clear();
            stage.commit(self)?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn block_pool_clear(&mut self, combat_seq: u64, player_slot: i32) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let slot = super::super::state::clamp_source_slot(player_slot);
            if let Some(pool) = self.provenance.pools.get_mut(usize::from(slot)) {
                pool.blocks.clear();
            }
            Ok(())
        })();
        self.source_status(result)
    }
}
