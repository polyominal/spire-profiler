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
        assert_ne!(source, 0);
    }
    for slot in 0..=TEAM_SLOT {
        let source = card(&mut state, "CAPACITY_LOST", i32::from(slot));
        assert_eq!(
            state.source_destination(source, 0),
            (7_u64 << 32) | (u64::from(slot) << 3) | 1
        );
        hit(&mut state, source, 3);
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
            .all(|row| row.damage_dealt == 3 && row.id.as_ref() == "UNATTRIBUTED")
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
    assert_eq!(state.damage_result_append(calculation, 2, 2, 0, 0, 4, 0), 1);
    assert_eq!(
        state.damage_modifier_contribution(calculation, u64::MAX, 3),
        0
    );
    assert_eq!(state.damage_modifier_contribution(calculation, a, 1), 0);
    assert_eq!(state.damage_calculation_weak_source(calculation, a), 0);
    assert_eq!(
        state.damage_calculation_enemy_hit(calculation, 900, 4, 0),
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
            .find(|row| row.id.as_ref() == "A")
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
                id: "NEGATIVE".into(),
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
        assert_ne!(source, 0);
    }
    let source = card(&mut state, "A", i32::MAX);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert!(
        combat
            .cards
            .iter()
            .all(|row: &CardStat| row.player <= TEAM_SLOT)
    );
    assert_eq!(state.source_count(source | 7), -1);
}

#[test]
fn producer_segments_preserve_wire_policy_and_credited_rows() {
    for (role, direct, attributed) in [
        (-1, Some(1), Some(1)),
        (0, Some(1), Some(1)),
        (1, Some(0), None),
        (2, None, Some(1)),
        (3, Some(0), None),
        (4, Some(0), None),
        (5, Some(0), Some(1)),
        (i32::MAX, Some(0), Some(1)),
    ] {
        for (segment, expected) in [
            (i32::MIN, direct),
            (-1, direct),
            (0, direct),
            (1, attributed),
            (2, None),
            (i32::MAX, None),
        ] {
            let mut state = fixture();
            let producer = card(&mut state, "PRODUCER", 0);
            let modifier = card(&mut state, "MODIFIER", 1);
            let calculation = state.damage_calculation_begin(7, producer, role, segment, 999);
            let Some(expected) = expected else {
                assert_eq!(calculation, 0, "role {role}, segment {segment}");
                assert!(state.provenance.calculations.is_empty());
                continue;
            };
            assert_ne!(calculation, 0, "role {role}, segment {segment}");
            assert_eq!(
                state.damage_modifier_contribution(calculation, modifier, 2),
                1
            );
            assert_eq!(state.damage_result_append(calculation, 5, 3, 2, 0, 4, 0), 1);
            assert_eq!(state.damage_calculation_commit(calculation), 1);
            let rows = &state.current.as_ref().expect("fixture combat exists").cards;
            let producer_id = if role <= 0 {
                "UNATTRIBUTED"
            } else {
                "PRODUCER"
            };
            let producer_row = rows
                .iter()
                .find(|row| row.id.as_ref() == producer_id)
                .expect("accepted hit credits the normalized producer");
            assert_eq!(
                (
                    producer_row.dmg_direct,
                    producer_row.dmg_attributed,
                    producer_row.dmg_modifier
                ),
                if expected == 0 { (3, 0, 0) } else { (0, 3, 0) },
                "role {role}, segment {segment}"
            );
            let modifier_row = rows
                .iter()
                .find(|row| row.id.as_ref() == "MODIFIER")
                .expect("modifier source has its own row");
            assert_eq!(
                (
                    modifier_row.dmg_direct,
                    modifier_row.dmg_attributed,
                    modifier_row.dmg_modifier
                ),
                (0, 0, 2)
            );
            assert_eq!(rows.iter().map(|row| row.damage_dealt).sum::<i64>(), 5);
            assert_eq!(rows.iter().map(|row| row.damage_blocked).sum::<i64>(), 2);
        }
    }
}

#[test]
fn interned_sources_preserve_lifetime_mixtures_and_expire_with_the_combat() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let b = card(&mut state, "B", 1);
    assert_eq!(card(&mut state, "A", 0), a);
    let mixed = state.source_accumulate(7, a, 2, b, 5);
    assert_eq!(state.source_weight(mixed, 0), 2);
    assert_eq!(state.source_weight(mixed, 1), 3);
    assert_eq!(state.source_accumulate(7, a, 4, b, 10), mixed);
    assert_eq!(state.source_accumulate(7, a, -2, b, 3), b);
    assert_eq!(state.source_accumulate(7, a, 0, b, 0), b);
    assert_eq!(state.source_accumulate(7, a, 2, b, 2), a);
    let prior = state.sources.entries.len();
    assert_eq!(state.source_accumulate(7, a, 2, b, 1), 0);
    assert_eq!(state.source_accumulate(7, a, i32::MIN, b, i32::MAX), 0);
    assert_eq!(state.sources.entries.len(), prior);
    for index in 1..=9000 {
        assert_ne!(state.source_accumulate(7, a, index, b, index + 1), 0);
    }
    assert!(state.sources.entries.len() > 9000);
    assert_ne!(state.source_accumulate(7, a, i32::MAX - 1, b, i32::MAX), 0);
    assert_eq!(state.source_accumulate(7, u64::MAX, 0, b, 3), b);
    assert_eq!(state.source_accumulate(7, a, 3, u64::MAX, 3), a);
    hit(&mut state, mixed, 5);
    assert_eq!(
        state.current.as_ref().expect("fixture exists").cards[0].damage_dealt,
        2
    );
    assert_eq!(state.combat_started(8, "NEXT", "normal", 1000, 1), 8);
    assert_eq!(state.source_count(mixed), -1);
    assert_eq!(state.source_accumulate(8, 0, 0, mixed, 3), mixed);
    assert_eq!(state.source_accumulate(8, mixed, 3, 0, 3), mixed);
    assert_eq!(state.source_accumulate(8, mixed, 3, 0, 4), 0);
    assert_eq!(state.source_accumulate(8, 0, 0, 0, 0), 0);
    assert_eq!(state.combat_started(7, "OLD", "normal", 1000, 1), 0);
}

