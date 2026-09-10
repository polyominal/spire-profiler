//! Combat boundaries release native provenance before persistence can reenter State.

use crate::data::persistence::{event_log, now_seconds, write_combat_file};
use crate::data::state::{Combat, CombatPhase, CombatResult, STATE};
use crate::{fail, marker};

pub fn combat_started(encounter_id: &str, encounter_type: &str) -> u64 {
    if !STATE.with(|cell| cell.borrow().initialized) {
        fail!("combat_started called before init");
        return 0;
    }
    let interrupted = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let combat = Combat::active_mut(&mut state.current)?;
        if combat.plays == 0 && combat.cards.is_empty() {
            return None;
        }
        combat.phase = CombatPhase::Finished(CombatResult::Interrupted);
        let finished = combat.clone();
        state.clear_combat_sources();
        Some(finished)
    });
    if let Some(combat) = interrupted {
        write_combat_file(&combat);
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.discard_combat();
        state.per_player.clear();
        let Some(seq) = state.next_combat_id.checked_add(1) else {
            fail!("combat IDs exhausted; combat not started");
            return 0;
        };
        state.next_combat_id = seq;
        let players = state
            .run_ctx
            .as_ref()
            .map(|run| run.players.clone())
            .unwrap_or_default();
        for player in &players {
            state.slot_index(i32::from(player.slot));
        }
        state.current = Some(Combat {
            seq,
            encounter_id: encounter_id.to_owned(),
            encounter_type: encounter_type.to_owned(),
            started_at: now_seconds(),
            run: state.run_ctx.as_ref().map(|run| run.run.clone()),
            players,
            ..Combat::default()
        });
        event_log!("combat {seq} started: {encounter_id} ({encounter_type})");
        u64::from(seq)
    })
}

pub fn combat_ended(combat_seq: u64) -> i32 {
    let staged = STATE.with(|cell| cell.borrow_mut().finished_combat(combat_seq));
    let Some(combat) = staged else { return 0 };
    if write_combat_file(&combat) {
        marker!("combat {} summary written", combat.seq);
    }
    1
}
