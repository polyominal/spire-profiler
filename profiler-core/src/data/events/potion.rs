//! Potion use: the prefix/postfix pair that drives the potion fallback.
//! `CombatHistory.PotionUsed` fires only after `PotionModel.OnUse`
//! completes (inside `OnUseWrapper`) — too late for powers applied during
//! OnUse — so the shim prefixes `OnUseWrapper`: `potion_context_begin` sets
//! the fallback BEFORE the effects run. The `PotionUsed` postfix
//! (`potion_used`) books the use: counter bump plus a fresh hash-0 entry
//! re-pointing the fallback. Both pushes land in the shared `orb_sources`
//! table, bounded by `caps::ORB_SOURCES`.

use crate::data::ledger;
use crate::data::persistence::event_log;
use crate::data::state::{Combat, OrbSource, STATE, SourceKind, caps};
use crate::fail;

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
        if state.orb_sources.len() >= caps::ORB_SOURCES {
            fail!("orb source map overflow");
            return;
        }
        state.orb_sources.push(OrbSource {
            hash: 0,
            id: potion_id.to_owned(),
            kind: SourceKind::Potion,
        });
        state.per_player[slot].potion_fallback = Some(state.orb_sources.len() - 1);
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
            // The prefix already recorded a fallback entry.
            if state.orb_sources.len() >= caps::ORB_SOURCES {
                fail!("orb source map overflow");
                event_log!("  potion used: {potion_id}");
                return;
            }
            state.orb_sources.push(OrbSource {
                hash: 0,
                id: potion_id.to_owned(),
                kind: SourceKind::Potion,
            });
            let slot = state.slot_index(player_slot);
            state.per_player[slot].potion_fallback = Some(state.orb_sources.len() - 1);
        }
        event_log!("  potion used: {potion_id}");
    });
}
