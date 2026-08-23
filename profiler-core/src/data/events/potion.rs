//! Potion use: the prefix/postfix pair that drives the potion fallback
//! (`potion_context_begin` sets it BEFORE the effects run, `potion_used`
//! bumps the counter and refreshes it after).

use crate::data::ledger;
use crate::data::persistence::append_log;
use crate::data::state::{OrbSource, STATE, SourceKind, caps};
use crate::fail;

/// A postfix there records the fallback too late; each use is a fresh entry.
pub fn potion_context_begin(potion_id: &str, player_slot: i32) {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || potion_id.is_empty() {
            return Vec::new();
        }
        if state
            .current
            .as_ref()
            .filter(|combat| !combat.finished)
            .is_none()
        {
            return Vec::new();
        }
        let slot = state.slot_index(player_slot);
        // A stale orb trigger would capture the potion's effects (the orb
        // fallback outranks the potion fallback in both chains).
        ledger::clear_fallbacks_in(&mut state, player_slot);
        if state.orb_sources.len() >= caps::ORB_SOURCES {
            fail("orb source map overflow".to_owned());
            return Vec::new();
        }
        state.orb_sources.push(OrbSource {
            hash: 0,
            id: potion_id.to_owned(),
            kind: SourceKind::Potion,
        });
        state.per_player[slot].potion_fallback = Some(state.orb_sources.len() - 1);
        vec![format!("  potion context begin: {potion_id}\n")]
    });
    for line in log_lines {
        append_log(line);
    }
}

pub fn potion_used(potion_id: &str, player_slot: i32) {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(combat) = state.current.as_mut().filter(|combat| !combat.finished) else {
            return Vec::new();
        };
        combat.potions_used += 1;
        if !potion_id.is_empty() {
            // The prefix already recorded a fallback entry.
            if state.orb_sources.len() >= caps::ORB_SOURCES {
                fail("orb source map overflow".to_owned());
                return vec![format!("  potion used: {potion_id}\n")];
            }
            state.orb_sources.push(OrbSource {
                hash: 0,
                id: potion_id.to_owned(),
                kind: SourceKind::Potion,
            });
            let slot = state.slot_index(player_slot);
            state.per_player[slot].potion_fallback = Some(state.orb_sources.len() - 1);
        }
        vec![format!("  potion used: {potion_id}\n")]
    });
    for line in log_lines {
        append_log(line);
    }
}
