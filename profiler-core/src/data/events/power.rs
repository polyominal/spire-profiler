//! Powers, debuffs, and doom: FIFO debuff layers, modifier contributions
//! and mitigations, forge credits, and the strength-reduction proration.

use crate::data::ledger;
use crate::data::ledger::AsyncFallback;
use crate::data::persistence::event_log;
use crate::data::state::{
    DebuffLayer, DoomLayer, DoomTarget, EnemyHit, PowerSourceEntry, STATE, SourceKind, SourceSlot,
    State, StrReduction, TEAM_SLOT, caps,
};
use crate::fail;

/// Record which source applied it; enemy-applied powers are skipped.
pub fn power_applied(
    power_id: &str,
    amount: i32,
    creature_hash: u64,
    is_player: i32,
    player_slot: i32,
) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || amount <= 0 {
            return;
        }
        // A positive Strength delta on an enemy reverts reductions; handled
        // before source resolution.
        if is_player == 0 && power_id == "STRENGTH_POWER" {
            consume_str_reductions_in(&mut state, creature_hash, amount as i64);
            return;
        }
        // The owner player slot, else the ambient play.
        let applier_slot = if is_player != 0 {
            state.slot_index(player_slot)
        } else {
            state.ambient_slot()
        };
        // A slotless event can fire before any slot event grew it.
        let ambient = applier_slot;
        let mut source_id: Option<String> = None;
        let mut kind = SourceKind::Card;
        if let Some(top) = state.context_stack.last() {
            source_id = Some(top.id.clone());
            kind = top.kind;
        } else if let Some((id, play_kind)) = state
            .per_player
            .get(ambient)
            .and_then(|slot| slot.active_play_source.clone())
        {
            source_id = Some(id);
            kind = play_kind;
        } else if let Some(i) = state
            .per_player
            .get(ambient)
            .and_then(|slot| slot.potion_fallback)
        {
            // Potions run outside plays and contexts.
            let source = &state.orb_sources[i];
            source_id = Some(source.id.clone());
            kind = source.kind;
        }
        let Some(source_id) = source_id else {
            return;
        };
        // Self-doom targets the player and is skipped.
        if power_id == "DOOM_POWER" {
            if is_player != 0 {
                return;
            }
            record_doom_layer_in(
                &mut state,
                creature_hash,
                &source_id,
                kind,
                amount as i64,
                applier_slot as SourceSlot,
            );
            return;
        }
        record_power_source_in(
            &mut state,
            power_id,
            &source_id,
            kind,
            amount as i64,
            is_player,
            creature_hash,
            applier_slot as SourceSlot,
        );
    });
}

fn record_doom_layer_in(
    state: &mut State,
    creature_hash: u64,
    source_id: &str,
    kind: SourceKind,
    amount: i64,
    player: SourceSlot,
) {
    if state.doom_layers.len() >= caps::DOOM_LAYERS {
        fail!("doom layer table overflow");
        return;
    }
    state.doom_layers.push(DoomLayer {
        creature_hash,
        source_id: source_id.to_owned(),
        kind,
        player,
        amount,
    });
    event_log!("  doom layer: creature {creature_hash} (+{amount} from '{source_id}')");
}

/// Merges repeated applications by the same source AND slot.
#[allow(clippy::too_many_arguments)] // one param per recorded applier field
fn record_power_source_in(
    state: &mut State,
    power_id: &str,
    source_id: &str,
    kind: SourceKind,
    amount: i64,
    is_player: i32,
    creature_hash: u64,
    player: SourceSlot,
) {
    if let Some(entry) = state
        .power_sources
        .iter_mut()
        .find(|e| e.power_id == power_id && e.source_id == source_id && e.player == player)
    {
        entry.amount += amount;
        event_log!(
            "  power {power_id} +{amount} attributed to '{source_id}' (total {})",
            entry.amount
        );
        return;
    }
    if state.power_sources.len() >= caps::POWER_SOURCES {
        fail!("power source table overflow");
        return;
    }
    state.power_sources.push(PowerSourceEntry {
        power_id: power_id.to_owned(),
        source_id: source_id.to_owned(),
        kind,
        player,
        amount,
    });
    event_log!("  power {power_id} +{amount} attributed to '{source_id}'");

    // Debuff layer for duration debuffs applied to enemies.
    if is_player == 0 && crate::data::persistence::is_duration_debuff(power_id) {
        if state.debuff_layers.len() >= caps::DEBUFF_LAYERS {
            fail!("debuff layer table overflow");
            return;
        }
        state.debuff_layers.push(DebuffLayer {
            creature_hash,
            power_id: power_id.to_owned(),
            source_id: source_id.to_owned(),
            kind,
            player,
            duration: amount,
        });
        event_log!(
            "  debuff layer: {power_id} on creature {creature_hash} (+{amount} from '{source_id}')"
        );
    }
}

