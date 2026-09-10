//! Native events carry explicit epochs and immutable source-transfer values.

use std::path::Path;

use crate::data::persistence::{
    bind_log_path, ensure_data_dir, event_log, max_combat_id, reset_log_sink,
};
use crate::data::state::{PlayerFilter, STATE, State, StorePaths};
use crate::marker;

mod combat;
mod run;
mod self_test;
#[cfg(test)]
mod tests;

pub use combat::{combat_ended, combat_started};
pub use run::{
    run_ended, run_history_clear, run_history_select, run_started, run_suspended, set_run_meta,
};
pub use self_test::self_test;

pub fn init(data_dir: &Path) {
    if STATE.with(|cell| cell.borrow().store_paths.is_some()) {
        return;
    }
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.store_paths = Some(StorePaths::new(data_dir));
        state.run_profile = -1;
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

pub fn source_capture(
    combat_seq: u64,
    capture_kind: i32,
    instance: u64,
    source_id: &str,
    source_kind: i32,
    source_slot: i32,
    generation_state: i32,
) -> u64 {
    STATE.with(|cell| {
        cell.borrow_mut().source_capture(
            combat_seq,
            capture_kind,
            instance,
            source_id,
            source_kind,
            source_slot,
            generation_state,
        )
    })
}

pub fn source_count(transfer: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().source_count(transfer))
}

pub fn source_destination(transfer: u64, index: i32) -> u64 {
    STATE.with(|cell| cell.borrow_mut().source_destination(transfer, index))
}

pub fn source_weight(transfer: u64, index: i32) -> u64 {
    STATE.with(|cell| cell.borrow_mut().source_weight(transfer, index))
}

pub fn source_transfer_begin(combat_seq: u64) -> u64 {
    STATE.with(|cell| cell.borrow_mut().source_transfer_begin(combat_seq))
}

pub fn source_transfer_add(transfer: u64, destination: u64, weight: u64) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .source_transfer_add(transfer, destination, weight)
    })
}

pub fn source_transfer_seal(transfer: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().source_transfer_seal(transfer))
}

pub fn source_transfer_release(transfer: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().source_transfer_release(transfer))
}

#[allow(clippy::too_many_arguments)]
pub fn power_attached(
    combat_seq: u64,
    power_instance: u64,
    power_id: &str,
    owner_creature: u64,
    owner_kind: i32,
    owner_slot: i32,
    amount: i32,
    source_transfer: u64,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().power_attached(
            combat_seq,
            power_instance,
            power_id,
            owner_creature,
            owner_kind,
            owner_slot,
            amount,
            source_transfer,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn power_amount_changed(
    combat_seq: u64,
    power_instance: u64,
    power_id: &str,
    owner_creature: u64,
    owner_kind: i32,
    owner_slot: i32,
    old_amount: i32,
    new_amount: i32,
    source_transfer: u64,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().power_amount_changed(
            combat_seq,
            power_instance,
            power_id,
            owner_creature,
            owner_kind,
            owner_slot,
            old_amount,
            new_amount,
            source_transfer,
        )
    })
}

pub fn power_removed(combat_seq: u64, power_instance: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().power_removed(combat_seq, power_instance))
}

pub fn power_provenance_invalidate(combat_seq: u64, power_instance: u64) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .power_provenance_invalidate(combat_seq, power_instance)
    })
}

pub fn card_generated(
    combat_seq: u64,
    card_instance: u64,
    source_transfer: u64,
    producer_role: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .card_generated(combat_seq, card_instance, source_transfer, producer_role)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn card_play_started(
    combat_seq: u64,
    execution_id: u64,
    card_instance: u64,
    card_id: &str,
    player_slot: i32,
    play_index: i32,
    play_count: i32,
    generation_state: i32,
    source_transfer: u64,
) -> u64 {
    STATE.with(|cell| {
        cell.borrow_mut().card_play_started(
            combat_seq,
            execution_id,
            card_instance,
            card_id,
            player_slot,
            play_index,
            play_count,
            generation_state,
            source_transfer,
        )
    })
}

pub fn card_play_finished(play: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().card_play_finished(play))
}

pub fn card_execution_ended(combat_seq: u64, execution_id: u64) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .card_execution_ended(combat_seq, execution_id)
    })
}

pub fn orb_channeled(combat_seq: u64, orb_instance: u64, source_transfer: u64) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .orb_channeled(combat_seq, orb_instance, source_transfer)
    })
}

