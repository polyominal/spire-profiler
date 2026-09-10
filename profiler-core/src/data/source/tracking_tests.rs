use super::*;

fn fixture() -> State {
    let mut state = State::default();
    state
        .reserve_lifecycle()
        .expect("fixture startup reserves the bounded owners");
    state.current.set(&Combat {
        seq: 7,
        ..Combat::default()
    });
    state
}

fn card(state: &mut State, id: &str) -> u64 {
    state.source_capture(7, 1, 5000, id, 0, 0, 0)
}

fn weights(state: &mut State, transfer: u64) -> Vec<u64> {
    (0..state.source_count(transfer))
        .map(|index| state.source_weight(transfer, index))
        .collect()
}

#[test]
fn global_grant_reserve_survives_all_power_admissions_and_collapses_before_overflow() {
    let mut state = fixture();
    let a = card(&mut state, "A");
    let b = card(&mut state, "B");
    for instance in 1..=caps::POWER_INSTANCES as u64 {
        assert_eq!(
            state.power_attached(7, instance, "POWER", instance + 1000, 1, 4, 1, a),
            1
        );
    }
    for instance in 1..=5 {
        let layers = if instance == 5 { 5 } else { 64 };
        for old in 1..layers {
            let source = if old % 2 == 1 { b } else { a };
            assert_eq!(
                state.power_amount_changed(
                    7,
                    instance,
                    "POWER",
                    instance + 1000,
                    1,
                    4,
                    old,
                    old + 1,
                    source
                ),
                1
            );
        }
    }
    assert_eq!(state.provenance.grants.len(), caps::POWER_GRANTS_TOTAL);
    assert_eq!(
        state.power_amount_changed(7, 5, "POWER", 1005, 1, 4, 5, 6, b),
        1
    );
    assert_eq!(state.provenance.grants.len(), caps::POWER_GRANTS_TOTAL - 4);
    let source = state.source_capture(7, 2, 5, "", 2, 4, 0);
    assert_eq!(state.source_count(source), 1);
    assert_eq!(
        state.source_destination(source, 0),
        Destination::Unknown(TEAM_SLOT)
            .token(CombatEpoch::from_wire(7).expect("fixture epoch is valid"))
    );
    state.source_transfer_release(source);
    assert_eq!(
        state.power_amount_changed(7, 5, "POWER", 1005, 1, 4, 6, 7, a),
        1
    );
    let source = state.source_capture(7, 2, 5, "", 2, 4, 0);
    assert_eq!(weights(&mut state, source), vec![6, 1]);
    assert_eq!(state.provenance.powers.len(), caps::POWER_INSTANCES);
}

#[test]
fn fifo_and_captured_identity_survive_power_compaction_and_grant_reuse() {
    let mut state = fixture();
    let a = card(&mut state, "A");
    let b = card(&mut state, "B");
    assert_eq!(state.power_attached(7, 1, "POWER", 101, 1, 4, 2, a), 1);
    assert_eq!(state.power_attached(7, 2, "POWER", 102, 1, 4, 3, b), 1);
    assert_eq!(state.power_attached(7, 3, "POWER", 103, 1, 4, 2, a), 1);
    assert_eq!(
        state.power_amount_changed(7, 3, "POWER", 103, 1, 4, 2, 6, b),
        1
    );
    let captured = state.source_capture(7, 2, 3, "", 2, 4, 0);
    let original = weights(&mut state, captured);
    assert_eq!(original, vec![1, 2]);
    assert_eq!(state.power_removed(7, 1), 1);
    assert_eq!(
        state.power_amount_changed(7, 3, "POWER", 103, 1, 4, 6, 3, a),
        1
    );
    let drained = state.source_capture(7, 2, 3, "", 2, 4, 0);
    assert_eq!(state.source_count(drained), 1);
    assert_eq!(
        state.source_destination(drained, 0),
        state.source_destination(b, 0)
    );
    assert_eq!(
        state.power_amount_changed(7, 3, "POWER", 103, 1, 4, 3, 0, a),
        1
    );
    assert_eq!(state.power_attached(7, 4, "POWER", 104, 1, 4, 8, a), 1);
    assert_eq!(weights(&mut state, captured), original);
    let last_source = state.source_capture(7, 2, 3, "", 2, 4, 0);
    assert_eq!(
        state.source_destination(last_source, 0),
        state.source_destination(b, 0)
    );
    let second = state.source_capture(7, 2, 2, "", 2, 4, 0);
    assert_eq!(
        state.source_destination(second, 0),
        state.source_destination(b, 0)
    );
}

