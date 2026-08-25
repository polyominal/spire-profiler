//! Attribution mechanics — the non-export helpers: source resolution, the
//! debuff-layer FIFO, the block pool, the modifier splits. The
//! model overview lives in [`super`]'s module doc; this module pins the
//! chain orders and the split arithmetic.
//!
//! # Resolution chains
//!
//! [`resolve_card_in`]: explicit card id → the slot's play source →
//! the innermost context → the orb fallback → the potion fallback →
//! `last_source` (when [`AsyncFallback::Allow`]) → None. Every branch keys
//! the created row at the resolved source's own slot: the event's explicit
//! slot, the generator's recorded slot, the context entry's slot, or the
//! caller's slot for the fallbacks. An explicit id equal to the slot's
//! `active_play_card_id` is the shim reporting the playing card's own id,
//! so it falls through to the play source instead of opening a card-kind
//! row.
//!
//! [`resolve_damage_route`] mirrors that order but records direct/indirect:
//! explicit id → direct; orb fallback → indirect; play source → direct;
//! innermost context → indirect iff a power; potion fallback → direct.
//! [`resolve_damage_source_in`] then lets poison layers claim the hit, and
//! finally falls back to `last_source` (indirect iff a power).
//!
//! # Block pool
//!
//! [`block_pool_push_in`] merges into an existing modifier-free chunk of
//! the same `(player, id, kind)`; a queued block-modifier contribution
//! blocks the merge and attaches to the new chunk (merging chunks with
//! different modifier breakdowns would corrupt the proportional split).
//! [`block_pool_consume_in`] drains FIFO, splitting each consumed slice
//! cumulatively-proportionally between the chunk's base and its modifier
//! slices; the residue lands on the base slice so the slices sum exactly.
//! The conservation bound is `credited <= blocked + total_mod_slices`: up
//! to one over-credited point per modifier slice (the residue mechanism),
//! plus blocked damage the ledger never recorded.
//!
//! # Damage modifier splits
//!
//! [`apply_pending_contribs_in`] carves the queued modifier share out of
//! the attacker's `dmg_direct` (then `dmg_attributed`) into the modifier
//! sources' `dmg_modifier`; the share is counted as unblocked.
//! [`split_over_appliers_in`] distributes a share across the recorded
//! appliers proportionally to their amounts (last share takes the
//! residue), or gives the modifier itself the full amount when none are
//! recorded.
//!
//! [`assert_card_damage_segments`] re-checks the segment decomposition
//! after every mutation so drift surfaces at the mutation site.
//!
//! ## Borrowing and handles
//!
//! The mutable tables live in [`state::State`] behind
//! `STATE: RefCell<State>`. Each function borrows State, mutates, and
//! releases before the caller emits log lines through `append_log` (which
//! re-borrows). A `&mut Combat` cannot escape the `STATE.with` closure, so
//! the `_in` helpers take `&mut State` instead. The `get_or_create_card*`
//! functions return the entry's INDEX: a `Vec` push reallocates the buffer
//! but never moves existing elements, so the index stays valid.

// Test-only: production defers log emission to the event callers.
#[cfg(test)]
use crate::data::persistence::append_log;
#[cfg(test)]
use crate::data::state::STATE;
use crate::data::state::{
    self, BlockEntry, BlockMod, CardStat, Combat, ContextEntry, OrbSource, PendingContrib,
    PlayerSlotState, SourceKind, SourceSlot, clamp_source_slot,
};
use crate::fail;

/// `card_index` is stable across appends, which never move elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRoute {
    pub card_index: usize,
    pub row_slot: SourceSlot,
    pub indirect: bool,
}

pub(crate) fn assert_card_damage_segments(card: &CardStat) {
    debug_assert_eq!(
        card.damage_dealt,
        card.dmg_direct + card.dmg_attributed + card.dmg_modifier,
        "damage segments of '{}' drifted from damage_dealt",
        card.id
    );
}

