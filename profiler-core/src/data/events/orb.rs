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
        let mut source_id: Option<String> = None;
        let mut kind = SourceKind::Card;
        if let Some(top) = state.context_stack.last() {
            source_id = Some(top.id.clone());
            kind = top.kind;
        } else if Combat::active(&state.current).is_some()
            && let Some((id, play_kind)) = state.per_player[slot].active_play_source.clone()
        {
            source_id = Some(id);
            kind = play_kind;
        } else if let Some(last_source) = state.last_source.clone() {
            source_id = Some(last_source.id);
            kind = last_source.kind;
        }
        let Some(source_id) = source_id else {
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
        if state.per_player[slot].active_play_source.is_some() {
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
