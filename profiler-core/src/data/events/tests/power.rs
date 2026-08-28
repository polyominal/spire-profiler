//! Power tests: power/debuff/doom attribution, mitigations, Osty summons,
//! forge credits, and the per-slot applier and TEAM context rows.

use super::*;
use crate::source_kind::SourceKind;
use crate::test_util::wiped_dir;

#[test]
fn forge_credits_the_named_source() {
    let base = wiped_dir("spire-profiler-test-forge");
    test_reset();
    init(&base);
    combat_started("FORGE_TEST", "test");

    forge("FURNACE_POWER", 2, 3, 0);

    combat_ended();

    let (combat, doc) = read_combat(&base);
    let furnace = card_row(&combat, "FURNACE_POWER");
    assert_eq!((furnace.kind, furnace.plays), (SourceKind::Power, 0));
    assert_eq!(card_json(&doc, "FURNACE_POWER")["forge"], 3);
}

#[test]
fn doom_kills_attribute_enemy_hp_to_the_doom_appliers() {
    let base = wiped_dir("spire-profiler-test-doom");
    test_reset();
    init(&base);
    combat_started("DOOM_TEST", "test");

    card_play_started("DEATHBRINGER", 0, 1, 0, 0);
    power_applied("DOOM_POWER", 4, 42, 0, 0);
    card_play_finished(0);
    context_begin("COUNTDOWN_POWER", 2, 0);
    power_applied("DOOM_POWER", 6, 42, 0, 0);
    context_end();

    doom_target_capture(42, 10);
    doom_kills_completed();
    card_play_started("NEUROSURGE", 0, 1, 0, 0);
    power_applied("DOOM_POWER", 5, 0, 1, 0);
    card_play_finished(0);
    // No layer and no context: falls back to the DOOM entry; the lingering
    // last_source must NOT be consulted (AsyncFallback::Skip).
    doom_target_capture(43, 7);
    doom_kills_completed();
    combat_ended();

    let (combat, _) = read_combat(&base);
    let deathbringer = card_row(&combat, "DEATHBRINGER");
    assert_eq!(
        (
            deathbringer.kind,
            deathbringer.plays,
            deathbringer.damage_dealt
        ),
        (SourceKind::Card, 1, 4)
    );
    let countdown = card_row(&combat, "COUNTDOWN_POWER");
    assert_eq!(
        (countdown.kind, countdown.plays, countdown.damage_dealt),
        (SourceKind::Power, 0, 6)
    );
    let neuro = card_row(&combat, "NEUROSURGE");
    assert_eq!(
        (neuro.kind, neuro.plays, neuro.damage_dealt),
        (SourceKind::Card, 1, 0)
    );
    let doom = card_row(&combat, "DOOM");
    assert_eq!(
        (doom.kind, doom.plays, doom.damage_dealt),
        (SourceKind::Card, 0, 7)
    );
}

#[test]
fn osty_summons_absorb_damage_for_the_player() {
    let base = wiped_dir("spire-profiler-test-osty");
    test_reset();
    init(&base);
    combat_started("OSTY_TEST", "test");

    card_play_started("BODYGUARD", 0, 1, 0, 0);
    osty_summoned("BODYGUARD", 0, 5, 0);
    card_play_finished(0);
    context_begin("BOUND_PHYLACTERY", 1, 0);
    osty_summoned("BOUND_PHYLACTERY", 1, 1, 0);
    context_end();

    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        osty_flag: 2,
        ..DamageDealt::default()
    });
    damage_dealt(DamageDealt {
        total: 4,
        unblocked: 4,
        osty_flag: 2,
        ..DamageDealt::default()
    });

    damage_dealt(DamageDealt {
        total: 9,
        unblocked: 9,
        card_source_id: "UNLEASH",
        osty_flag: 1,
        ..DamageDealt::default()
    });

    osty_killed(0);
    combat_ended();

    let (combat, _) = read_combat(&base);
    let bodyguard = card_row(&combat, "BODYGUARD");
    assert_eq!((bodyguard.kind, bodyguard.plays), (SourceKind::Card, 1));
    let phylactery = card_row(&combat, "BOUND_PHYLACTERY");
    assert_eq!(phylactery.kind, SourceKind::Relic);
    assert_eq!(bodyguard.block_effective, 5);
    assert_eq!(phylactery.block_effective, 1);
    let osty = card_row(&combat, "OSTY");
    assert_eq!(osty.kind, SourceKind::Osty);
    let unleash = card_row(&combat, "UNLEASH");
    assert_eq!(
        (unleash.kind, unleash.plays, unleash.damage_dealt),
        (SourceKind::Card, 0, 9)
    );
}

