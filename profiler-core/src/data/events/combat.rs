//! Combat boundaries release native provenance before persistence can reenter State.

use std::ops::Deref;

use crate::data::persistence::{event_log, now_seconds, write_combat_file};
use crate::data::state::{Combat, CombatPhase, CombatResult, Label, ModelId, STATE};
use crate::{fail, marker};

/// A writer owns the staging lease only while no State guard is held.
pub struct FinishedCombat(Option<Combat>);

impl FinishedCombat {
    pub(crate) fn take() -> Option<Self> {
        STATE.with(|cell| {
            cell.borrow_mut()
                .finish_combat
                .take()
                .map(|value| Self(Some(value)))
        })
    }
}

impl Deref for FinishedCombat {
    type Target = Combat;
    fn deref(&self) -> &Combat {
        self.0
            .as_ref()
            .expect("the lease owns its staging buffer until Drop")
    }
}

impl Drop for FinishedCombat {
    fn drop(&mut self) {
        STATE.with(|cell| cell.borrow_mut().finish_combat = self.0.take());
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "keep validation, staging, and writer reborrows in lifecycle order"
)]
pub fn combat_started(encounter_id: &str, encounter_type: &str) -> u64 {
    if !STATE.with(|cell| cell.borrow().ready()) {
        fail!("combat_started called before init");
        return 0;
    }
    let (Ok(encounter_id), Ok(encounter_type)) = (
        ModelId::try_from(encounter_id),
        Label::try_from(encounter_type),
    ) else {
        fail!("combat metadata exceeds text limits or contains NUL; combat not started");
        return 0;
    };
    let interrupted = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.finish_combat.as_ref()?;
        let Some(combat) = Combat::active_mut(&mut state.current) else {
            return Some(false);
        };
        if combat.plays == 0 && combat.cards.is_empty() {
            return Some(false);
        }
        combat.phase = CombatPhase::Finished(CombatResult::Interrupted);
        let state = &mut *state;
        state
            .finish_combat
            .as_mut()
            .expect("staging availability checked before mutation")
            .clone_from(
                state
                    .current
                    .as_ref()
                    .expect("the interrupted combat is present"),
            );
        state.clear_combat_sources();
        Some(true)
    });
    match interrupted {
        None => {
            fail!("combat finish staging is leased; combat not started");
            return 0;
        }
        Some(true) => {
            let combat = FinishedCombat::take().expect("the staged interruption owns a buffer");
            write_combat_file(&combat);
        }
        Some(false) => {}
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.discard_combat();
        state.per_player.clear();
        let Some(previous) = state.next_combat_id else {
            fail!("combat ID scan failed; combat not started");
            return 0;
        };
        let Some(seq) = previous.checked_add(1) else {
            fail!("combat IDs exhausted; combat not started");
            return 0;
        };
        state.next_combat_id = Some(seq);
        let state = &mut *state;
        debug_assert!(
            state.current.storage.cards.capacity() >= crate::data::state::caps::COMBAT_CARDS
                && state.current.storage.players.capacity()
                    >= crate::data::state::caps::MAX_PLAYERS,
            "initialized combat storage must retain its row and roster capacity"
        );
        state.current.set(&Combat {
            seq,
            encounter_id,
            encounter_type,
            started_at: now_seconds(),
            run: state.run_ctx.as_ref().map(|run| run.run.clone()),
            ..Combat::default()
        });
        if let Some(run) = state.run_ctx.as_ref() {
            state.current.storage.players.clone_from(&run.players);
        }
        for index in 0..state.current.storage.players.len() {
            state.slot_index(i32::from(state.current.storage.players[index].slot));
        }
        event_log!(
            "combat {seq} started: {encounter_id} ({encounter_type})",
            encounter_id = state.current.storage.encounter_id,
            encounter_type = state.current.storage.encounter_type
        );
        u64::from(seq)
    })
}

pub fn combat_ended(combat_seq: u64) -> i32 {
    if !STATE.with(|cell| cell.borrow_mut().finished_combat(combat_seq)) {
        return 0;
    }
    let combat = FinishedCombat::take().expect("successful finalization leaves a staged combat");
    if write_combat_file(&combat) {
        marker!("combat {} summary written", combat.seq);
    }
    1
}