pub fn consume_debuff_layers_in(
    state: &mut state::State,
    creature_hash: u64,
    power_id: &str,
    amount: i64,
) {
    let mut remaining = amount;
    let mut i = 0;
    while i < state.debuff_layers.len() && remaining > 0 {
        let layer = &state.debuff_layers[i];
        if layer.creature_hash != creature_hash || layer.power_id != power_id {
            i += 1;
            continue;
        }
        let take = layer.duration.min(remaining);
        state.debuff_layers[i].duration -= take;
        remaining -= take;
        if state.debuff_layers[i].duration <= 0 {
            state.debuff_layers.remove(i);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
fn attribute_debuff_damage(creature_hash: u64, power_id: &str, amount: i64) -> bool {
    let mut logs = Vec::new();
    let outcome = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        attribute_debuff_damage_in(&mut state, creature_hash, power_id, amount, &mut logs)
    });
    for line in logs {
        append_log(line);
    }
    outcome
}

fn attribute_debuff_damage_in(
    state: &mut state::State,
    creature_hash: u64,
    power_id: &str,
    amount: i64,
    logs: &mut Vec<String>,
) -> bool {
    let combat = state.current.as_mut().filter(|combat| !combat.finished);
    let Some(combat) = combat else { return false };
    let layers = &state.debuff_layers;
    let mut total_duration: i64 = 0;
    let mut matched: usize = 0;
    for layer in layers {
        if layer.creature_hash == creature_hash && layer.power_id == power_id {
            total_duration += layer.duration;
            matched += 1;
        }
    }
    if matched == 0 || total_duration <= 0 {
        return false;
    }

    let mut allocated: i64 = 0;
    let mut seen: usize = 0;
    for layer in layers {
        if layer.creature_hash != creature_hash || layer.power_id != power_id {
            continue;
        }
        seen += 1;
        // The last matching layer takes the rounding residue.
        let share = if seen == matched {
            amount - allocated
        } else {
            (amount * layer.duration) / total_duration
        };
        if share > 0 {
            let index = get_or_create_card_kind(combat, layer.player, &layer.source_id, layer.kind);
            let card = &mut combat.cards[index];
            card.damage_dealt += share;
            card.dmg_attributed += share;
            allocated += share;
            assert_card_damage_segments(card);
        }
    }
    logs.push(format!(
        "  debuff damage {amount} from {power_id} split across {matched} layers\n"
    ));
    true
}

/// Kind is not part of the search: an id is unique per slot.
fn find_card(combat: &Combat, slot: SourceSlot, id: &str) -> Option<usize> {
    combat
        .cards
        .iter()
        .position(|card| card.player == slot && card.id == id)
}

pub fn get_or_create_card_kind(
    combat: &mut Combat,
    slot: SourceSlot,
    id: &str,
    kind: SourceKind,
) -> usize {
    if let Some(index) = find_card(combat, slot, id) {
        return index;
    }
    combat.cards.push(CardStat {
        player: slot,
        id: id.to_owned(),
        kind,
        ..CardStat::default()
    });
    combat.cards.len() - 1
}

pub fn get_or_create_card(combat: &mut Combat, slot: SourceSlot, id: &str) -> usize {
    get_or_create_card_kind(combat, slot, id, SourceKind::Card)
}

/// The Doom kill catch-all opts out: a kill with no recorded layer must
/// stay neutral rather than guess at an unrelated earlier hook.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AsyncFallback {
    Allow,
    Skip,
}

/// The `_in` variant takes the state directly so callers already holding
/// the borrow can resolve without a nested borrow.
pub fn resolve_card_in(
    state: &mut state::State,
    explicit_id: &str,
    slot: i32,
    explicit_slot: i32,
    async_fallback: AsyncFallback,
) -> Option<(usize, SourceSlot)> {
    let slot = state.slot_index(slot);
    let explicit_slot = clamp_source_slot(explicit_slot);
    let combat: &mut Combat = match state.current.as_mut() {
        Some(combat) if !combat.finished => combat,
        _ => return None,
    };
    let slot_state = &state.per_player[slot];
    let (index, row_slot) = if !explicit_id.is_empty()
        && Some(explicit_id) != slot_state.active_play_card_id.as_deref()
    {
        (
            get_or_create_card(combat, explicit_slot, explicit_id),
            explicit_slot,
        )
    } else if let Some((id, kind)) = slot_state.active_play_source.clone() {
        let row_slot = slot_state.active_play_source_slot;
        (
            get_or_create_card_kind(combat, row_slot, &id, kind),
            row_slot,
        )
    } else if let Some(top) = state.context_stack.last() {
        // Clone before the card append: the id must outlive the stack read.
        let id = top.id.clone();
        let kind = top.kind;
        let row_slot = top.slot;
        (
            get_or_create_card_kind(combat, row_slot, &id, kind),
            row_slot,
        )
    } else if let Some(i) = slot_state.orb_fallback {
        let source = &state.orb_sources[i];
        let id = source.id.clone();
        let kind = source.kind;
        (
            get_or_create_card_kind(combat, slot as SourceSlot, &id, kind),
            slot as SourceSlot,
        )
    } else if let Some(i) = slot_state.potion_fallback {
        let source = &state.orb_sources[i];
        let id = source.id.clone();
        let kind = source.kind;
        (
            get_or_create_card_kind(combat, slot as SourceSlot, &id, kind),
            slot as SourceSlot,
        )
    } else {
        // The most recent hook context outlives its postfix pop.
        if async_fallback == AsyncFallback::Skip {
            return None;
        }
        let last = state.last_source.clone()?;
        (
            get_or_create_card_kind(combat, last.slot, &last.id, last.kind),
            last.slot,
        )
    };
    debug_assert!(
        index < combat.cards.len(),
        "resolved card index {index} out of {} cards",
        combat.cards.len()
    );
    Some((index, row_slot))
}