#[test]
fn debuff_layers_attribute_poison_ticks_to_appliers() {
    let base = wiped_dir("spire-profiler-test-debuff");
    test_reset();
    init(&base);
    combat_started("DEBUFF_TEST", "test");

    card_play_started("BOUNCING_FLASK", 0, 1, 0, 0);
    power_applied("POISON_POWER", 3, 42, 0, 0);
    card_play_finished(0);
    context_begin("NOXIOUS_FUMES_POWER", 2, 0);
    power_applied("POISON_POWER", 2, 42, 0, 0);
    context_end();

    damage_dealt(DamageDealt {
        total: 5,
        unblocked: 5,
        receiver_hash: 42,
        ..DamageDealt::default()
    });
    power_decreased("POISON_POWER", 1, 42, 0, 0);
    combat_ended();

    let (combat, _) = read_combat(&base);
    let flask = card_row(&combat, "BOUNCING_FLASK");
    assert_eq!(
        (flask.kind, flask.plays, flask.damage_dealt),
        (SourceKind::Card, 1, 3)
    );
    let noxious = card_row(&combat, "NOXIOUS_FUMES_POWER");
    assert_eq!(
        (noxious.kind, noxious.plays, noxious.damage_dealt),
        (SourceKind::Power, 0, 2)
    );
}

#[test]
fn weak_and_buff_mitigation_credit_their_appliers() {
    let base = wiped_dir("spire-profiler-test-mitigation");
    test_reset();
    init(&base);
    combat_started("MITIGATION_TEST", "test");

    card_play_started("MALAISE", 0, 1, 0, 0);
    power_applied("WEAK_POWER", 2, 42, 0, 0);
    card_play_finished(0);
    card_play_started("GO_FOR_THE_EYES", 0, 1, 0, 0);
    power_applied("WEAK_POWER", 1, 42, 0, 0);
    card_play_finished(0);
    weak_mitigation(4, 42);

    card_play_started("BUFFER", 0, 1, 0, 0);
    power_applied("BUFFER_POWER", 1, 0, 1, 0);
    card_play_finished(0);
    buff_mitigation("BUFFER_POWER", 6);
    // No recorded applier: the power entry itself takes the credit.
    buff_mitigation("INTANGIBLE_POWER", 4);

    combat_ended();

    let (combat, doc) = read_combat(&base);
    assert_eq!(card_json(&doc, "MALAISE")["mitigate_debuff"], 4);
    let eyes = card_row(&combat, "GO_FOR_THE_EYES");
    assert_eq!(
        (eyes.kind, eyes.plays, eyes.damage_dealt),
        (SourceKind::Card, 1, 0)
    );
    assert_eq!(card_json(&doc, "BUFFER")["mitigate_buff"], 6);
    let intangible = card_row(&combat, "INTANGIBLE_POWER");
    assert_eq!(intangible.kind, SourceKind::Power);
    assert_eq!(card_json(&doc, "INTANGIBLE_POWER")["mitigate_buff"], 4);
}