pub fn orb_context_begin(combat_seq: u64, orb_instance: u64, play: u64, owner_slot: i32) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .orb_context_begin(combat_seq, orb_instance, play, owner_slot)
    })
}

pub fn damage_calculation_begin(
    combat_seq: u64,
    source_transfer: u64,
    producer_role: i32,
    segment: i32,
    original_target: u64,
) -> u64 {
    STATE.with(|cell| {
        cell.borrow_mut().damage_calculation_begin(
            combat_seq,
            source_transfer,
            producer_role,
            segment,
            original_target,
        )
    })
}

pub fn damage_modifier_contribution(calculation: u64, source_transfer: u64, amount: i32) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .damage_modifier_contribution(calculation, source_transfer, amount)
    })
}

pub fn damage_calculation_enemy_hit(
    calculation: u64,
    dealer_creature: u64,
    base_damage: i32,
    dealer_strength: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().damage_calculation_enemy_hit(
            calculation,
            dealer_creature,
            base_damage,
            dealer_strength,
        )
    })
}

pub fn damage_calculation_weak_source(calculation: u64, source_transfer: u64) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .damage_calculation_weak_source(calculation, source_transfer)
    })
}

pub fn damage_result_append(
    calculation: u64,
    total: i32,
    unblocked: i32,
    blocked: i32,
    result_kind: i32,
    receiver_slot: i32,
    weak_prevented: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().damage_result_append(
            calculation,
            total,
            unblocked,
            blocked,
            result_kind,
            receiver_slot,
            weak_prevented,
        )
    })
}

pub fn damage_calculation_commit(calculation: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().damage_calculation_commit(calculation))
}

pub fn damage_calculation_abort(calculation: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().damage_calculation_abort(calculation))
}

pub fn damage_unattributed(
    combat_seq: u64,
    total: i32,
    unblocked: i32,
    blocked: i32,
    result_kind: i32,
    receiver_slot: i32,
    weak_prevented: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().damage_unattributed(
            combat_seq,
            total,
            unblocked,
            blocked,
            result_kind,
            receiver_slot,
            weak_prevented,
        )
    })
}

pub fn buff_mitigation(combat_seq: u64, source_transfer: u64, prevented: i32) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .buff_mitigation(combat_seq, source_transfer, prevented)
    })
}

pub fn block_modifier_contribution(
    combat_seq: u64,
    source_transfer: u64,
    amount: i32,
    receiver_slot: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().block_modifier_contribution(
            combat_seq,
            source_transfer,
            amount,
            receiver_slot,
        )
    })
}

pub fn block_gained(combat_seq: u64, amount: i32, source_transfer: u64, receiver_slot: i32) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .block_gained(combat_seq, amount, source_transfer, receiver_slot)
    })
}

pub fn forge(combat_seq: u64, source_transfer: u64, amount: i32) -> i32 {
    STATE.with(|cell| cell.borrow_mut().forge(combat_seq, source_transfer, amount))
}

pub fn osty_summoned(
    combat_seq: u64,
    source_transfer: u64,
    hp_amount: i32,
    owner_slot: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .osty_summoned(combat_seq, source_transfer, hp_amount, owner_slot)
    })
}

pub fn osty_killed(combat_seq: u64, owner_slot: i32, play: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().osty_killed(combat_seq, owner_slot, play))
}

pub fn doom_batch_begin(combat_seq: u64) -> u64 {
    STATE.with(|cell| cell.borrow_mut().doom_batch_begin(combat_seq))
}

pub fn doom_target_capture(
    batch: u64,
    creature_instance: u64,
    doom_power_instance: u64,
    current_hp: i32,
) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut().doom_target_capture(
            batch,
            creature_instance,
            doom_power_instance,
            current_hp,
        )
    })
}

pub fn doom_kills_completed(batch: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().doom_kills_completed(batch))
}

pub fn doom_batch_abort(batch: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().doom_batch_abort(batch))
}

pub fn turn_started(combat_seq: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().turn_started(combat_seq))
}

pub fn block_pool_clear(combat_seq: u64, player_slot: i32) -> i32 {
    STATE.with(|cell| cell.borrow_mut().block_pool_clear(combat_seq, player_slot))
}

pub fn player_died(combat_seq: u64, player_slot: i32) -> i32 {
    STATE.with(|cell| cell.borrow_mut().player_died(combat_seq, player_slot))
}

pub fn potion_used(combat_seq: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().potion_used(combat_seq))
}