#[test]
fn strength_recovery_consumes_latest_creature_reductions_across_power_instances() {
    let mut state = fixture();
    let a = card(&mut state, "A");
    let b = card(&mut state, "B");
    assert_eq!(
        state.power_attached(7, 1, "STRENGTH_POWER", 100, 1, 4, 0, a),
        1
    );
    assert_eq!(
        state.power_attached(7, 2, "STRENGTH_POWER", 100, 1, 4, 0, b),
        1
    );
    assert_eq!(
        state.power_amount_changed(7, 1, "STRENGTH_POWER", 100, 1, 4, 0, -4, a),
        1
    );
    assert_eq!(
        state.power_amount_changed(7, 2, "STRENGTH_POWER", 100, 1, 4, 0, -6, b),
        1
    );
    assert_eq!(
        state.power_amount_changed(7, 1, "STRENGTH_POWER", 100, 1, 4, -4, 1, a),
        1
    );
    assert_eq!(
        state
            .provenance
            .reductions
            .iter()
            .map(|entry| (entry.power_instance, entry.amount))
            .collect::<Vec<_>>(),
        vec![(1, 4), (2, 1)]
    );
    assert_eq!(
        state.power_amount_changed(7, 2, "STRENGTH_POWER", 100, 1, 4, -6, -4, b),
        1
    );
    assert_eq!(
        state
            .provenance
            .reductions
            .iter()
            .map(|entry| (entry.power_instance, entry.amount))
            .collect::<Vec<_>>(),
        vec![(1, 3)]
    );
}

#[test]
fn reduction_capacity_failure_does_not_publish_a_partial_power_balance() {
    let mut state = fixture();
    let source = card(&mut state, "REDUCER");
    for instance in 1..=caps::STR_REDUCTIONS as u64 {
        assert_eq!(
            state.power_attached(
                7,
                instance,
                "STRENGTH_POWER",
                instance + 1000,
                1,
                4,
                0,
                source
            ),
            1
        );
        assert_eq!(
            state.power_amount_changed(
                7,
                instance,
                "STRENGTH_POWER",
                instance + 1000,
                1,
                4,
                0,
                -1,
                source
            ),
            1
        );
    }
    assert_eq!(
        state.power_attached(7, 100, "STRENGTH_POWER", 1100, 1, 4, 10, source),
        1
    );
    assert_eq!(
        state.power_amount_changed(7, 100, "STRENGTH_POWER", 1100, 1, 4, 10, 9, source),
        0
    );
    let index = state
        .provenance
        .powers
        .iter()
        .position(|power| power.instance == 100)
        .expect("failed observation retains its attachment");
    assert_eq!(state.provenance.powers[index].observed, 10);
    assert!(!state.provenance.powers[index].trusted);
    assert_eq!(
        state
            .provenance
            .grants
            .iter()
            .filter(|entry| entry.power == index)
            .map(|entry| entry.grant.remaining())
            .sum::<u32>(),
        10
    );
    assert_eq!(state.provenance.reductions.len(), caps::STR_REDUCTIONS);
}

#[test]
fn all_five_play_slots_reject_before_counters_and_reuse_without_token_aliasing() {
    let mut state = fixture();
    let source = card(&mut state, "CARD");
    let mut plays = Vec::new();
    for owner in 0..caps::MAX_PLAYER_SLOTS as i32 {
        for count in 0..caps::ACTIVE_PLAYS_PER_SLOT {
            let execution = 1 + (owner as usize * caps::ACTIVE_PLAYS_PER_SLOT + count) as u64;
            let play = state.card_play_started(7, execution, 5000, "CARD", owner, 0, 1, 0, source);
            assert_ne!(play, 0);
            plays.push(play);
        }
    }
    assert_eq!(state.provenance.plays.len(), caps::ACTIVE_PLAYS);
    let before = state
        .current
        .as_ref()
        .expect("fixture combat remains active")
        .clone();
    for owner in 0..caps::MAX_PLAYER_SLOTS as i32 {
        assert_eq!(
            state.card_play_started(7, 1000, 5000, "NEW_CARD", owner, 0, 1, 0, source),
            0
        );
    }
    let combat = state
        .current
        .as_ref()
        .expect("rejected plays preserve the combat");
    assert_eq!(combat.plays, before.plays);
    assert_eq!(combat.cards, before.cards);
    assert_eq!(state.card_play_finished(plays[0]), 1);
    let replacement = state.card_play_started(7, 2000, 5000, "CARD", 0, 0, 1, 0, source);
    assert_ne!(replacement, 0);
    assert_ne!(replacement, plays[0]);
    assert_eq!(state.card_play_finished(plays[0]), 0);
    assert_eq!(state.card_play_finished(replacement), 1);
}

