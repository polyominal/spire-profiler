//! Existing FIFO block and LIFO Osty slices keep their outer arithmetic.
//! Positive credits advance source prefixes separately from base residue.

use super::*;

impl SourceBlock {
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

impl LedgerStage<'_> {
    #[allow(
        clippy::too_many_lines,
        reason = "bounded FIFO overflow and modifier allocation share one transactional update"
    )]
    fn push_block(
        &mut self,
        slot: SourceSlot,
        source: SourceSnapshot,
        amount: u64,
    ) -> Result<(), SourceFailure> {
        let pool = self.pool(slot)?;
        let mut pending = std::mem::take(&mut pool.pending);
        let modifiers = pending
            .iter()
            .try_fold(0_u64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(SourceFailure::Arithmetic)?;
        let base = amount.saturating_sub(modifiers);
        if pending.is_empty()
            && let Some(entry) = pool
                .blocks
                .iter_mut()
                .find(|entry| entry.mods.is_empty() && entry.base.source() == &source)
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
            let tail = pool
                .blocks
                .last_mut()
                .expect("a full bounded pool has a tail");
            let remaining = tail
                .remaining
                .checked_add(amount)
                .ok_or(SourceFailure::Arithmetic)?;
            *tail = SourceBlock {
                base: SourceSnapshot::unknown(source.epoch()).prefix(),
                base_original: remaining,
                base_consumed: 0,
                remaining,
                mods: Box::default(),
            };
            self.capacity_lost = true;
            return Ok(());
        }
        let lost = pending.len() > SourceBlock::MAX_MODS;
        if lost {
            let unknown = pending
                .drain(SourceBlock::MAX_MODS - 1..)
                .try_fold(0_u64, |sum, (_, amount)| sum.checked_add(amount))
                .ok_or(SourceFailure::Arithmetic)?;
            pending.push((SourceSnapshot::unknown(source.epoch()), unknown));
        }
        let mut remaining = base;
        let mut mods = Vec::with_capacity(pending.len().min(SourceBlock::MAX_MODS));
        for (source, amount) in pending.into_iter().take(SourceBlock::MAX_MODS) {
            remaining = remaining
                .checked_add(amount)
                .ok_or(SourceFailure::Arithmetic)?;
            mods.push(SourceBlockMod {
                source: source.prefix(),
                original: amount,
                consumed: 0,
            });
        }
        if remaining > 0 {
            pool.blocks.push(SourceBlock {
                base: source.prefix(),
                base_original: base,
                base_consumed: 0,
                remaining,
                mods: mods.into_boxed_slice(),
            });
        }
        self.capacity_lost |= lost;
        Ok(())
    }

    pub(super) fn consume_block(
        &mut self,
        slot: SourceSlot,
        blocked: u64,
    ) -> Result<(), SourceFailure> {
        let mut remaining = blocked;
        while remaining > 0 {
            let pool = self.pool(slot)?;
            let Some(block) = pool.blocks.first_mut() else {
                break;
            };
            let take = remaining.min(block.remaining);
            let credits = block.consume(take)?;
            remaining -= take;
            if block.remaining == 0 {
                pool.blocks.remove(0);
            }
            for (destination, field, amount) in credits {
                self.credit(
                    destination,
                    field,
                    i64::try_from(amount).map_err(|_| SourceFailure::Arithmetic)?,
                )?;
            }
        }
        if remaining > 0 {
            self.credit(
                Destination::Unknown(TEAM_SLOT),
                CreditField::BlockEffective,
                i64::try_from(remaining).map_err(|_| SourceFailure::Arithmetic)?,
            )?;
            self.unobserved_defense = true;
        }
        let pool = self.pool(slot)?;
        pool.pending.clear();
        pool.pending_failed = false;
        Ok(())
    }

    pub(super) fn absorb_osty(
        &mut self,
        slot: SourceSlot,
        amount: u64,
    ) -> Result<(), SourceFailure> {
        let mut remaining = amount;
        while remaining > 0 {
            let pool = self.pool(slot)?;
            let Some(entry) = pool.osty.last_mut() else {
                break;
            };
            let take = remaining.min(entry.remaining);
            let credits = entry.source.credit(take)?;
            entry.remaining -= take;
            remaining -= take;
            if entry.remaining == 0 {
                pool.osty.pop();
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
                self.combat,
                TEAM_SLOT,
                "OSTY",
                crate::source_kind::SourceKind::Osty,
            )
            .ok_or(SourceFailure::Capacity)?;
            self.credit(
                Destination::Row(row as u32),
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
        let mut observed_slot = None;
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            let slot = self.sources.diagnostics.slot(receiver_slot);
            observed_slot = Some(slot);
            let source = self.source_snapshot(epoch, transfer)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let mut stage = LedgerStage::new(self)?;
            let pool = stage.pool(slot)?;
            if pool.pending_failed {
                debug_assert!(
                    pool.pending.is_empty(),
                    "rejected modifier batches cannot retain partial credits"
                );
                return Err(SourceFailure::Packet);
            }
            if pool.pending.len() == caps::PENDING_BLOCK_CONTRIBS {
                return Err(SourceFailure::Capacity);
            }
            if amount > 0 {
                pool.pending.push((source, amount as u64));
            }
            stage.commit()?;
            Ok(())
        })();
        if result.is_err()
            && let Some(slot) = observed_slot
        {
            while self.provenance.pools.len() <= usize::from(slot) {
                self.provenance.pools.push(SourcePool::default());
            }
            let pool = &mut self.provenance.pools[usize::from(slot)];
            pool.pending.clear();
            pool.pending_failed = true;
        }
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
            let epoch = self.provenance_epoch(combat_seq)?;
            let mut source = self.source_snapshot(epoch, transfer)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let slot = self.sources.diagnostics.slot(receiver_slot);
            let mut stage = LedgerStage::new(self)?;
            let pool = stage.pool(slot)?;
            if pool.pending_failed {
                // A missing modifier leaves no trustworthy base/modifier split.
                source = SourceSnapshot::unknown(epoch);
                pool.pending_failed = false;
            }
            if amount == 0 {
                pool.pending.clear();
                return stage.commit();
            }
            stage.combat.block_total = stage
                .combat
                .block_total
                .checked_add(i64::from(amount))
                .ok_or(SourceFailure::Arithmetic)?;
            stage.source_credit(&source, CreditField::BlockGained, amount as u64)?;
            stage.push_block(slot, source, amount as u64)?;
            stage.commit()?;
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
            let epoch = self.provenance_epoch(combat_seq)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let source = self.source_snapshot(epoch, transfer)?;
            let mut stage = LedgerStage::new(self)?;
            stage.source_credit(&source, field, amount as u64)?;
            stage.commit()?;
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
            let epoch = self.provenance_epoch(combat_seq)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let source = self.source_snapshot(epoch, transfer)?;
            let slot = self.sources.diagnostics.slot(owner_slot);
            let mut stage = LedgerStage::new(self)?;
            let pool = stage.pool(slot)?;
            if pool.osty.len() == caps::OSTY_STACK {
                return Err(SourceFailure::Capacity);
            }
            if amount > 0 {
                pool.osty.push(SourceOsty {
                    source: source.prefix(),
                    remaining: amount as u64,
                });
            }
            stage.commit()?;
            self.slot_index(i32::from(slot));
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn osty_killed(&mut self, combat_seq: u64, owner_slot: i32, play: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let owner = self.sources.diagnostics.slot(owner_slot);
            let source = if play == 0 {
                None
            } else {
                let token = self.provenance_token(play, TokenKind::CardPlay)?;
                Some(
                    self.provenance
                        .plays
                        .iter()
                        .find(|play| play.serial == token.payload && play.owner_slot == owner)
                        .ok_or(SourceFailure::Token)?
                        .source
                        .clone(),
                )
            };
            let mut stage = LedgerStage::new(self)?;
            let remaining = stage
                .pool(owner)?
                .osty
                .iter()
                .try_fold(0_u64, |sum, entry| sum.checked_add(entry.remaining))
                .ok_or(SourceFailure::Arithmetic)?;
            if let Some(source) = source {
                for (destination, amount) in source.budgets(remaining).take(remaining)? {
                    stage.credit(
                        destination,
                        CreditField::BlockEffective,
                        -i64::try_from(amount).map_err(|_| SourceFailure::Arithmetic)?,
                    )?;
                }
            }
            stage.pool(owner)?.osty.clear();
            stage.commit()?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn block_pool_clear(&mut self, combat_seq: u64, player_slot: i32) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let slot = self.sources.diagnostics.slot(player_slot);
            if let Some(pool) = self.provenance.pools.get_mut(usize::from(slot)) {
                pool.blocks.clear();
                pool.pending.clear();
                pool.pending_failed = false;
            }
            Ok(())
        })();
        self.source_status(result)
    }
}
