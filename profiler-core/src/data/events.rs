//! Test adapter for seeded event walks. Production uses explicitly owned engines.
use super::state::{STATE, State};

pub fn test_reset() {
    STATE.with(|cell| *cell.borrow_mut() = State::default());
}
pub fn combat_started(encounter: &str, kind: &str) -> u64 {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let seq = state.current.as_ref().map_or(1, |combat| combat.seq + 1);
        state.combat_started(seq, encounter, kind, 1_786_579_200, 0)
    })
}
pub fn combat_ended(epoch: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().combat_ended(epoch))
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

pub fn block_gained(combat_seq: u64, amount: i32, source_transfer: u64, receiver_slot: i32) -> i32 {
    block_gained_with_modifiers(
        combat_seq,
        amount,
        source_transfer,
        receiver_slot,
        &[],
        false,
    )
}

pub fn block_gained_with_modifiers(
    combat_seq: u64,
    amount: i32,
    source_transfer: u64,
    receiver_slot: i32,
    modifiers: &[crate::abi::BlockModifier],
    incomplete: bool,
) -> i32 {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let entries: Vec<_> = modifiers
            .iter()
            .map(|entry| (entry.source, entry.credit))
            .collect();
        let modifiers = state.parse_block_modifiers(combat_seq, &entries, incomplete);
        state.block_gained(
            combat_seq,
            amount,
            source_transfer,
            receiver_slot,
            modifiers,
        )
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

pub fn block_pool_loss(combat_seq: u64, player_slot: i32, amount: i32) -> i32 {
    STATE.with(|cell| {
        cell.borrow_mut()
            .block_pool_loss(combat_seq, player_slot, amount)
    })
}

pub fn player_died(combat_seq: u64, player_slot: i32) -> i32 {
    STATE.with(|cell| cell.borrow_mut().player_died(combat_seq, player_slot))
}

pub fn potion_used(combat_seq: u64) -> i32 {
    STATE.with(|cell| cell.borrow_mut().potion_used(combat_seq))
}