#[test]
fn strength_reduction_records_mitigates_and_reverts_lifo() {
    let base = wiped_dir("spire-profiler-test-strred");
    test_reset();
    init(&base);
    combat_started("STRRED_TEST", "test");

    card_play_started("PIERCING_WAIL", 0, 1, 0, 0);
    power_decreased("STRENGTH_POWER", 8, 42, 0, 0);
    card_play_finished(0);
    card_play_started("MALAISE", 0, 1, 0, 0);
    power_decreased("STRENGTH_POWER", 2, 42, 0, 0);
    card_play_finished(0);

    enemy_hit_context(10, -4);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });

    power_applied("STRENGTH_POWER", 8, 42, 0, 0);
    enemy_hit_context(10, 4);
    damage_dealt(DamageDealt {
        total: 14,
        unblocked: 14,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });

    combat_ended();

    let (_, doc) = read_combat(&base);
    assert_eq!(card_json(&doc, "PIERCING_WAIL")["mitigate_str"], 10);
    assert_eq!(card_json(&doc, "MALAISE")["mitigate_str"], 2);
}

/// The TEAM value must not fabricate a `per_player` entry.
#[test]
fn team_slot_context_keys_a_team_row() {
    let base = wiped_dir("spire-profiler-test-mp2-teamctx");
    test_reset();
    init(&base);
    combat_started("MP2_TEAMCTX_TEST", "test");

    context_begin("MALEVOLENCE_POWER", 2, 4);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        ..DamageDealt::default()
    });
    context_end();
    combat_ended();

    let rows = current_rows();
    let row = rows
        .iter()
        .find(|c| c.id == "MALEVOLENCE_POWER")
        .expect("TEAM context row");
    assert_eq!(row.player, 4);
    assert_eq!(row.damage_dealt, 3);
    STATE.with(|cell| {
        let st = cell.borrow();
        assert_eq!(
            st.per_player.len(),
            1,
            "the TEAM slot must not grow per_player (only slot 0 appeared)"
        );
    });
}

/// A contribution splits across both rows, each keyed at its applier's slot.
#[test]
fn power_appliers_record_their_slots() {
    let base = wiped_dir("spire-profiler-test-mp2-power");
    test_reset();
    init(&base);
    combat_started("MP2_POWER_TEST", "test");

    card_play_started("INFLAME", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 2, 0, 1, 0);
    card_play_finished(0);
    card_play_started("INFLAME", 0, 1, 0, 1);
    power_applied("STRENGTH_POWER", 3, 0, 1, 1);
    card_play_finished(1);
    STATE.with(|cell| {
        let st = cell.borrow();
        let strength: Vec<_> = st
            .power_sources
            .iter()
            .filter(|e| e.power_id == "STRENGTH_POWER")
            .collect();
        assert_eq!(strength.len(), 2, "per-slot applier records, not merged");
        assert!(strength.iter().any(|e| e.player == 0 && e.amount == 2));
        assert!(strength.iter().any(|e| e.player == 1 && e.amount == 3));
    });
    card_play_started("BASH", 0, 1, 0, 1);
    damage_modifier_contribution("STRENGTH_POWER", 2, 5, 1);
    damage_dealt(DamageDealt {
        total: 11,
        unblocked: 11,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    card_play_finished(1);
    combat_ended();

    let rows = current_rows();
    let p1 = rows
        .iter()
        .find(|c| c.player == 0 && c.id == "INFLAME")
        .expect("P1's INFLAME row");
    assert_eq!(p1.dmg_modifier, 2);
    let p2 = rows
        .iter()
        .find(|c| c.player == 1 && c.id == "INFLAME")
        .expect("P2's INFLAME row");
    assert_eq!(p2.dmg_modifier, 3);
    let bash = rows
        .iter()
        .find(|c| c.player == 1 && c.id == "BASH")
        .expect("P2's BASH row");
    assert_eq!(bash.damage_dealt, 6, "BASH keeps its base 6");
}