/// On the PLAYER the decrease consumes the recorded appliers FIFO so
/// expired Strength is never credited again.
pub fn power_decreased(
    power_id: &str,
    amount: i32,
    creature_hash: u64,
    is_player: i32,
    player_slot: i32,
) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || amount <= 0 {
            return;
        }
        state.slot_index(player_slot);
        if power_id == "STRENGTH_POWER" {
            if is_player != 0 {
                consume_player_strength_in(&mut state, amount as i64);
            } else if state
                .current
                .as_mut()
                .is_some_and(|combat| !combat.finished)
            {
                record_str_reduction_in(&mut state, creature_hash, amount as i64);
            }
            return;
        }
        if !crate::data::persistence::is_duration_debuff(power_id) {
            return;
        }
        ledger::consume_debuff_layers_in(&mut state, creature_hash, power_id, amount as i64);
        event_log!("  debuff {power_id} on creature {creature_hash} decreased by {amount}");
    });
}

/// Temporary powers record their +5 before later card applications, so
/// FIFO is the correct order; exhausted entries are removed outright.
fn consume_player_strength_in(state: &mut State, amount: i64) {
    let mut remaining = amount;
    let mut consumed = 0_i64;
    let mut i = 0;
    while i < state.power_sources.len() && remaining > 0 {
        if state.power_sources[i].power_id != "STRENGTH_POWER" {
            i += 1;
            continue;
        }
        let take = state.power_sources[i].amount.min(remaining);
        state.power_sources[i].amount -= take;
        consumed += take;
        remaining -= take;
        if state.power_sources[i].amount <= 0 {
            state.power_sources.remove(i);
        } else {
            i += 1;
        }
    }
    event_log!("  player strength -{amount}: {consumed} consumed from appliers (FIFO)");
}

pub fn doom_target_capture(creature_hash: i32, current_hp: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || current_hp <= 0 {
            return;
        }
        if state.doom_targets.len() >= caps::DOOM_TARGETS {
            fail!("doom target table overflow");
            return;
        }
        let hash = ledger::u64_from_hash(creature_hash);
        state.doom_targets.push(DoomTarget {
            creature_hash: hash,
            hp: current_hp as i64,
        });
        event_log!("  doom target capture: creature {hash} at {current_hp} hp");
    });
}

/// Attribute each captured target's HP to the Doom layers FIFO.
pub fn doom_kills_completed() {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return;
        }
        if state
            .current
            .as_mut()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            state.doom_targets.clear();
            return;
        }
        // Drain up front for disjoint borrows.
        let targets = std::mem::take(&mut state.doom_targets);
        let count = targets.len();
        for target in &targets {
            if !attribute_doom_target_in(&mut state, target.creature_hash, target.hp) {
                return;
            }
        }
        event_log!("  doom kills completed, {count} targets attributed");
    });
}

/// First against the matching Doom layers FIFO, then to the active context
/// or a DOOM catch-all entry.
fn attribute_doom_target_in(state: &mut State, creature_hash: u64, hp: i64) -> bool {
    let mut remaining = hp;
    let mut i = 0;
    while i < state.doom_layers.len() && remaining > 0 {
        let layer = &state.doom_layers[i];
        if layer.creature_hash != creature_hash {
            i += 1;
            continue;
        }
        let take = layer.amount.min(remaining);
        let (source_id, kind, player) = (layer.source_id.clone(), layer.kind, layer.player);
        {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return false;
            };
            if let Some(index) = ledger::get_or_create_card_kind(combat, player, &source_id, kind) {
                let card = &mut combat.cards[index];
                card.damage_dealt += take;
                card.dmg_attributed += take;
                ledger::assert_card_damage_segments(card);
            }
        }
        remaining -= take;
        state.doom_layers[i].amount -= take;
        if state.doom_layers[i].amount <= 0 {
            state.doom_layers.remove(i);
        } else {
            i += 1;
        }
    }
    if remaining > 0 {
        // The async-gap fallback is SKIPPED: a kill with no recorded layer
        // has an unknown source, and guessing the most recent hook would
        // misattribute it.
        let ambient = state.ambient_slot() as i32;
        let index = ledger::resolve_card_in(state, "", ambient, ambient, AsyncFallback::Skip)
            .map(|(index, _)| index);
        match index {
            Some(index) => {
                let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                    return false;
                };
                let card = &mut combat.cards[index];
                card.damage_dealt += remaining;
                card.dmg_attributed += remaining;
                ledger::assert_card_damage_segments(card);
            }
            None => {
                let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                    return false;
                };
                if let Some(index) =
                    ledger::get_or_create_card_kind(combat, TEAM_SLOT, "DOOM", SourceKind::Card)
                {
                    let card = &mut combat.cards[index];
                    card.damage_dealt += remaining;
                    card.dmg_attributed += remaining;
                    ledger::assert_card_damage_segments(card);
                }
            }
        }
    }
    true
}