/// The sibling tables arrive separately because the caller holds a
/// `&mut Combat` borrowed out of `State`.
fn resolve_damage_route(
    combat: &mut Combat,
    context_stack: &[ContextEntry],
    orb_sources: &[OrbSource],
    slot: &PlayerSlotState,
    caller_slot: SourceSlot,
    explicit_id: &str,
    explicit_slot: SourceSlot,
) -> Option<DamageRoute> {
    let (index, row_slot, indirect) =
        if !explicit_id.is_empty() && Some(explicit_id) != slot.active_play_card_id.as_deref() {
            (
                get_or_create_card(combat, explicit_slot, explicit_id),
                explicit_slot,
                false,
            )
        } else if let Some(i) = slot.orb_fallback {
            let source = &orb_sources[i];
            let id = source.id.clone();
            let kind = source.kind;
            (
                get_or_create_card_kind(combat, caller_slot, &id, kind),
                caller_slot,
                true,
            )
        } else if let Some((id, kind)) = slot.active_play_source.clone() {
            let row_slot = slot.active_play_source_slot;
            (
                get_or_create_card_kind(combat, row_slot, &id, kind),
                row_slot,
                false,
            )
        } else if let Some(top) = context_stack.last() {
            let id = top.id.clone();
            let kind = top.kind;
            let row_slot = top.slot;
            (
                get_or_create_card_kind(combat, row_slot, &id, kind),
                row_slot,
                kind == SourceKind::Power,
            )
        } else {
            let i = slot.potion_fallback?;
            let source = &orb_sources[i];
            let id = source.id.clone();
            let kind = source.kind;
            (
                get_or_create_card_kind(combat, caller_slot, &id, kind),
                caller_slot,
                false,
            )
        };
    debug_assert!(
        index < combat.cards.len(),
        "resolved damage route index {index} out of {} cards",
        combat.cards.len()
    );
    Some(DamageRoute {
        card_index: index,
        row_slot,
        indirect,
    })
}

/// The route chain first, then poison layers, then `last_source`.
pub fn resolve_damage_source_in(
    state: &mut state::State,
    explicit_id: &str,
    receiver_hash: u64,
    total: i64,
    slot: i32,
    explicit_slot: i32,
    logs: &mut Vec<String>,
) -> Option<DamageRoute> {
    {
        let slot_index = state.slot_index(slot);
        let combat = state.current.as_mut().filter(|combat| !combat.finished)?;
        if let Some(route) = resolve_damage_route(
            combat,
            &state.context_stack,
            &state.orb_sources,
            &state.per_player[slot_index],
            slot_index as SourceSlot,
            explicit_id,
            clamp_source_slot(explicit_slot),
        ) {
            return Some(route);
        }
    }
    if attribute_debuff_damage_in(state, receiver_hash, "POISON_POWER", total, logs) {
        return None;
    }
    let last_source = state.last_source.clone()?;
    let combat = state.current.as_mut().filter(|combat| !combat.finished)?;
    let index =
        get_or_create_card_kind(combat, last_source.slot, &last_source.id, last_source.kind);
    debug_assert!(
        index < combat.cards.len(),
        "last-source route index {index} out of {} cards",
        combat.cards.len()
    );
    Some(DamageRoute {
        card_index: index,
        row_slot: last_source.slot,
        indirect: last_source.kind == SourceKind::Power,
    })
}

/// A new attribution source invalidates the capture window.
pub fn clear_fallbacks_in(state: &mut state::State, slot: i32) {
    let slot = state.slot_index(slot);
    state.per_player[slot].orb_fallback = None;
    state.per_player[slot].potion_fallback = None;
}