#[test]
fn generated_admission_preserves_counters_and_bad_replacements_invalidate_suppliers() {
    let mut state = fixture();
    let source = state.source_capture(7, 4, 0, "RELIC", 1, 0, 0);
    for instance in 1..=caps::GENERATED_INSTANCES as u64 {
        assert_eq!(state.card_generated(7, instance, source, 3), 1);
    }
    let before = state
        .current
        .as_ref()
        .expect("fixture combat remains active")
        .clone();
    assert_eq!(state.card_generated(7, 1000, source, 3), 0);
    let combat = state
        .current
        .as_ref()
        .expect("rejected generation preserves the combat");
    assert_eq!(combat.generation_triggers, before.generation_triggers);
    assert_eq!(combat.cards, before.cards);
    assert_eq!(state.card_generated(7, 1, u64::MAX, 3), 0);
    let generated = state.source_capture(7, 1, 1, "GENERATED", 0, 0, 1);
    assert_eq!(state.source_count(generated), 1);
    assert_eq!(
        state.source_destination(generated, 0),
        Destination::Unknown(TEAM_SLOT)
            .token(CombatEpoch::from_wire(7).expect("fixture epoch is valid"))
    );
    assert_eq!(state.orb_channeled(7, 100, source), 1);
    assert_eq!(state.orb_channeled(7, 100, u64::MAX), 0);
    assert_eq!(state.orb_context_begin(7, 100, 0, 0), 0);
}

#[test]
fn grant_release_and_tracking_reset_retain_every_initialized_source_buffer() {
    let mut state = fixture();
    let source = card(&mut state, "CARD");
    let mut before: Vec<_> = state
        .provenance
        .grants
        .all_slots()
        .iter()
        .map(|entry| entry.grant.source().shares().as_ptr() as usize)
        .collect();
    before.sort_unstable();
    for instance in 1..=20 {
        assert_eq!(
            state.power_attached(7, instance, "POWER", instance + 100, 1, 4, 1, source),
            1
        );
    }
    for instance in (1..=20).step_by(2) {
        assert_eq!(state.power_removed(7, instance), 1);
    }
    state.provenance.clear_tracking();
    for instance in 21..=40 {
        assert_eq!(
            state.power_attached(7, instance, "POWER", instance + 100, 1, 4, 1, source),
            1
        );
    }
    let mut after: Vec<_> = state
        .provenance
        .grants
        .all_slots()
        .iter()
        .map(|entry| entry.grant.source().shares().as_ptr() as usize)
        .collect();
    after.sort_unstable();
    assert_eq!(after, before);
    assert_eq!(state.provenance.grants.len(), 20);
}

#[test]
fn missing_identity_counts_observed_events_even_when_retained_tables_are_full() {
    let mut state = fixture();
    let source = state.source_capture(7, 4, 0, "RELIC", 1, 0, 0);
    for instance in 1..=caps::GENERATED_INSTANCES as u64 {
        assert_eq!(state.card_generated(7, instance, source, 3), 1);
    }
    let before = state
        .current
        .as_ref()
        .expect("fixture combat remains active")
        .generation_triggers;
    assert_eq!(state.card_generated(7, 0, source, 3), 0);
    assert_eq!(
        state
            .current
            .as_ref()
            .expect("observation preserves the combat")
            .generation_triggers,
        before + 1
    );
    assert_eq!(state.provenance.generated.len(), caps::GENERATED_INSTANCES);
    for execution in 1..=caps::ACTIVE_PLAYS_PER_SLOT as u64 {
        assert_ne!(
            state.card_play_started(7, execution, 5000, "CARD", 0, 0, 1, 0, source),
            0
        );
    }
    let before = state
        .current
        .as_ref()
        .expect("fixture combat remains active")
        .plays;
    assert_eq!(
        state.card_play_started(7, 0, 0, "UNCLASSIFIED", 0, 0, 1, 3, 0),
        0
    );
    assert_eq!(
        state
            .current
            .as_ref()
            .expect("observation preserves the combat")
            .plays,
        before + 1
    );
    assert_eq!(state.provenance.plays.len(), caps::ACTIVE_PLAYS_PER_SLOT);
}
