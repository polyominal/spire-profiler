//! Orb and potion tests: potion fallback attribution and expiry, the
//! orb-source cap, and the fallback clearing rules.

use super::*;
use crate::data::state::OrbSource;
use crate::test_util::scratch_dir;

#[test]
fn flex_potion_strength_attributes_to_the_potion_and_expires_fifo() {
    let base = scratch_dir("spire-profiler-test-flexpotion");
    test_reset();
    init(&base);
    combat_started("FLEX_TEST", "test");

    potion_context_begin("FLEX_POTION", 0);
    power_applied("STRENGTH_POWER", 5, 0, 1, 0);
    potion_used("FLEX_POTION", 0);
    STATE.with(|cell| {
        let state = cell.borrow();
        let entry = state
            .power_sources
            .iter()
            .find(|entry| entry.power_id == "STRENGTH_POWER")
            .expect("strength source recorded");
        assert_eq!(entry.source_id, "FLEX_POTION");
        assert_eq!(entry.kind, SourceKind::Potion);
        assert_eq!(entry.amount, 5);
    });

    card_play_started("STRIKE", 0, 1, 0, 0);
    for _ in 0..4 {
        damage_modifier_contribution("STRENGTH_POWER", 2, 5, 0);
        damage_dealt(DamageDealt {
            total: 11,
            unblocked: 11,
            ..DamageDealt::default()
        });
    }
    card_play_finished(0);

    power_decreased("STRENGTH_POWER", 5, 0, 1, 0);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(
            !state
                .power_sources
                .iter()
                .any(|entry| entry.power_id == "STRENGTH_POWER"),
            "expired strength source removed from the table"
        );
    });

    card_play_started("INFLAME", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 2, 0, 1, 0);
    card_play_finished(0);
    card_play_started("STRIKE", 1, 2, 0, 0);
    damage_modifier_contribution("STRENGTH_POWER", 2, 2, 0);
    damage_dealt(DamageDealt {
        total: 8,
        unblocked: 8,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();

    let (combat, doc) = read_combat(&base);
    let potion = card_row(&combat, "FLEX_POTION");
    assert_eq!(potion.kind, 3);
    assert_eq!(card_json(&doc, "FLEX_POTION")["dmg_modifier"], 20);
    let inflame = card_row(&combat, "INFLAME");
    assert_eq!(inflame.kind, 0);
    assert_eq!(card_json(&doc, "INFLAME")["dmg_modifier"], 2);
    assert_eq!(
        STATE.with(|cell| cell
            .borrow()
            .current
            .as_ref()
            .expect("combat exists")
            .potions_used),
        1
    );
    let strike = card_row(&combat, "STRIKE");
    assert_eq!((strike.kind, strike.plays, strike.damage_dealt), (0, 2, 30));
}

#[test]
fn potion_sources_are_bounded_at_the_orb_source_cap() {
    let base = scratch_dir("spire-profiler-test-potioncap");
    test_reset();
    init(&base);
    combat_started("POTION_CAP_TEST", "test");
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        for i in 0..caps::ORB_SOURCES {
            state.orb_sources.push(OrbSource {
                hash: 1000 + i as i32,
                id: format!("ORB_{i}"),
                kind: SourceKind::Card,
            });
        }
    });

    potion_context_begin("FLEX_POTION", 0);
    potion_used("FLEX_POTION", 0);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.orb_sources.len(), caps::ORB_SOURCES);
        assert!(state.per_player[0].potion_fallback.is_none());
        let combat = state.current.as_ref().expect("combat exists");
        assert_eq!(combat.potions_used, 1);
    });
    combat_ended();
}

/// A potion used after an orb trigger must clear the live orb fallback
/// first — otherwise the orb's source would capture the potion's effects.
#[test]
fn potion_use_clears_a_live_orb_fallback() {
    let base = scratch_dir("spire-profiler-test-potionorb");
    test_reset();
    init(&base);
    combat_started("POTION_ORB_TEST", "test");

    card_play_started("ZAP", 0, 1, 0, 0);
    orb_channeled(1002, 0);
    orb_context_begin(1002, 0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        ..DamageDealt::default()
    });
    card_play_finished(0);

    potion_context_begin("FIRE_POTION", 0);
    damage_dealt(DamageDealt {
        total: 20,
        unblocked: 20,
        ..DamageDealt::default()
    });
    combat_ended();

    let (combat, doc) = read_combat(&base);
    let fire = card_row(&combat, "FIRE_POTION");
    assert_eq!((fire.kind, fire.plays, fire.damage_dealt), (3, 0, 20));
    assert_eq!(card_json(&doc, "FIRE_POTION")["dmg_direct"], 20);
    let zap = card_row(&combat, "ZAP");
    assert_eq!((zap.kind, zap.plays, zap.damage_dealt), (0, 1, 3));
    assert_eq!(card_json(&doc, "ZAP")["dmg_attributed"], 3);
}

/// Slot 1's play clears only its own fallbacks, so no cross-player capture
/// happens in either direction.
#[test]
fn potion_fallbacks_do_not_capture_across_slots() {
    let base = scratch_dir("spire-profiler-test-mp-potion");
    test_reset();
    init(&base);
    combat_started("MP_POTION_TEST", "test");

    potion_context_begin("FIRE_POTION", 0);
    potion_used("FIRE_POTION", 0);
    card_play_started("STRIKE", 0, 1, 0, 1);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(state.per_player[0].potion_fallback.is_some());
        assert!(state.per_player[1].potion_fallback.is_none());
    });
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    card_play_finished(1);
    damage_dealt(DamageDealt {
        total: 20,
        unblocked: 20,
        ..DamageDealt::default()
    });
    combat_ended();

    let (combat, _) = read_combat(&base);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!((strike.kind, strike.plays, strike.damage_dealt), (0, 1, 6));
    let fire = card_row(&combat, "FIRE_POTION");
    assert_eq!((fire.kind, fire.plays, fire.damage_dealt), (3, 0, 20));
}