/// Queue it on the DEALER's slot for attribution when the hit lands.
pub fn damage_modifier_contribution(
    modifier_id: &str,
    kind: i32,
    contribution: i32,
    player_slot: i32,
) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || contribution <= 0 {
            return;
        }
        let mod_kind = if kind == 1 {
            SourceKind::Relic
        } else {
            SourceKind::Power
        };
        // Each share carries its applier's slot; the no-applier fallback
        // rides the DEALER's slot.
        let shares = ledger::split_over_appliers_in(
            &state,
            modifier_id,
            mod_kind,
            contribution as i64,
            player_slot,
        );
        let slot = state.slot_index(player_slot);
        let mut count = 0usize;
        for share in shares {
            if state.per_player[slot].pending_contribs.len() >= caps::PENDING_CONTRIBS {
                fail!("pending modifier contribution overflow");
                break;
            }
            state.per_player[slot].pending_contribs.push(share);
            count += 1;
        }
        event_log!("  modifier {modifier_id} +{contribution} split across {count} appliers");
    });
}

/// Queue it on the GAINER's slot to attach to the next block chunk.
pub fn block_modifier_contribution(
    modifier_id: &str,
    kind: i32,
    contribution: i32,
    player_slot: i32,
) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || contribution <= 0 {
            return;
        }
        let mod_kind = if kind == 1 {
            SourceKind::Relic
        } else {
            SourceKind::Power
        };
        // Same applier-slot stamping as the damage path.
        let shares = ledger::split_over_appliers_in(
            &state,
            modifier_id,
            mod_kind,
            contribution as i64,
            player_slot,
        );
        let slot = state.slot_index(player_slot);
        let mut count = 0usize;
        for share in shares {
            if state.per_player[slot].pending_block_contribs.len() >= caps::PENDING_BLOCK_CONTRIBS {
                fail!("pending block modifier contribution overflow");
                break;
            }
            state.per_player[slot].pending_block_contribs.push(share);
            count += 1;
        }
        event_log!("  block modifier {modifier_id} +{contribution} split across {count} appliers");
    });
}

/// Credited to the FIFO head source of the enemy's WEAK_POWER layers.
pub fn weak_mitigation(prevented: i32, dealer_hash: u64) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || prevented <= 0 {
            return;
        }
        if state
            .current
            .as_mut()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            return;
        }
        let layer = state
            .debuff_layers
            .iter()
            .find(|l| l.creature_hash == dealer_hash && l.power_id == "WEAK_POWER")
            .map(|l| (l.source_id.clone(), l.kind, l.player));
        if let Some((source_id, kind, player)) = layer {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            if let Some(index) = ledger::get_or_create_card_kind(combat, player, &source_id, kind) {
                combat.cards[index].mitigate_debuff += prevented as i64;
                event_log!("  weak mitigation +{prevented} credited to '{source_id}'");
            }
        } else {
            event_log!("  weak mitigation +{prevented} with no recorded layer (uncredited)");
        }
    });
}

/// Split the credit across the buff's recorded appliers.
pub fn buff_mitigation(power_id: &str, prevented: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || prevented <= 0 {
            return;
        }
        if state
            .current
            .as_mut()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            return;
        }
        let ambient = state.ambient_slot() as i32;
        let shares = ledger::split_over_appliers_in(
            &state,
            power_id,
            SourceKind::Power,
            prevented as i64,
            ambient,
        );
        let count = shares.len();
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return;
        };
        for share in shares {
            if let Some(index) =
                ledger::get_or_create_card_kind(combat, share.player, &share.id, share.kind)
            {
                combat.cards[index].mitigate_buff += share.amount;
            }
        }
        event_log!("  buff mitigation {power_id} +{prevented} split across {count} appliers");
    });
}

