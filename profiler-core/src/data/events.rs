//! The event surface: the `spire_profiler_*` export bodies as plain `pub fn`s
//! taking Rust types; `abi.rs` wraps these in the C signatures.
//!
//! Borrowing discipline: every function holds the STATE borrow exactly once,
//! collects any log lines, releases the borrow, and only then calls
//! `append_log` (which re-borrows STATE — a nested borrow would panic).

use std::path::{Path, PathBuf};

use crate::data::ledger;
use crate::data::persistence::{append_log, ensure_data_dir, max_combat_id};
use crate::data::state::{
    ContextEntry, PlayerFilter, STATE, SourceKind, State, caps, clamp_source_slot,
};
use crate::fail;

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
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.initialized {
            return Vec::new();
        }
        // The conversion to a path happens once, here.
        let data_dir = PathBuf::from(data_dir);
        state.data_dir = data_dir.clone();
        state.log_path_full = data_dir.join("profiler.log");
        state.runs_dir_full = data_dir.join("runs");
        state.runs_path_full = data_dir.join("runs.jsonl");
        state.initialized = true;
        vec![format!(
            "profiler core initialized; data dir: {}\n",
            data_dir.display()
        )]
    });
    let _ = ensure_data_dir();
    // One past the highest id in the store's file names.
    STATE.with(|cell| cell.borrow_mut().next_combat_id = max_combat_id());
    for line in log_lines {
        append_log(line);
    }
    eprintln!(
        "[SpireProfiler] core initialized, data dir: {data_dir}",
        data_dir = data_dir.display()
    );
}

/// The innermost context is where damage and block without an explicit
/// source are attributed.
pub fn context_begin(source_id: &str, kind: i32, player_slot: i32) {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if !state.initialized || source_id.is_empty() {
            return Vec::new();
        }
        if state.context_stack.len() >= caps::CONTEXT_STACK {
            fail(format!(
                "context stack overflow ({}) entries",
                caps::CONTEXT_STACK
            ));
            return Vec::new();
        }
        let kind = SourceKind::from_c(kind);
        // The slot is a row key, not a per-player index.
        let slot = clamp_source_slot(player_slot);
        state.context_stack.push(ContextEntry {
            id: source_id.to_owned(),
            kind,
            slot,
        });
        // The stack must still be within bounds.
        debug_assert!(
            state.context_stack.len() <= caps::CONTEXT_STACK,
            "context stack grew past its cap"
        );
        state.last_source = Some(ContextEntry {
            id: source_id.to_owned(),
            kind,
            slot,
        });
        // The context is team-global but the fallbacks are per slot, so
        // only the AMBIENT slot's clear.
        let ambient = state.ambient_slot() as i32;
        ledger::clear_fallbacks_in(&mut state, ambient);
        vec![format!("  context begin: {source_id} ({})\n", kind.name())]
    });
    for line in log_lines {
        append_log(line);
    }
}

pub fn context_end() {
    let log_lines = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.context_stack.is_empty() {
            return Vec::new();
        }
        let top = state
            .context_stack
            .pop()
            .expect("context stack was just checked non-empty");
        vec![format!("  context end: {}\n", top.id)]
    });
    for line in log_lines {
        append_log(line);
    }
}

/// The combat panel's avatar press. `current` keeps the finished combat
/// between fights, so the row stays live then too; a no-op only with no
/// combat on record.
pub fn panel_filter_toggle(slot: u8) {
    let label = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current.as_ref()?;
        let next = state.player_filter.toggle(slot);
        state.player_filter = next;
        Some(if next == PlayerFilter::All {
            "all".to_owned()
        } else {
            format!("P{}", slot + 1)
        })
    });
    if let Some(label) = label {
        eprintln!("[SpireProfiler] panel filter: {label}");
    }
}

/// Clears module state so a test can start from a fresh core (the process
/// lifetime of the game would never need this).
pub fn test_reset() {
    STATE.with(|cell| *cell.borrow_mut() = State::default());
}
