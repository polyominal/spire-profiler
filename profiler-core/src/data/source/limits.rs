use super::*;
use crate::data::state::{CardStat, SourceKind};

fn fixture() -> State {
    State {
        current: Some(Combat {
            seq: 7,
            ..Combat::default()
        }),
        ..State::default()
    }
}

fn card(state: &mut State, id: &str, slot: i32) -> u64 {
    let instance = state
        .current
        .as_ref()
        .expect("fixture combat exists")
        .cards
        .len() as u64
        + 1;
    state.source_capture(7, 1, instance, id, 0, slot, 0)
}

fn hit(state: &mut State, source: u64, amount: i32) {
    let calculation = state.damage_calculation_begin(7, source, 1, 0, 999);
    assert_ne!(calculation, 0);
    assert_eq!(
        state.damage_result_append(calculation, amount, amount, 0, 0, 4, 0),
        1
    );
    assert_eq!(state.damage_calculation_commit(calculation), 1);
}

#[test]
fn ordinary_rows_leave_one_unknown_destination_per_creditor_slot() {
    let mut state = fixture();
    for index in 0..caps::COMBAT_CARDS - caps::UNKNOWN_ROWS {
        let source = card(&mut state, &format!("CARD_{index}"), 0);
        assert_eq!(state.source_transfer_release(source), 1);
    }
    for slot in 0..=TEAM_SLOT {
        let source = card(&mut state, "CAPACITY_LOST", i32::from(slot));
        assert_eq!(
            state.source_destination(source, 0),
            (7_u64 << 32) | (u64::from(slot) << 3) | 1
        );
        hit(&mut state, source, 3);
        state.source_transfer_release(source);
    }
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(combat.cards.len(), caps::COMBAT_CARDS);
    assert_eq!(
        combat
            .cards
            .iter()
            .filter(|row| row.kind == SourceKind::Unknown)
            .count(),
        5
    );
    assert!(
        combat
            .cards
            .iter()
            .filter(|row| row.kind == SourceKind::Unknown)
            .all(|row| row.damage_dealt == 3 && row.id == "UNATTRIBUTED")
    );
}

#[test]
fn unknown_row_tokens_and_unknown_slot_tokens_normalize_to_one_supplier() {
    let mut state = fixture();
    let named = card(&mut state, "UNATTRIBUTED", 4);
    hit(&mut state, named, 2);
    assert_eq!(state.damage_unattributed(7, 3, 3, 0, 0, 4, 0), 1);
    let combat = state.current.as_ref().expect("fixture combat exists");
    let row = combat
        .cards
        .iter()
        .position(|row| row.kind == SourceKind::Unknown)
        .expect("fallback materialized its row");
    let transfer = state.source_transfer_begin(7);
    assert_eq!(
        state.source_transfer_add(transfer, (7_u64 << 32) | ((row as u64) << 3), 1),
        1
    );
    assert_eq!(
        state.source_transfer_add(transfer, (7_u64 << 32) | (4 << 3) | 1, 1),
        1
    );
    assert_eq!(state.source_transfer_seal(transfer), 1);
    assert_eq!(state.source_count(transfer), 1);
    assert_eq!(state.card_generated(7, 500, transfer, 2), 1);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(combat.generation_triggers, 1);
    assert_eq!(
        combat
            .cards
            .iter()
            .filter(|row| row.id == "UNATTRIBUTED")
            .count(),
        2
    );
}

