//! Orb attribution: a channeled orb records its source, and an orb trigger
//! activates that source as the owner slot's fallback. The `orb_sources`
//! table stays global; which slot's fallback points at it is per-slot.

use crate::data::persistence::event_log;
use crate::data::state::{Combat, OrbSource, STATE, SourceKind, caps};
use crate::fail;

/// Innermost context, then the active card, then `last_source`.
pub fn orb_channeled(hash: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return;
        }
        let slot = state.slot_index(player_slot);
        let resolved: Option<(String, SourceKind)> = if let Some(top) = state.context_stack.last() {
            Some((top.id.clone(), top.kind))
        } else if Combat::active(&state.current).is_some()
            && let Some(play) = state.per_player[slot].active_play.clone()
        {
            Some((play.id, play.kind))
        } else {
            state.last_source.clone().map(|last| (last.id, last.kind))
        };
        let Some((source_id, kind)) = resolved else {
            event_log!("  orb channeled with no attribution source");
            return;
        };
        let existing = state
            .orb_sources
            .iter()
            .position(|source| source.hash == hash);
        match existing {
            Some(i) => {
                state.orb_sources[i] = OrbSource {
                    hash,
                    id: source_id.clone(),
                    kind,
                }
            }
            None => {
                if state.orb_sources.len() >= caps::ORB_SOURCES {
                    fail!("orb source map overflow");
                    return;
                }
                state.orb_sources.push(OrbSource {
                    hash,
                    id: source_id.clone(),
                    kind,
                });
            }
        }
        event_log!("  orb channeled, source: {source_id} ({})", kind.name());
    });
}

/// Activate its channeling source as the OWNER SLOT's fallback.
pub fn orb_context_begin(hash: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized {
            return;
        }
        let slot = state.slot_index(player_slot);
        // The slot's potion fallback must not capture the orb's damage.
        state.per_player[slot].potion_fallback = None;
        // Only the FIRST orb trigger credits the channeling source.
        if state.per_player[slot].active_play.is_some() {
            if state.per_player[slot].orb_first_trigger_used {
                state.per_player[slot].orb_fallback = None;
                event_log!("  orb trigger (later during play) credited to the card");
                return;
            }
            state.per_player[slot].orb_first_trigger_used = true;
        }
        match state
            .orb_sources
            .iter()
            .position(|source| source.hash == hash)
        {
            Some(i) => {
                state.per_player[slot].orb_fallback = Some(i);
                let source = &state.orb_sources[i];
                event_log!(
                    "  orb trigger, fallback: {} ({})",
                    source.id,
                    source.kind.name()
                );
            }
            None => {
                state.per_player[slot].orb_fallback = None;
                event_log!("  orb trigger with no recorded source");
            }
        }
    });
}
