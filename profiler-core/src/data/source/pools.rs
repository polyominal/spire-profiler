//! Block grants drain FIFO; only adjacent modifier-free equal sources merge.
//! An overflowing pool replaces its last remaining slice and the new grant with
//! Unknown at the same tail position. Missing block provenance credits observed
//! absorption to the receiver's Unknown row, never inventing a block gain.
//! Each gain owns its complete bounded modifier batch. Incomplete batches keep
//! observed producer gains but place their effective slice under Unknown.
//! Modifier budgets never exceed the observed gain. One modifier uses cumulative
//! floors with producer residue; multiple modifiers share a monotone weighted
//! prefix (ties favor later modifiers, then the producer). Source prefixes keep
//! the same supplier weights through partial consumption and adjacent merges.

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
        let after = if self.mods.len() <= 1 {
            self.mods
                .iter()
                .map(|entry| {
                    (u128::from(entry.original) * u128::from(consumed_after) / u128::from(total))
                        as u64
                })
                .collect::<Box<[_]>>()
        } else {
            // Independent modifier floors can catch up together and exceed a hit.
            // A shared monotone prefix assigns each consumed point exactly once.
            let mut weights: Vec<_> = self.mods.iter().map(|entry| entry.original).collect();
            weights.push(self.base_original);
            SourcePrefix::allocation(consumed_after, weights.iter().copied())?
        };
        let mod_deltas: Vec<_> = self
            .mods
            .iter()
            .zip(after)
            .map(|(entry, after)| {
                after
                    .checked_sub(entry.consumed)
                    .ok_or(SourceFailure::Arithmetic)
            })
            .collect::<Result<_, _>>()?;
        let allocated = mod_deltas
            .iter()
            .try_fold(0_u64, |sum, delta| sum.checked_add(*delta))
            .ok_or(SourceFailure::Arithmetic)?;
        let base_delta = take
            .checked_sub(allocated)
            .ok_or(SourceFailure::Arithmetic)?;
        let mut credits = Vec::new();
        if base_delta > 0 {
            for (destination, amount) in self.base.credit(base_delta)? {
                credits.push((destination, CreditField::BlockEffective, amount));
            }
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
        self.remaining = self
            .remaining
            .checked_sub(take)
            .ok_or(SourceFailure::Arithmetic)?;
        Ok(credits)
    }
}

impl LedgerStage<'_> {
    fn push_block(
        &mut self,
        slot: SourceSlot,
        source: SourceSnapshot,
        amount: u64,
        modifiers: BlockModifiers,
    ) -> Result<(), SourceFailure> {
        let epoch = source.epoch();
        let (source, entries) = if modifiers.incomplete {
            (SourceSnapshot::unknown_for(epoch, slot), Box::default())
        } else {
            (source, modifiers.entries)
        };
        let modifier_total = entries
            .iter()
            .try_fold(0_u64, |sum, (_, amount)| sum.checked_add(*amount))
            .ok_or(SourceFailure::Arithmetic)?;
        let modifier_budget = modifier_total.min(amount);
        let budgets =
            RootBudgets::proportional(modifier_budget, entries.iter().map(|(_, amount)| *amount))?;
        let base = amount - modifier_budget;
        let pool = self.pool(slot)?;
        if modifier_budget == 0
            && let Some(entry) = pool.blocks.last_mut()
            && entry.mods.is_empty()
            && entry.base.source() == &source
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
                .expect("a full positive-capacity pool has a tail");
            let remaining = tail
                .remaining
                .checked_add(amount)
                .ok_or(SourceFailure::Arithmetic)?;
            *tail = SourceBlock {
                base: SourceSnapshot::unknown_for(epoch, slot).prefix(),
                base_original: remaining,
                remaining,
                mods: Box::default(),
            };
            self.capacity_lost = true;
            return Ok(());
        }
        let remaining = amount;
        if remaining > 0 {
            pool.blocks.push(SourceBlock {
                base: source.prefix(),
                base_original: base,
                remaining,
                mods: entries
                    .into_iter()
                    .zip(budgets)
                    .filter_map(|((source, _), amount)| {
                        (amount > 0).then(|| SourceBlockMod {
                            source: source.prefix(),
                            original: amount,
                            consumed: 0,
                        })
                    })
                    .collect(),
            });
        }
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
                Destination::Unknown(slot),
                CreditField::BlockEffective,
                i64::try_from(remaining).map_err(|_| SourceFailure::Arithmetic)?,
            )?;
            self.diagnostics.report(SourceFailure::Token);
        }
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
    pub(crate) fn parse_block_modifiers(
        &mut self,
        combat_seq: u64,
        entries: &[(u64, i64)],
        incomplete: bool,
    ) -> BlockModifiers {
        let parsed = (|| {
            if incomplete || entries.len() > caps::BLOCK_MODIFIERS {
                return Err(SourceFailure::Packet);
            }
            let epoch = self.provenance_epoch(combat_seq)?;
            entries
                .iter()
                .map(|(handle, amount)| {
                    let amount = u32::try_from(*amount)
                        .ok()
                        .filter(|amount| *amount <= i32::MAX as u32)
                        .ok_or(SourceFailure::Packet)?;
                    let source = self.source_snapshot(epoch, *handle)?;
                    Ok((source, u64::from(amount)))
                })
                .collect::<Result<Box<[_]>, SourceFailure>>()
        })();
        match parsed {
            Ok(entries) => BlockModifiers {
                entries,
                incomplete: false,
            },
            Err(_) => {
                self.capture_failed("block-modifier-incomplete");
                BlockModifiers {
                    entries: Box::default(),
                    incomplete: true,
                }
            }
        }
    }

    pub(crate) fn block_gained(
        &mut self,
        combat_seq: u64,
        amount: i32,
        transfer: u64,
        receiver_slot: i32,
        modifiers: BlockModifiers,
    ) -> i32 {
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            let source = self.source_snapshot(epoch, transfer).unwrap_or_else(|_| {
                self.capture_failed("block-producer-incomplete");
                SourceSnapshot::unknown_for(
                    epoch,
                    super::super::state::clamp_source_slot(receiver_slot),
                )
            });
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
            stage.source_credit(&source, CreditField::BlockGained, amount as u64)?;
            stage.push_block(slot, source, amount as u64, modifiers)?;
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
            let slot = super::super::state::clamp_source_slot(owner_slot);
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
            let owner = super::super::state::clamp_source_slot(owner_slot);
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
            let slot = super::super::state::clamp_source_slot(player_slot);
            if let Some(pool) = self.provenance.pools.get_mut(usize::from(slot)) {
                pool.blocks.clear();
            }
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn block_pool_loss(
        &mut self,
        combat_seq: u64,
        player_slot: i32,
        amount: i32,
    ) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            if amount < 0 {
                return Err(SourceFailure::Packet);
            }
            let slot = self.sources.diagnostics.slot(player_slot);
            if amount == 0 {
                return Ok(());
            }
            let mut stage = LedgerStage::new(self)?;
            let mut remaining = amount as u64;
            while remaining > 0 {
                let pool = stage.pool(slot)?;
                let Some(block) = pool.blocks.first_mut() else {
                    break;
                };
                let take = remaining.min(block.remaining);
                // Loss advances the modifier prefix without defense credit.
                drop(block.consume(take)?);
                remaining -= take;
                if block.remaining == 0 {
                    pool.blocks.remove(0);
                }
            }
            if remaining > 0 {
                stage.diagnostics.report(SourceFailure::Token);
            }
            stage.commit()?;
            self.slot_index(i32::from(slot));
            Ok(())
        })();
        self.source_status(result)
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