/// Clears every slot's fallbacks for the turn/combat boundaries.
pub fn clear_all_fallbacks_in(state: &mut state::State) {
    for slot in &mut state.per_player {
        slot.orb_fallback = None;
        slot.potion_fallback = None;
    }
}

/// Same-source chunks merge only when both are modifier-free — merging
/// different modifier breakdowns would corrupt the proportional split.
pub fn block_pool_push_in(
    state: &mut state::State,
    id: &str,
    kind: SourceKind,
    base: i64,
    slot: i32,
    row_slot: i32,
) {
    let slot = state.slot_index(slot);
    let row_slot = clamp_source_slot(row_slot);
    let slot_state = &mut state.per_player[slot];
    // Two players' same-id chunks stay separate rows even in one pool.
    if slot_state.pending_block_contribs.is_empty()
        && let Some(entry) = slot_state
            .block_pool
            .iter_mut()
            .find(|e| e.mods.is_empty() && e.player == row_slot && e.id == id && e.kind == kind)
    {
        entry.remaining += base;
        entry.base_original += base;
        return;
    }
    if slot_state.block_pool.len() >= state::caps::BLOCK_POOL {
        fail!("block pool overflow");
        slot_state.pending_block_contribs.clear();
        return;
    }
    let mut entry = BlockEntry {
        id: id.to_owned(),
        kind,
        player: row_slot,
        base_original: base,
        base_consumed: 0,
        remaining: base,
        mods: Vec::new(),
    };
    for contrib in &slot_state.pending_block_contribs {
        if entry.mods.len() >= BlockEntry::MAX_MODS {
            fail!("block chunk modifier overflow");
            break;
        }
        entry.mods.push(BlockMod {
            id: contrib.id.clone(),
            kind: contrib.kind,
            player: contrib.player,
            original: contrib.amount,
            consumed: 0,
        });
        entry.remaining += contrib.amount;
    }
    slot_state.pending_block_contribs.clear();
    // A chunk with nothing to absorb is dropped rather than pooled.
    if entry.remaining <= 0 {
        return;
    }
    slot_state.block_pool.push(entry);
}

/// Consumes FIFO, splitting each slice cumulatively-proportionally.
pub fn block_pool_consume_in(state: &mut state::State, blocked: i64, slot: i32) -> i64 {
    let slot = state.slot_index(slot);
    let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
        return 0;
    };
    let pool = &mut state.per_player[slot].block_pool;
    let mut remaining = blocked;
    let mut credited: i64 = 0;
    // Counted before the loop: the bound allows up to one over-credited
    // point per modifier slice (the residue mechanism).
    let total_mod_slices: i64 = pool.iter().map(|entry| entry.mods.len() as i64).sum();
    let mut i = 0;
    while i < pool.len() && remaining > 0 {
        let take = pool[i].remaining.min(remaining);
        if take > 0 {
            credited += consume_block_chunk_in(combat, &mut pool[i], take);
            remaining -= take;
        }
        if pool[i].remaining <= 0 {
            pool.remove(i);
        } else {
            i += 1;
        }
    }
    state.per_player[slot].pending_block_contribs.clear();
    // `credited == blocked` does not hold in general: the shim can report
    // blocked damage the ledger never recorded, and the residue mechanism
    // over-credits by at most one point per modifier slice.
    debug_assert!(
        credited <= blocked + total_mod_slices,
        "block pool over-credited {credited} of {blocked} consumed (max {total_mod_slices} residue)"
    );
    credited
}

/// Consumes `take` from one chunk cumulatively-proportionally. Returns the
/// total credited.
fn consume_block_chunk_in(combat: &mut Combat, chunk: &mut BlockEntry, take: i64) -> i64 {
    let total = chunk.total_original();
    let consumed_after = (total - chunk.remaining) + take;
    let mut base_delta = if total > 0 {
        (chunk.base_original * consumed_after) / total - chunk.base_consumed
    } else {
        0
    };
    let mut allocated = base_delta;
    let mut mod_deltas = [0_i64; BlockEntry::MAX_MODS];
    for (j, m) in chunk.mods.iter().enumerate() {
        mod_deltas[j] = if total > 0 {
            (m.original * consumed_after) / total - m.consumed
        } else {
            0
        };
        allocated += mod_deltas[j];
    }
    base_delta += take - allocated;
    let mut credited: i64 = 0;
    if base_delta > 0 {
        let index = get_or_create_card_kind(combat, chunk.player, &chunk.id, chunk.kind);
        combat.cards[index].block_effective += base_delta;
        credited += base_delta;
    }
    for (j, m) in chunk.mods.iter_mut().enumerate() {
        m.consumed += mod_deltas[j];
        // A monotone floor sequence never passes its recorded original.
        debug_assert!(
            m.consumed <= m.original,
            "modifier consumed over its original"
        );
        if mod_deltas[j] > 0 {
            let index = get_or_create_card_kind(combat, m.player, &m.id, m.kind);
            combat.cards[index].blk_modifier += mod_deltas[j];
            credited += mod_deltas[j];
        }
    }
    chunk.base_consumed += base_delta;
    // The residue lands on the base slice; base consumption may exceed the
    // base's own share but never the chunk's total.
    debug_assert!(
        chunk.base_consumed <= chunk.total_original(),
        "base consumed past the chunk total"
    );
    chunk.remaining -= take;
    credited
}