#[test]
fn power_layer_overflow_collapses_the_entire_balance_before_new_grants() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let b = card(&mut state, "B", 1);
    assert_eq!(
        state.power_attached(7, 70, "STRENGTH_POWER", 80, 0, 0, 1, a),
        1
    );
    for old in 1..=caps::POWER_GRANTS_PER_INSTANCE as i32 {
        let source = if old % 2 == 1 { b } else { a };
        assert_eq!(
            state.power_amount_changed(7, 70, "STRENGTH_POWER", 80, 0, 0, old, old + 1, source),
            1
        );
    }
    let source = state.source_capture(7, 2, 70, "", 2, 0, 0);
    assert_eq!(
        state.source_destination(source, 0),
        (7_u64 << 32) | (4 << 3) | 1
    );
    assert_eq!(state.source_count(source), 1);
    state.source_transfer_release(source);
    assert_eq!(
        state.power_amount_changed(7, 70, "STRENGTH_POWER", 80, 0, 0, 65, 66, b),
        1
    );
    let source = state.source_capture(7, 2, 70, "", 2, 0, 0);
    assert_eq!(state.source_count(source), 2);
    assert_eq!(
        (
            state.source_weight(source, 0),
            state.source_weight(source, 1)
        ),
        (65, 1)
    );
}

#[test]
fn exhausted_instance_table_does_not_retarget_a_missing_power() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    for instance in 1..=caps::POWER_INSTANCES as u64 {
        assert_eq!(
            state.power_attached(7, instance, "POWER", instance + 1000, 1, 4, 1, a),
            1
        );
    }
    assert_eq!(state.power_attached(7, 999, "POWER", 1999, 1, 4, 1, a), 0);
    let missing = state.source_capture(7, 2, 999, "", 2, 0, 0);
    assert_eq!(
        state.source_destination(missing, 0),
        (7_u64 << 32) | (4 << 3) | 1
    );
    let known = state.source_capture(7, 2, 1, "", 2, 0, 0);
    assert_eq!(
        state.source_destination(known, 0),
        state.source_destination(a, 0)
    );
}

#[test]
fn failed_capture_invalidates_group_and_classified_fallback_books_once() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let calculation = state.damage_calculation_begin(7, a, 1, 0, 999);
    assert_eq!(
        state.damage_modifier_contribution(calculation, u64::MAX, 3),
        0
    );
    assert_eq!(state.damage_result_append(calculation, 7, 3, 4, 0, 4, 0), 0);
    assert_eq!(state.damage_calculation_commit(calculation), 0);
    assert_eq!(state.damage_unattributed(7, 7, 3, 4, 0, 4, 0), 1);
    assert_eq!(state.damage_calculation_commit(calculation), 0);
    let combat = state.current.as_ref().expect("fixture combat exists");
    let unknown = combat
        .cards
        .iter()
        .find(|row| row.kind == SourceKind::Unknown)
        .expect("fallback has a row");
    assert_eq!(
        (
            unknown.damage_dealt,
            unknown.dmg_attributed,
            unknown.damage_blocked
        ),
        (7, 7, 4)
    );
    assert_eq!(
        combat
            .cards
            .iter()
            .find(|row| row.id == "A")
            .expect("source row exists")
            .damage_dealt,
        0
    );
}

#[test]
fn late_arithmetic_failure_publishes_no_partial_row_or_pool_updates() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let b = card(&mut state, "B", 1);
    assert_eq!(state.block_gained(7, 10, a, 0), 1);
    let row = &mut state.current.as_mut().expect("fixture combat exists").cards[1];
    row.damage_dealt = i64::MAX;
    row.dmg_direct = i64::MAX;
    let before = state
        .current
        .as_ref()
        .expect("fixture combat exists")
        .cards
        .clone();
    let calculation = state.damage_calculation_begin(7, b, 1, 0, 999);
    assert_eq!(
        state.damage_result_append(calculation, 10, 0, 10, 1, 0, 0),
        1
    );
    assert_eq!(state.damage_result_append(calculation, 1, 1, 0, 0, 4, 0), 1);
    assert_eq!(state.damage_calculation_commit(calculation), 0);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(combat.cards, before);
    assert_eq!(combat.damage_received, 0);
    assert_eq!(state.provenance.pools[0].blocks[0].remaining, 10);
}

