//! Combat lifecycle and attribution: the turn boundary, the block pool, and
//! every damage/block credit, including the Osty defensive stack.

use super::power::apply_str_mitigation_in;
use crate::data::ledger;
use crate::data::ledger::AsyncFallback;
use crate::data::persistence::{event_log, now_seconds, write_combat_file};
use crate::data::state::{
    Combat, OstyEntry, PlayerSlotState, STATE, SourceKind, State, TEAM_SLOT, caps,
    clamp_source_slot,
};
use crate::{fail, marker};

fn assert_damage_segments(combat: &Combat) {
    for card in &combat.cards {
        ledger::assert_card_damage_segments(card);
    }
}

pub fn turn_started() {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let turns = {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            combat.turns += 1;
            combat.turns
        };
        // The boundary is team-wide: every slot's fallbacks clear.
        ledger::clear_all_fallbacks_in(&mut state);
        state.last_source = None;
        event_log!("  turn {turns} started");
    });
}

pub fn block_pool_clear(player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.slot_state_mut(player_slot).block_pool.clear();
    });
    event_log!("  block pool cleared");
}

/// Record the summoned Osty HP on the owner slot's defensive stack.
pub fn osty_summoned(source_id: &str, source_kind: i32, hp_amount: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || hp_amount <= 0 {
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
        let slot = state.slot_index(player_slot);
        let kind = SourceKind::from_c(source_kind);
        if state.per_player[slot].osty_stack.len() >= caps::OSTY_STACK {
            fail!("osty stack overflow");
            return;
        }
        let entry = if !source_id.is_empty() {
            // The shim's named source has no slot.
            Some((source_id.to_owned(), kind, clamp_source_slot(player_slot)))
        } else if let Some((index, row_slot)) = ledger::resolve_card_in(
            &mut state,
            "",
            player_slot,
            player_slot,
            AsyncFallback::Allow,
        ) {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            Some((
                combat.cards[index].id.clone(),
                combat.cards[index].kind,
                row_slot,
            ))
        } else {
            None
        };
        let Some((entry_id, entry_kind, entry_player)) = entry else {
            return;
        };
        state.per_player[slot].osty_stack.push(OstyEntry {
            id: entry_id.clone(),
            kind: entry_kind,
            player: entry_player,
            remaining: hp_amount as i64,
        });
        event_log!(
            "  osty summoned: +{hp_amount} hp from '{entry_id}' ({})",
            entry_kind.name()
        );
    });
}

/// Consume the owner slot's defensive stack LIFO; overflow credits the
/// "OSTY" entry itself.
fn absorb_osty_damage(damage: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || damage <= 0 {
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
        let slot = state.slot_index(player_slot);
        let mut remaining: i64 = damage as i64;
        let mut i = state.per_player[slot].osty_stack.len();
        while i > 0 && remaining > 0 {
            i -= 1;
            if state.per_player[slot].osty_stack[i].remaining <= 0 {
                continue;
            }
            let take = state.per_player[slot].osty_stack[i]
                .remaining
                .min(remaining);
            let (id, kind, player) = (
                state.per_player[slot].osty_stack[i].id.clone(),
                state.per_player[slot].osty_stack[i].kind,
                state.per_player[slot].osty_stack[i].player,
            );
            {
                let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                    return;
                };
                let index = ledger::get_or_create_card_kind(combat, player, &id, kind);
                combat.cards[index].block_effective += take;
            }
            state.per_player[slot].osty_stack[i].remaining -= take;
            remaining -= take;
            if state.per_player[slot].osty_stack[i].remaining <= 0 {
                state.per_player[slot].osty_stack.remove(i);
            }
        }
        if remaining > 0 {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            // The overflow has no player owner: key it at the TEAM slot.
            let index =
                ledger::get_or_create_card_kind(combat, TEAM_SLOT, "OSTY", SourceKind::Osty);
            combat.cards[index].block_effective += remaining;
        }
        event_log!(
            "  osty absorbed {damage} damage ({} from summon sources)",
            damage as i64 - remaining
        );
    });
}

