use super::*;
use crate::data::state::{CombatResult, RunOutcome};

#[test]
fn team_defeat_requires_every_slot_dead() {
    let base = unique_dir("spire-profiler-test-mp-defeat");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 0, "Standard", "SEED_MP_DEFEAT", 0, "", 0);
    for all_dead in [false, true] {
        combat_started("MP_DEFEAT_TEST", "test");
        let defend = SourceFixture::card("DEFEND", 1);
        let play = defend.play();
        defend.finish(play);
        player_died(combat_epoch(), 0);
        if all_dead {
            player_died(combat_epoch(), 1);
            player_died(combat_epoch(), 1);
        }
        combat_ended(combat_epoch());
    }
    run_ended(RunOutcome::Defeat);
    let combats: Vec<CombatRec> = read_all_combats(&base)
        .into_iter()
        .map(|(record, _)| record)
        .collect();
    assert_eq!(combats.len(), 2, "both combat records persisted");
    assert_eq!(
        combats[0].result,
        CombatResult::Completed,
        "a surviving teammate keeps the team alive"
    );
    assert_eq!(
        combats[1].result,
        CombatResult::Defeat,
        "all players dead is a team defeat"
    );
}

#[test]
fn invalid_damage_packets_reject_and_zero_groups_close_without_credit() {
    let base = combat_fixture("NEGATIVE_TEST");
    let strike = SourceFixture::card("STRIKE", 0);
    let play = strike.play();
    strike.deal(5, 0);
    for (total, unblocked, blocked) in [
        (-3, -3, 0),
        (5, 7, -2),
        (5, 2, 2),
        (5, 0, 6),
        (i32::MAX, i32::MAX, 1),
    ] {
        let calculation = strike
            .with_transfer(|source| damage_calculation_begin(combat_epoch(), source, 1, 0, 999));
        assert_eq!(
            damage_result_append(calculation, total, unblocked, blocked, 0, 4, 0),
            0
        );
        assert_eq!(damage_calculation_commit(calculation), 0);
    }
    let zero =
        strike.with_transfer(|source| damage_calculation_begin(combat_epoch(), source, 1, 0, 999));
    assert_eq!(damage_result_append(zero, 0, 0, 0, 0, 4, 0), 1);
    assert_eq!(damage_calculation_commit(zero), 1);
    assert_eq!(damage_calculation_commit(zero), 0);
    strike.block(4, 0);
    assert_eq!(
        strike.with_transfer(|source| block_gained(combat_epoch(), -2, source, 0)),
        0
    );
    assert_eq!(
        strike.with_transfer(|source| block_gained(combat_epoch(), 0, source, 0)),
        1
    );
    strike.finish(play);
    combat_ended(combat_epoch());
    let (combat, _) = read_combat(&base);
    assert_eq!(combat.damage_received, 0);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(strike.damage_dealt, 5);
    assert_eq!(strike.block_gained, 4);
    assert_eq!(combat.cards.len(), 1, "no rows for the rejected packets");
    STATE.with(|cell| {
        assert_eq!(
            cell.borrow()
                .current
                .as_ref()
                .expect("combat exists")
                .block_total,
            4
        )
    });
}