#[test]
fn journal_unwind_restores_touched_rows_pools_counters_and_appended_entries() {
    let mut state = fixture();
    let source = card(&mut state, "DEFEND", 0);
    assert_eq!(state.block_gained(7, 10, source, 0), 1);
    let before = state.snapshot();
    let rows = state.current.as_ref().expect("fixture exists").cards.len();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut stage = LedgerStage::new(&mut state).expect("fixture is active");
        stage.consume_block(0, 4).expect("pool covers four block");
        stage
            .credit(Destination::Unknown(TEAM_SLOT), CreditField::Forge, 9)
            .expect("reserved Unknown row exists");
        stage.combat.damage_received = 123;
        stage.pool(3).expect("player slot is valid").osty.clear();
        panic!("fault after multiple provisional writes");
    }));
    assert!(result.is_err());
    assert_eq!(state.snapshot(), before);
    assert_eq!(
        state.current.as_ref().expect("fixture exists").cards.len(),
        rows
    );
    assert_eq!(state.provenance.pools.len(), 1);
    assert_eq!(state.provenance.pools[0].blocks[0].remaining, 10);
}

#[test]
fn released_handles_expire_without_releasing_native_consumers() {
    let mut state = fixture();
    let a = card(&mut state, "A", 0);
    let b = card(&mut state, "B", 1);
    let mixed = state.source_accumulate(7, a, 2, b, 5);
    assert_eq!(state.source_accumulate(7, a, 2, b, 5), mixed);
    assert_eq!(
        state.power_attached(7, 70, "STRENGTH_POWER", 80, 0, 0, 5, mixed),
        1
    );
    let calculation = state.damage_calculation_begin(7, mixed, 1, 0, 999);
    for handle in [a, b, mixed] {
        assert_eq!(state.source_release(handle), 1);
    }
    assert!(state.sources.entries.is_empty() && state.sources.index.is_empty());
    assert_eq!(state.damage_result_append(calculation, 5, 5, 0, 0, 4, 0), 1);
    assert_eq!(state.damage_calculation_commit(calculation), 1);
    let retained = state.source_capture(7, 2, 70, "", 2, 0, 0);
    assert!(retained > mixed, "released serials are never reused");
    assert_eq!(
        (
            state.source_weight(retained, 0),
            state.source_weight(retained, 1)
        ),
        (2, 3)
    );
    hit(&mut state, retained, 5);
    let combat = state.current.as_ref().expect("fixture is active");
    assert_eq!(
        (combat.cards[0].damage_dealt, combat.cards[1].damage_dealt),
        (4, 6)
    );
    assert_eq!(state.source_release(mixed), 0);
    assert_eq!(state.source_count(mixed), -1);
    assert_eq!(state.source_count(retained), 2);
    assert_eq!(state.combat_started(8, "NEXT", "normal", 1000, 1), 8);
    assert_eq!(state.source_release(retained), 0);
    assert!(state.sources.entries.is_empty() && state.sources.index.is_empty());
}

#[test]
fn source_collection_schedule_never_changes_credited_results() {
    let mut retained = fixture();
    let mut collected = fixture();
    let roots =
        [&mut retained, &mut collected].map(|state| (card(state, "A", 0), card(state, "B", 1)));
    for index in 1..=1_000 {
        for (state, (a, b)) in [&mut retained, &mut collected].into_iter().zip(roots) {
            let source = state.source_accumulate(7, a, index, b, index + 1);
            hit(state, source, index % 17 + 1);
        }
        let source = collected.source_accumulate(7, roots[1].0, index, roots[1].1, index + 1);
        assert_eq!(collected.source_release(source), 1);
        assert_eq!(collected.sources.entries.len(), 2);
        assert_eq!(collected.sources.index.len(), 2);
        assert_eq!(collected.snapshot(), retained.snapshot());
    }
    collected.sources.serial = PAYLOAD_MAX - 1;
    let last = collected.source_accumulate(7, roots[1].0, 1, roots[1].1, 2);
    assert_ne!(last, 0);
    assert_eq!(collected.source_release(last), 1);
    assert_eq!(
        collected.source_accumulate(7, roots[1].0, 1, roots[1].1, 2),
        0
    );
    assert_eq!(collected.source_count(last), -1);
}