/// Sign-extends a C# GetHashCode int to the u64 creature keys.
pub fn u64_from_hash(hash: i32) -> u64 {
    hash as u64
}

/// Splits `amount` across the recorded appliers of `power_id`; with no
/// recorded appliers the modifier itself gets the full amount.
pub fn split_over_appliers_in(
    state: &state::State,
    power_id: &str,
    fallback_kind: SourceKind,
    amount: i64,
    fallback_slot: i32,
) -> Vec<PendingContrib> {
    let fallback_slot = clamp_source_slot(fallback_slot);
    let mut total_recorded: i64 = 0;
    let mut matches: usize = 0;
    for entry in &state.power_sources {
        if entry.power_id == power_id {
            total_recorded += entry.amount.max(1);
            matches += 1;
        }
    }
    if matches == 0 {
        return vec![PendingContrib {
            id: power_id.to_owned(),
            kind: fallback_kind,
            player: fallback_slot,
            amount,
        }];
    }
    let mut out = Vec::with_capacity(matches);
    let mut allocated: i64 = 0;
    let mut seen: usize = 0;
    for entry in &state.power_sources {
        if entry.power_id != power_id {
            continue;
        }
        seen += 1;
        let share = if seen == matches {
            amount - allocated
        } else {
            (amount * entry.amount.max(1)) / total_recorded
        };
        allocated += share;
        if share <= 0 {
            continue;
        }
        out.push(PendingContrib {
            id: entry.source_id.clone(),
            kind: entry.kind,
            player: entry.player,
            amount: share,
        });
    }
    out
}

/// Route-agnostic: `dmg_direct` first, then `dmg_attributed`, because the
/// route is decided by the caller's resolution and is not visible here.
pub fn apply_pending_contribs_in(
    state: &mut state::State,
    attacker_index: usize,
    slot: i32,
    logs: &mut Vec<String>,
) {
    let slot = state.slot_index(slot);
    let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
        return;
    };
    debug_assert!(
        attacker_index < combat.cards.len(),
        "attacker index {attacker_index} out of {} cards",
        combat.cards.len()
    );
    if state.per_player[slot].pending_contribs.is_empty() {
        return;
    }
    // Take the queue before iterating: card appends below must not alias
    // it.
    let pending = std::mem::take(&mut state.per_player[slot].pending_contribs);
    let total_pending: i64 = pending.iter().map(|contrib| contrib.amount).sum();
    {
        let attacker = &mut combat.cards[attacker_index];
        // A direct-route hit grew dmg_direct; an attributed-route hit grew
        // dmg_attributed. Carve in that order, counting the share as
        // unblocked.
        debug_assert!(
            attacker.damage_dealt >= total_pending,
            "queued modifier share exceeds the attacker's damage"
        );
        let mut remaining = total_pending;
        let from_direct = attacker.dmg_direct.min(remaining);
        attacker.dmg_direct -= from_direct;
        remaining -= from_direct;
        let from_attributed = attacker.dmg_attributed.min(remaining);
        attacker.dmg_attributed -= from_attributed;
        remaining -= from_attributed;
        debug_assert_eq!(
            remaining, 0,
            "queued modifier share exceeds the hit's direct+attributed damage"
        );
        attacker.damage_dealt -= from_direct + from_attributed;
        assert_card_damage_segments(attacker);
    }
    for contrib in &pending {
        let index = get_or_create_card_kind(combat, contrib.player, &contrib.id, contrib.kind);
        let source = &mut combat.cards[index];
        source.damage_dealt += contrib.amount;
        source.dmg_modifier += contrib.amount;
        assert_card_damage_segments(source);
    }
    logs.push(format!(
        "  {total_pending} modifier damage attributed to sources\n"
    ));
}

#[cfg(test)]
mod tests;