#[test]
fn defense_totals_reject_overflow_without_materializing_unknown_rows() {
    for destination in 0..3 {
        for negative in [0, -2] {
            let mut state = fixture();
            let a = card(&mut state, "A", 0);
            let b = card(&mut state, "B", 1);
            let source = [a, b, 0][destination];
            let combat = state.current.as_mut().expect("fixture combat exists");
            combat.cards[0].mitigate_debuff = i64::MAX - 1;
            combat.cards.push(CardStat {
                id: "NEGATIVE".to_owned(),
                player: 2,
                block_effective: negative,
                ..CardStat::default()
            });
            let before = combat.cards.clone();
            assert_eq!(state.buff_mitigation(7, source, 2), 0);
            assert_eq!(
                state.current.as_ref().expect("fixture combat exists").cards,
                before
            );
            assert_eq!(state.buff_mitigation(7, source, 1), 1);
            let rows = &state.current.as_ref().expect("fixture combat exists").cards;
            let positive_defense: i128 = rows
                .iter()
                .map(|row| {
                    (i128::from(row.block_effective)
                        + i128::from(row.blk_modifier)
                        + i128::from(row.mitigate_debuff)
                        + i128::from(row.mitigate_buff)
                        + i128::from(row.mitigate_str))
                    .max(0)
                })
                .sum();
            assert_eq!(positive_defense, i128::from(i64::MAX));
        }
    }
}

#[test]
fn defense_overflow_rolls_back_consumed_block_and_fallback() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let b = card(&mut state, "B", 1);
    assert_eq!(state.block_gained(7, 10, b, 0), 1);
    let combat = state.current.as_mut().expect("fixture combat exists");
    combat.cards[0].mitigate_buff = i64::MAX - 1;
    let before = combat.cards.clone();
    let calculation = state.damage_calculation_begin(7, a, 1, 0, 999);
    assert_eq!(state.damage_calculation_weak_source(calculation, a), 1);
    assert_eq!(state.damage_result_append(calculation, 1, 0, 1, 1, 0, 1), 1);
    assert_eq!(state.damage_calculation_commit(calculation), 0);
    assert_eq!(state.damage_unattributed(7, 1, 0, 1, 1, 0, 1), 0);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(combat.cards, before);
    assert_eq!(combat.damage_received, 0);
    assert_eq!(state.provenance.pools[0].blocks[0].remaining, 10);
    assert_eq!(state.damage_unattributed(7, 1, 0, 1, 1, 0, 0), 1);
    assert_eq!(state.provenance.pools[0].blocks[0].remaining, 9);
}

#[test]
fn empty_live_poison_does_not_reuse_its_retained_attachment_source() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    assert_eq!(
        state.power_attached(7, 70, "POISON_POWER", 80, 1, 4, 1, a),
        1
    );
    let frozen = state.source_capture(7, 2, 70, "", 2, 4, 0);
    assert_eq!(
        state.power_amount_changed(7, 70, "POISON_POWER", 80, 1, 4, 1, 0, 0),
        1
    );
    let live = state.source_capture(7, 2, 70, "", 2, 4, 0);
    assert_eq!(
        state.source_destination(live, 0),
        (7_u64 << 32) | (4 << 3) | 1
    );
    assert_eq!(
        state.source_destination(frozen, 0),
        state.source_destination(a, 0)
    );
}

#[test]
fn malformed_scalars_clamp_without_turning_reserved_tokens_into_objects() {
    let mut state = fixture();
    for value in [i32::MIN, -1, 0, 1, 4, 5, i32::MAX] {
        let source = state.source_capture(7, value, 0, "", value, value, value);
        assert_eq!(state.source_count(source), 1);
        assert_eq!(state.source_transfer_release(source), 1);
    }
    let source = card(&mut state, "A", i32::MAX);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert!(
        combat
            .cards
            .iter()
            .all(|row: &CardStat| row.player <= TEAM_SLOT)
    );
    assert_eq!(state.source_transfer_add(source, u64::MAX, 1), 0);
    assert_eq!(state.source_count(source), -1);
}