/// The remaining unabsorbed HP is removed from the killer card's credit.
pub fn osty_killed(player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Reborrow so the combat and the stack borrow disjoint fields.
        let state = &mut *state;
        if !state.initialized {
            return;
        }
        let slot = state.slot_index(player_slot);
        let remaining: i64 = state.per_player[slot]
            .osty_stack
            .iter()
            .map(|e| e.remaining)
            .sum();
        if remaining > 0
            && let Some(combat) = state.current.as_mut()
            && let Some((id, kind)) = state.per_player[slot].active_play_source.clone()
        {
            // A generated instance's play credits its generator's slot.
            let row_slot = state.per_player[slot].active_play_source_slot;
            let index = ledger::get_or_create_card_kind(combat, row_slot, &id, kind);
            combat.cards[index].block_effective -= remaining;
            state.per_player[slot].osty_stack.clear();
            event_log!("  osty died: -{remaining} effective block on '{id}'");
            return;
        }
        state.per_player[slot].osty_stack.clear();
        event_log!("  osty killed, stack cleared");
    });
}

pub fn combat_started(encounter_id: &str, encounter_type: &str) {
    if !STATE.with(|cell| cell.borrow().initialized) {
        fail!("combat_started called before init");
        return;
    }
    // A combat that never reported its end is flushed as interrupted.
    let interrupted = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Finished combats are kept for the UI summary.
        let combat = state.current.as_mut().filter(|combat| !combat.finished)?;
        if combat.plays == 0 && combat.cards.is_empty() {
            return None;
        }
        combat.result = "interrupted".to_owned();
        Some(combat.clone())
    });
    if let Some(combat) = interrupted {
        write_combat_file(&combat);
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Seeded at init as one past the store's highest id.
        state.next_combat_id += 1;
        // Per-player transient state clears wholesale at the boundary.
        state.per_player.clear();
        ledger::clear_all_fallbacks_in(&mut state);
        state.last_source = None;
        state.orb_sources.clear();
        state.power_sources.clear();
        state.generated_instances.clear();
        state.doom_layers.clear();
        state.doom_targets.clear();
        state.debuff_layers.clear();
        state.str_reductions.clear();
        state.enemy_hit = None;
        let seq = state.next_combat_id;
        state.current = Some(Combat {
            seq,
            encounter_id: encounter_id.to_owned(),
            encounter_type: encounter_type.to_owned(),
            started_at: now_seconds(),
            run_seq: state.run_ctx.seq,
            run_character: state.run_ctx.character.clone(),
            run_ascension: state.run_ctx.ascension,
            run_game_mode: state.run_ctx.game_mode.clone(),
            run_seed: state.run_ctx.seed.clone(),
            // The roster is in-memory only.
            players: state.run_ctx.players.clone(),
            ..Combat::default()
        });
        event_log!("combat {seq} started: {encounter_id} ({encounter_type})");
    });
}

/// The field list mirrors the shim's C export. `card_source_slot` is the
/// row key of the explicit-card branch.
#[derive(Default)]
pub struct DamageDealt<'a> {
    pub total: i32,
    pub unblocked: i32,
    pub blocked: i32,
    pub card_source_id: &'a str,
    pub to_player: i32,
    pub receiver_hash: u64,
    pub osty_flag: i32,
    pub dealer_hash: u64,
    pub dealer_slot: i32,
    pub receiver_slot: i32,
    pub card_source_slot: i32,
}

pub fn damage_dealt(args: DamageDealt) {
    // Intent-display recalcs can queue contributions with no hit
    // following; every early-return branch drops them.
    let total = args.total;
    let receiver_slot = args.receiver_slot;
    let needs_osty_absorb = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        damage_dealt_in(&mut state, args)
    });
    // Every card's four segments must still decompose damage_dealt.
    STATE.with(|cell| {
        let state = cell.borrow();
        if let Some(combat) = state.current.as_ref().filter(|combat| !combat.finished) {
            assert_damage_segments(combat);
        }
    });
    if needs_osty_absorb {
        absorb_osty_damage(total, receiver_slot);
    }
}

