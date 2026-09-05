//! Potion use: the prefix/postfix pair that drives the potion fallback.
//! `CombatHistory.PotionUsed` fires only after `PotionModel.OnUse`
//! completes (inside `OnUseWrapper`) — too late for powers applied during
//! OnUse — so the shim prefixes `OnUseWrapper`: `potion_context_begin` sets
//! the fallback BEFORE the effects run. The `PotionUsed` postfix
//! (`potion_used`) books the use: counter bump plus a fresh per-slot
//! fallback source.

use crate::data::ledger;
use crate::data::persistence::event_log;
use crate::data::state::{Combat, Fallback, PotionSource, STATE, SourceKind};

/// A postfix there records the fallback too late; each use is a fresh entry.
pub fn potion_context_begin(potion_id: &str, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || potion_id.is_empty() {
            return;
        }
        if Combat::active(&state.current).is_none() {
            return;
        }
        let slot = state.slot_index(player_slot);
        // A stale orb trigger would capture the potion's effects (the orb
        // fallback outranks the potion fallback in both chains).
        ledger::clear_fallbacks_in(&mut state, player_slot);
        state.per_player[slot].fallback = Some(Fallback::Potion(PotionSource {
            id: potion_id.to_owned(),
            kind: SourceKind::Potion,
        }));
        event_log!("  potion context begin: {potion_id}");
    });
}

pub fn potion_used(potion_id: &str, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(combat) = Combat::active_mut(&mut state.current) else {
            return;
        };
        combat.potions_used += 1;
        if !potion_id.is_empty() {
            let slot = state.slot_index(player_slot);
            state.per_player[slot].fallback = Some(Fallback::Potion(PotionSource {
                id: potion_id.to_owned(),
                kind: SourceKind::Potion,
            }));
        }
        event_log!("  potion used: {potion_id}");
    });
}