/// `player_slot` is the forging player; the row keys at it.
pub fn forge(source_id: &str, source_kind: i32, amount: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || amount <= 0 {
            return;
        }
        if state
            .current
            .as_mut()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            return;
        }
        let kind = SourceKind::from_c(source_kind);
        // The explicit kind matters for relic/power forges (a bare
        // resolve_card would create a kind-0 entry). The row keys at the
        // forging player's slot.
        let index = if !source_id.is_empty() {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            let row_slot = crate::data::state::clamp_source_slot(player_slot);
            ledger::get_or_create_card_kind(combat, row_slot, source_id, kind)
        } else {
            ledger::resolve_card_in(
                &mut state,
                "",
                player_slot,
                player_slot,
                AsyncFallback::Allow,
            )
            .map(|(index, _)| index)
        };
        match index {
            Some(index) => {
                let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                    return;
                };
                combat.cards[index].forge += amount as i64;
                let id = combat.cards[index].id.clone();
                event_log!("  forge +{amount} attributed to '{id}'");
            }
            None => event_log!("  forge +{amount} attributed to nothing"),
        }
    });
}

/// Captured by the ModifyDamage prefix, consumed by the next hit to
/// prorate Strength-reduction mitigation.
pub fn enemy_hit_context(base_damage: i32, dealer_str: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return;
        }
        state.enemy_hit = Some(EnemyHit {
            base: base_damage as i64,
            str: dealer_str as i64,
        });
    });
}

/// Credits the Strength-reduction mitigation for a player-received hit from
/// `dealer_hash`.
pub(super) fn apply_str_mitigation_in(state: &mut State, dealer_hash: u64) {
    if dealer_hash == 0 {
        return;
    }
    let mut total_reduction: i64 = 0;
    let mut matched: usize = 0;
    for r in &state.str_reductions {
        if r.creature_hash == dealer_hash {
            total_reduction += r.amount;
            matched += 1;
        }
    }
    if matched == 0 || total_reduction <= 0 {
        return;
    }
    // The captured hit is consumed with the mitigation: a stale capture
    // (intent-display recalcs) mis-prorates at most this one hit.
    let Some(hit) = state.enemy_hit.take() else {
        return;
    };
    let hypothetical = (hit.base + hit.str + total_reduction).max(0);
    let effective = total_reduction.min(hypothetical);
    if effective <= 0 {
        return;
    }
    let combat = state.current.as_mut().filter(|combat| !combat.finished);
    let Some(combat) = combat else { return };
    let mut allocated: i64 = 0;
    let mut seen: usize = 0;
    for r in &state.str_reductions {
        if r.creature_hash != dealer_hash {
            continue;
        }
        seen += 1;
        let share = if seen == matched {
            effective - allocated
        } else {
            (effective * r.amount) / total_reduction
        };
        if share > 0
            && let Some(index) =
                ledger::get_or_create_card_kind(combat, r.player, &r.source_id, r.kind)
        {
            combat.cards[index].mitigate_str += share;
            allocated += share;
        }
    }
    event_log!("  str reduction mitigated {effective} across {matched} sources");
}

/// The reducer's slot is captured at reduction time; the credit fires
/// later, with no ambient play.
fn record_str_reduction_in(state: &mut State, creature_hash: u64, amount: i64) {
    let ambient = state.ambient_slot() as i32;
    let Some((index, row_slot)) =
        ledger::resolve_card_in(&mut *state, "", ambient, ambient, AsyncFallback::Allow)
    else {
        return;
    };
    let card = state
        .current
        .as_ref()
        .and_then(|combat| combat.cards.get(index));
    let Some(card) = card else { return };
    let (card_id, card_kind) = (card.id.clone(), card.kind);
    for r in &mut state.str_reductions {
        if r.creature_hash == creature_hash && r.source_id == card_id && r.player == row_slot {
            r.amount += amount;
            return;
        }
    }
    if state.str_reductions.len() >= caps::STR_REDUCTIONS {
        fail!("str reduction table overflow");
        return;
    }
    state.str_reductions.push(StrReduction {
        creature_hash,
        source_id: card_id.clone(),
        kind: card_kind,
        player: row_slot,
        amount,
    });
    event_log!("  str reduction: creature {creature_hash} -{amount} by '{card_id}'");
}

/// An enemy's Strength went up: consume recorded reductions LIFO.
fn consume_str_reductions_in(state: &mut State, creature_hash: u64, amount: i64) {
    let mut remaining = amount;
    let mut i = state.str_reductions.len();
    while i > 0 && remaining > 0 {
        i -= 1;
        if state.str_reductions[i].creature_hash != creature_hash {
            continue;
        }
        let take = state.str_reductions[i].amount.min(remaining);
        state.str_reductions[i].amount -= take;
        remaining -= take;
        if state.str_reductions[i].amount <= 0 {
            state.str_reductions.remove(i);
        }
    }
}