fn damage_dealt_in(state: &mut State, args: DamageDealt) -> bool {
    if state
        .current
        .as_mut()
        .filter(|combat| !combat.finished)
        .is_none()
    {
        return false;
    }
    if args.osty_flag == 1 && args.to_player == 0 {
        record_osty_dealt_in(
            state,
            args.total,
            args.blocked,
            args.card_source_id,
            args.dealer_slot,
            args.card_source_slot,
        );
        return false;
    }
    // Osty absorbed: consume the owner slot's summon HP stack.
    if args.osty_flag == 2 {
        state
            .slot_state_mut(args.dealer_slot)
            .pending_contribs
            .clear();
        return true;
    }
    if args.to_player != 0 {
        record_damage_to_player_in(
            state,
            args.total,
            args.unblocked,
            args.blocked,
            args.card_source_id,
            args.dealer_hash,
            args.receiver_hash,
            args.dealer_slot,
            args.receiver_slot,
            args.card_source_slot,
        );
        return false;
    }
    record_enemy_damage_in(
        state,
        args.total,
        args.unblocked,
        args.blocked,
        args.card_source_id,
        args.receiver_hash,
        args.dealer_slot,
        args.card_source_slot,
    );
    false
}

/// Resolve against the DEALER's slot state, credit, apply the queued
/// splits.
#[allow(clippy::too_many_arguments)]
fn record_enemy_damage_in(
    state: &mut State,
    total: i32,
    unblocked: i32,
    blocked: i32,
    card_source_id: &str,
    receiver_hash: u64,
    dealer_slot: i32,
    card_source_slot: i32,
) {
    if let Some(route) = ledger::resolve_damage_source_in(
        state,
        card_source_id,
        receiver_hash,
        total as i64,
        dealer_slot,
        card_source_slot,
    ) {
        let index = route.card_index;
        {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            let card = &mut combat.cards[index];
            card.damage_dealt += total as i64;
            if route.indirect {
                card.dmg_attributed += total as i64;
            } else {
                card.dmg_direct += total as i64;
            }
            card.damage_blocked += blocked as i64;
            ledger::assert_card_damage_segments(card);
        }
        ledger::apply_pending_contribs_in(state, index, dealer_slot);
        let id = {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            combat.cards[index].id.clone()
        };
        event_log!(
            "  damage {total} ({blocked} blocked, {unblocked} unblocked) attributed to '{id}'"
        );
    } else {
        // Either way the dealer's queued contributions drop.
        state.slot_state_mut(dealer_slot).pending_contribs.clear();
    }
}

/// Credit the card source directly, keyed at the card's owner slot.
#[allow(clippy::too_many_arguments)]
fn record_osty_dealt_in(
    state: &mut State,
    total: i32,
    blocked: i32,
    source: &str,
    dealer_slot: i32,
    card_source_slot: i32,
) {
    if !source.is_empty() {
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return;
        };
        let row_slot = clamp_source_slot(card_source_slot);
        let index = ledger::get_or_create_card(combat, row_slot, source);
        let card = &mut combat.cards[index];
        card.damage_dealt += total as i64;
        card.dmg_direct += total as i64;
        card.damage_blocked += blocked as i64;
        ledger::assert_card_damage_segments(card);
    }
    state.slot_state_mut(dealer_slot).pending_contribs.clear();
    event_log!("  osty dealt {total} damage via '{source}'");
}

