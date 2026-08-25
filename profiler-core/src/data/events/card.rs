//! Card plays and card instances: play start/finish and generation.
//! The play stack is per player slot (co-op plays interleave).
//! A row's `plays` counts the source's OWN triggers only — never the plays
//! of cards it generated — so the conservation invariant
//! `plays + generation_triggers == Σ row plays + generated_plays` stays
//! exact.

use crate::data::ledger;
use crate::data::persistence::event_log;
use crate::data::state::{
    Combat, GeneratedInstance, PlayerSlotState, STATE, SourceKind, SourceSlot, State, caps,
    clamp_source_slot,
};
use crate::fail;

pub fn card_play_started(
    card_id: &str,
    play_index: i32,
    play_count: i32,
    card_hash: i32,
    player_slot: i32,
) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // Reborrow so disjoint fields borrow field-precisely.
        let state = &mut *state;
        if state
            .current
            .as_mut()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            fail!("card_play_started called before init or outside a combat");
            return;
        }
        // Co-op plays interleave across slots.
        let slot = state.slot_index(player_slot);
        ledger::clear_fallbacks_in(state, player_slot);
        // Metadata read before the combat borrow for disjoint fields.
        let generator = load_play_metadata_in(state, card_hash);
        // Capture generation BEFORE consuming the entry: Anger cloning
        // Anger still makes this a generated play.
        let generated = generator.is_some();
        // A generated instance plays as its top-level non-generated
        // ancestor; nested generation collapses. The row slot is the
        // GENERATOR's recorded slot, else the playing player's own.
        let play_slot: SourceSlot = generator
            .as_ref()
            .map_or(slot as SourceSlot, |entry| entry.player);
        let play_source = generator.map_or_else(
            || (card_id.to_owned(), SourceKind::Card),
            |entry| (entry.source_id, entry.kind),
        );
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return;
        };
        let slot_state = &mut state.per_player[slot];
        slot_state.active_play_source_slot = play_slot;
        // The FIRST orb trigger credits the channeling source.
        slot_state.orb_first_trigger_used = false;
        let index =
            ledger::get_or_create_card_kind(combat, play_slot, &play_source.0, play_source.1);
        record_card_play_in(combat, slot_state, index, card_id, !generated);
        slot_state.play_depth += 1;
        combat.plays += 1;
        if generated {
            // Generated plays count toward the combat total but not the
            // generator row's plays.
            combat.generated_plays += 1;
        }
        event_log!("  play: {card_id} (play {}/{play_count})", play_index + 1);
        if play_source.0 != card_id {
            // Everything during the play credits the generator.
            event_log!(
                "    play credits generator {source_id} ({})",
                play_source.1.name(),
                source_id = play_source.0
            );
        }
    });
}

/// `count_play` is false for generated instances: their play must not
/// inflate the generator's own `plays`.
fn record_card_play_in(
    combat: &mut Combat,
    slot_state: &mut PlayerSlotState,
    index: usize,
    card_id: &str,
    count_play: bool,
) {
    let card = &mut combat.cards[index];
    if count_play {
        card.plays += 1;
    }
    // Everything during this play attributes to exactly this source.
    let (source_id, source_kind) = (card.id.clone(), card.kind);
    slot_state.active_play_source = Some((source_id, source_kind));
    // The resolution chain must ignore the card's own id.
    slot_state.active_play_card_id = Some(card_id.to_owned());
}

fn load_play_metadata_in(state: &State, card_hash: i32) -> Option<GeneratedInstance> {
    if card_hash != 0 {
        state
            .generated_instances
            .iter()
            .find(|e| e.hash == card_hash)
            .cloned()
    } else {
        None
    }
}

pub fn card_play_finished(player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state
            .current
            .as_ref()
            .is_some_and(|combat| !combat.finished)
        {
            return;
        }
        let slot_state = state.slot_state_mut(player_slot);
        slot_state.play_depth = slot_state.play_depth.saturating_sub(1);
        if slot_state.play_depth == 0 {
            slot_state.active_play_source = None;
            slot_state.active_play_source_slot = 0;
            slot_state.active_play_card_id = None;
        }
    });
}

/// A non-card generator counts each generation as a trigger; card-kind
/// generators count their own plays instead.
pub fn card_generated(card_hash: i32, source_id: &str, source_kind: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || card_hash == 0 {
            return;
        }
        let (resolved_source, kind) = resolve_cause_in(&state, source_id, source_kind);
        let Some(resolved_source) = resolved_source else {
            event_log!("  card generated with no attribution source");
            return;
        };
        // The creator is a real player, but the value only ever feeds row
        // keys, so the pure clamp (no per_player growth) suffices.
        let player = clamp_source_slot(player_slot);
        // A non-card generator has no plays of its own to count: each
        // generation it resolves to is one trigger. Card-kind generators
        // count their own plays instead, so nothing is booked here for
        // them.
        if kind != SourceKind::Card
            && let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished)
        {
            let index = ledger::get_or_create_card_kind(combat, player, &resolved_source, kind);
            combat.cards[index].plays += 1;
            combat.generation_triggers += 1;
        }
        let existing = state
            .generated_instances
            .iter()
            .position(|e| e.hash == card_hash);
        match existing {
            Some(i) => {
                state.generated_instances[i] = GeneratedInstance {
                    hash: card_hash,
                    source_id: resolved_source.clone(),
                    kind,
                    player,
                };
            }
            None => {
                if state.generated_instances.len() >= caps::GENERATED_INSTANCES {
                    fail!("generated instance table overflow");
                    return;
                }
                state.generated_instances.push(GeneratedInstance {
                    hash: card_hash,
                    source_id: resolved_source.clone(),
                    kind,
                    player,
                });
            }
        }
        event_log!(
            "  card generated, generator: {resolved_source} ({})",
            kind.name()
        );
    });
}

/// Explicit source, innermost context, ambient play source, potion
/// fallback, then `last_source`.
fn resolve_cause_in(
    state: &State,
    source_id: &str,
    source_kind: i32,
) -> (Option<String>, SourceKind) {
    let ambient = state.ambient_slot();
    let mut resolved_source: Option<String> = None;
    let mut kind = SourceKind::Card;
    if !source_id.is_empty() {
        resolved_source = Some(source_id.to_owned());
        kind = SourceKind::from_c(source_kind);
    } else if let Some(top) = state.context_stack.last().cloned() {
        if top.kind == SourceKind::Power {
            // The causing "source" is the power itself; resolve it to the
            // source that applied the power (with that source's kind).
            if let Some(applier) = state.power_sources.iter().find(|e| e.power_id == top.id) {
                resolved_source = Some(applier.source_id.clone());
                kind = applier.kind;
            } else {
                resolved_source = Some(top.id);
                kind = top.kind;
            }
        } else {
            resolved_source = Some(top.id);
            kind = top.kind;
        }
    } else if let Some((id, play_kind)) = state
        .per_player
        .get(ambient)
        .and_then(|slot| slot.active_play_source.clone())
    {
        resolved_source = Some(id);
        kind = play_kind;
    } else if let Some(i) = state
        .per_player
        .get(ambient)
        .and_then(|slot| slot.potion_fallback)
    {
        let source = &state.orb_sources[i];
        resolved_source = Some(source.id.clone());
        kind = source.kind;
    } else if let Some(last) = &state.last_source {
        // The async gap: the causing hook already returned (its context
        // popped at the first await), but `last_source` still names it.
        resolved_source = Some(last.id.clone());
        kind = last.kind;
    }
    (resolved_source, kind)
}
