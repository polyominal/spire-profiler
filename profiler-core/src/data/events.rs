//! The event surface: the `spire_profiler_*` export bodies as plain `pub fn`s
//! taking Rust types; `abi.rs` wraps these in the C signatures.
//!
//! Borrowing discipline: every event holds the STATE borrow while mutating
//! and may emit a trace line immediately; the `profiler.log` sink owns its
//! path and never re-enters STATE.
//! TODO: extend steady-state allocation freedom from trace emission to the
//! event-state representation.

use std::path::{Path, PathBuf};

use crate::data::ledger;
use crate::data::persistence::{
    bind_log_path, ensure_data_dir, event_log, max_combat_id, reset_log_sink,
};
use crate::data::state::{
    ContextEntry, PlayerFilter, STATE, SourceKind, State, caps, clamp_source_slot,
};
use crate::{fail, marker};

mod card;
mod combat;
mod orb;
mod potion;
mod power;
mod run;
mod self_test;

#[cfg(test)]
mod tests;

pub use card::{card_generated, card_play_finished, card_play_started};
pub use combat::{
    DamageDealt, block_gained, block_pool_clear, combat_ended, combat_started, damage_dealt,
    osty_killed, osty_summoned, player_died, turn_started,
};
pub use orb::{orb_channeled, orb_context_begin};
pub use potion::{potion_context_begin, potion_used};
pub use power::{
    block_modifier_contribution, buff_mitigation, damage_modifier_contribution,
    doom_kills_completed, doom_target_capture, enemy_hit_context, forge, power_applied,
    power_decreased, weak_mitigation,
};
pub use run::{
    run_ended, run_history_clear, run_history_select, run_started, run_suspended, set_run_meta,
};
pub use self_test::self_test;

pub fn init(data_dir: &Path) {
    if STATE.with(|cell| cell.borrow().initialized) {
        return;
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        // The conversion to a path happens once, here.
        let data_dir = PathBuf::from(data_dir);
        state.data_dir = data_dir.clone();
        state.runs_dir_full = data_dir.join("runs");
        state.runs_path_full = data_dir.join("runs.jsonl");
        state.initialized = true;
    });
    bind_log_path(&data_dir.join("profiler.log"));
    let _ = ensure_data_dir();
    // Seeded at the store's highest id; combat start increments before
    // taking it, so the first new combat gets max+1.
    STATE.with(|cell| cell.borrow_mut().next_combat_id = max_combat_id());
    event_log!(
        "profiler core initialized; data dir: {}",
        data_dir.display()
    );
    marker!("core initialized, data dir: {}", data_dir.display());
}

/// The innermost context is where damage and block without an explicit
/// source are attributed.
pub fn context_begin(source_id: &str, kind: i32, player_slot: i32) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || source_id.is_empty() {
            return;
        }
        if state.context_stack.len() >= caps::CONTEXT_STACK {
            fail!("context stack overflow ({}) entries", caps::CONTEXT_STACK);
            return;
        }
        let kind = SourceKind::from_c(kind);
        // The slot is a row key, not a per-player index.
        let slot = clamp_source_slot(player_slot);
        state.context_stack.push(ContextEntry {
            id: source_id.to_owned(),
            kind,
            slot,
        });
        state.last_source = Some(ContextEntry {
            id: source_id.to_owned(),
            kind,
            slot,
        });
        // The context is team-global but the fallbacks are per slot, so
        // only the AMBIENT slot's clear.
        let ambient = state.ambient_slot() as i32;
        ledger::clear_fallbacks_in(&mut state, ambient);
        event_log!("  context begin: {source_id} ({})", kind.name());
    });
}

pub fn context_end() {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(top) = state.context_stack.pop() {
            event_log!("  context end: {}", top.id);
        }
    });
}

/// The combat panel's avatar press. `current` keeps the finished combat
/// between fights, so the row stays live then too; a no-op only with no
/// combat on record.
pub fn panel_filter_toggle(slot: u8) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.current.is_none() {
            return;
        }
        state.player_filter = state.player_filter.toggle(slot);
        match state.player_filter {
            PlayerFilter::Player(_) => marker!("panel filter: P{}", slot + 1),
            PlayerFilter::All => marker!("panel filter: all"),
        }
    });
}

/// Clears module state so a test can start from a fresh core (the process
/// lifetime of the game would never need this).
pub fn test_reset() {
    STATE.with(|cell| *cell.borrow_mut() = State::default());
    reset_log_sink();
}