/// The death flag is NOT set here: the shim's Kill patch fires `player_died`
/// on every death path.
#[allow(clippy::too_many_arguments)] // one param per ABI scalar the branch needs
fn record_damage_to_player_in(
    state: &mut State,
    total: i32,
    unblocked: i32,
    blocked: i32,
    card_source_id: &str,
    dealer_hash: u64,
    receiver_hash: u64,
    dealer_slot: i32,
    receiver_slot: i32,
    card_source_slot: i32,
) {
    {
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return;
        };
        combat.damage_received += total as i64;
    }
    if blocked > 0 {
        let _ = ledger::block_pool_consume_in(state, blocked as i64, receiver_slot);
    }
    // An explicit self hit or a dealer-less card-sourced HP loss.
    let is_self_hit = (dealer_hash != 0 && dealer_hash == receiver_hash)
        || (dealer_hash == 0 && !card_source_id.is_empty());
    if is_self_hit && unblocked > 0 {
        if let Some((index, _)) = ledger::resolve_card_in(
            state,
            card_source_id,
            dealer_slot,
            card_source_slot,
            AsyncFallback::Allow,
        ) {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            combat.cards[index].self_damage += unblocked as i64;
            let id = combat.cards[index].id.clone();
            event_log!("  self damage {unblocked} attributed to '{id}'");
        }
    } else {
        // Enemy hit: credit Strength-reduction mitigation.
        apply_str_mitigation_in(state, dealer_hash);
    }
    // Unlanded queue policy: the attacker's contributions drop.
    state.slot_state_mut(dealer_slot).pending_contribs.clear();
    event_log!("  player took {total} damage ({unblocked} unblocked)");
}

/// The base portion of a block gain: the gain minus the queued ModifyBlock
/// contributions, clamped at zero.
fn block_base_after_mods(slot_state: &PlayerSlotState, amount: i64) -> i64 {
    let mod_total: i64 = slot_state
        .pending_block_contribs
        .iter()
        .map(|contrib| contrib.amount)
        .sum();
    (amount - mod_total).max(0)
}

pub fn block_gained(amount: i32, card_id: &str, player_slot: i32, source_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Reborrow the RefCell guard so the combat and the slot's
        // block-pool/pending-contrib fields can be borrowed on disjoint
        // fields.
        let state = &mut *state;
        {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            combat.block_total += amount as i64;
        }
        let slot = state.slot_index(player_slot);
        // `source_slot` is the row key of the explicit-card branch: an ally
        // block (a card owned by one player gaining block on another's
        // creature) keys its row at the owner's slot.
        let Some((index, row_slot)) = ledger::resolve_card_in(
            state,
            card_id,
            player_slot,
            source_slot,
            AsyncFallback::Allow,
        ) else {
            state.per_player[slot].pending_block_contribs.clear();
            event_log!("  block +{amount} attributed to nothing (no active card)");
            return;
        };
        let base = block_base_after_mods(&state.per_player[slot], amount as i64);
        let (id, kind) = {
            let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
                return;
            };
            combat.cards[index].block_gained += amount as i64;
            (combat.cards[index].id.clone(), combat.cards[index].kind)
        };
        // The chunk remembers the SOURCE's row slot: the pool is the
        // receiver's (per-slot), but an ally block credits the owner's row
        // when consumed.
        ledger::block_pool_push_in(state, &id, kind, base, player_slot, row_slot as i32);
        event_log!("  block +{amount} attributed to '{id}'");
    });
}

/// Every death path funnels through Kill, so this is the single source of
/// the per-slot death flag.
pub fn player_died(player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return;
        }
        let slot = state.slot_index(player_slot);
        state.per_player[slot].died = true;
        event_log!("  player {player_slot} died");
    });
}

pub fn combat_ended() {
    // write_combat_file re-borrows STATE, so the finished record is staged
    // and written after the borrow releases.
    let (seq, record) = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // The game only loses when EVERY player is dead, so the record is a
        // defeat iff every slot that appeared has its death flag set. Only
        // PLAYER entries participate: the TEAM pseudo-slot (index 4, grown
        // only by a corrupt wire slot) has no creature and can never die.
        let team_defeat = !state.per_player.is_empty()
            && state
                .per_player
                .iter()
                .take(caps::MAX_PLAYERS)
                .all(|slot| slot.died);
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return (None, None);
        };
        if team_defeat {
            combat.result = "defeat".to_owned();
        }
        combat.finished = true;
        let seq = combat.seq;
        let record = combat.clone();
        (Some(seq), Some(record))
    });
    if let Some(combat) = record {
        write_combat_file(&combat);
    }
    if let Some(seq) = seq {
        marker!("combat {seq} summary written");
    }
}
