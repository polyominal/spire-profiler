//! Card tests: plays (per-slot play stacks) and generation (the
//! generator-tree credit model and its async-gap resolution).

use super::*;
use crate::source_kind::SourceKind;

#[test]
fn deck_and_generated_copies_of_the_same_card_credit_differently() {
    let base = combat_fixture("GEN_DECK_TEST");

    card_play_started("INFERNAL_BLADE", 0, 1, 0, 0);
    card_generated(7001, "", 0, 0);
    card_play_finished(0);
    card_play_started("PILLAGE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("PILLAGE", 0, 1, 7001, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, doc) = read_combat(&base);
    let pillage = card_row(&combat, "PILLAGE");
    assert_eq!(
        (pillage.kind, pillage.plays, pillage.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    let blade = card_row(&combat, "INFERNAL_BLADE");
    assert_eq!(
        (blade.kind, blade.plays, blade.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    assert_no_key(&doc, "origin");
}

#[test]
fn generated_card_damage_credits_the_generator() {
    let base = combat_fixture("GEN_DMG_TEST");

    card_play_started("INFERNAL_BLADE", 0, 1, 0, 0);
    card_generated(9001, "", 0, 0);
    card_play_finished(0);
    card_play_started("PILLAGE", 0, 1, 9001, 0);
    damage_dealt(DamageDealt {
        total: 9,
        unblocked: 9,
        card_source_id: "PILLAGE",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let log = read_test_file(&base, "profiler.log");
    assert!(
        log.contains(
            "  play: PILLAGE (play 1/1)\n    \
             play credits generator INFERNAL_BLADE (card)\n"
        ),
        "a generated play renders both lines without an owned buffer:\n{log}"
    );
    let (combat, _) = read_combat(&base);
    assert_no_card(&combat, "PILLAGE");
    let blade = card_row(&combat, "INFERNAL_BLADE");
    assert_eq!(
        (blade.kind, blade.plays, blade.damage_dealt),
        (SourceKind::Card, 1, 9)
    );
    let (plays, generated_plays, _) = current_play_counters();
    assert_eq!((plays, generated_plays), (2, 1));
}

/// Playing all three generated cards leaves the generator at plays 1.
#[test]
fn plays_count_the_sources_own_triggers_only() {
    let base = combat_fixture("TRIGGERS_TEST");

    card_play_started("JACKPOT", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 25,
        unblocked: 25,
        card_source_id: "JACKPOT",
        ..DamageDealt::default()
    });
    card_generated(9101, "", 0, 0);
    card_generated(9102, "", 0, 0);
    card_generated(9103, "", 0, 0);
    card_play_finished(0);
    for hash in [9101, 9102, 9103] {
        card_play_started("SHIV", 0, 1, hash, 0);
        damage_dealt(DamageDealt {
            total: 4,
            unblocked: 4,
            card_source_id: "SHIV",
            ..DamageDealt::default()
        });
        card_play_finished(0);
    }
    context_begin("NINJA_SCROLL", 1, 0);
    card_generated(9201, "", 0, 0);
    context_end();
    context_begin("NINJA_SCROLL", 1, 0);
    card_generated(9202, "", 0, 0);
    context_end();
    card_play_started("SHIV", 0, 1, 9201, 0);
    damage_dealt(DamageDealt {
        total: 4,
        unblocked: 4,
        card_source_id: "SHIV",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    assert_no_card(&combat, "SHIV");
    let jackpot = card_row(&combat, "JACKPOT");
    // 1 own play; 25 own + 12 generated damage; EV per trigger = 37/1.
    assert_eq!(
        (jackpot.kind, jackpot.plays, jackpot.damage_dealt),
        (SourceKind::Card, 1, 37)
    );
    let ninja = card_row(&combat, "NINJA_SCROLL");
    assert_eq!(
        (ninja.kind, ninja.plays, ninja.damage_dealt),
        (SourceKind::Relic, 2, 4)
    );
    // The conservation identity: 5 real plays + 2 generation triggers equal
    // Σ row plays (1 + 2) + 4.
    let (plays, generated_plays, generation_triggers) = current_play_counters();
    assert_eq!(plays, 5);
    assert_eq!(generated_plays, 4);
    assert_eq!(generation_triggers, 2);
}

#[test]
fn generated_card_block_credits_the_generator() {
    let base = combat_fixture("GEN_BLK_TEST");

    card_play_started("INFERNAL_BLADE", 0, 1, 0, 0);
    card_generated(9002, "", 0, 0);
    card_play_finished(0);
    card_play_started("IRON_WAVE", 0, 1, 9002, 0);
    block_gained(5, "IRON_WAVE", 0, 0);
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    assert_no_card(&combat, "IRON_WAVE");
    let blade = card_row(&combat, "INFERNAL_BLADE");
    assert_eq!(
        (
            blade.kind,
            blade.plays,
            blade.damage_dealt,
            blade.damage_blocked,
            blade.damage_dealt - blade.damage_blocked,
            blade.block_gained,
        ),
        (SourceKind::Card, 1, 0, 0, 0, 5)
    );
}

/// A DECK copy's explicit id equals the playing card's own id: no-op.
#[test]
fn deck_card_own_id_as_explicit_source_is_a_no_op() {
    let base = combat_fixture("DECK_SELF_TEST");

    card_play_started("IRON_WAVE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 5,
        unblocked: 5,
        card_source_id: "IRON_WAVE",
        ..DamageDealt::default()
    });
    block_gained(5, "IRON_WAVE", 0, 0);
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let wave = card_row(&combat, "IRON_WAVE");
    assert_eq!(
        (wave.kind, wave.plays, wave.damage_dealt),
        (SourceKind::Card, 1, 5)
    );
    assert_eq!(wave.block_gained, 5);
}

#[test]
fn unrelated_explicit_source_beats_the_play_source() {
    let base = combat_fixture("EXPL_SRC_TEST");

    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 4,
        unblocked: 4,
        card_source_id: "MERCURY_HOURGLASS",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 0)
    );
    let mercury = card_row(&combat, "MERCURY_HOURGLASS");
    assert_eq!(
        (mercury.kind, mercury.plays, mercury.damage_dealt),
        (SourceKind::Card, 0, 4)
    );
}

/// The Crossbow shape: the context pops at the first await.
#[test]
fn crossbow_generation_resolves_across_the_async_gap() {
    let base = combat_fixture("GEN_ASYNC_TEST");

    context_begin("CROSSBOW", 1, 0);
    context_end();
    card_generated(7001, "", 0, 0);
    card_play_started("PILLAGE", 0, 1, 7001, 0);
    damage_dealt(DamageDealt {
        total: 2,
        unblocked: 2,
        card_source_id: "PILLAGE",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    assert_no_card(&combat, "PILLAGE");
    let crossbow = card_row(&combat, "CROSSBOW");
    assert_eq!((crossbow.plays, crossbow.damage_dealt), (1, 2));
    let (_, generated_plays, generation_triggers) = current_play_counters();
    assert_eq!((generated_plays, generation_triggers), (1, 1));
}

/// The fallback must never outlive its turn.
#[test]
fn generation_after_the_turn_boundary_has_no_source() {
    let base = combat_fixture("GEN_NOSRC_TEST");

    context_begin("CROSSBOW", 1, 0);
    context_end();
    turn_started();
    card_generated(7002, "", 0, 0);
    card_play_started("PILLAGE", 0, 1, 7002, 0);
    damage_dealt(DamageDealt {
        total: 2,
        unblocked: 2,
        card_source_id: "PILLAGE",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let pillage = card_row(&combat, "PILLAGE");
    assert_eq!((pillage.plays, pillage.damage_dealt), (1, 2));
    assert_no_card(&combat, "CROSSBOW");
    let (_, generated_plays, generation_triggers) = current_play_counters();
    assert_eq!((generated_plays, generation_triggers), (0, 0));
}

/// Generated cards get no ledger entry: the nested play credits the root.
#[test]
fn nested_generation_chain_collapses_to_the_root_generator() {
    let base = combat_fixture("GEN_CHAIN_TEST");

    card_play_started("DISTRACTION", 0, 1, 0, 0);
    card_generated(7001, "", 0, 0);
    card_play_finished(0);
    // The generated BLADE_OF_INK's ambient play source is DISTRACTION (the
    // root), so its nested SHIV resolves to DISTRACTION too.
    card_play_started("BLADE_OF_INK", 0, 1, 7001, 0);
    card_generated(7002, "", 0, 0);
    card_play_finished(0);
    card_play_started("SHIV", 0, 1, 7002, 0);
    damage_dealt(DamageDealt {
        total: 4,
        unblocked: 4,
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let distraction = card_row(&combat, "DISTRACTION");
    assert_eq!(
        (
            distraction.kind,
            distraction.plays,
            distraction.damage_dealt
        ),
        (SourceKind::Card, 1, 4)
    );
    let (plays, generated_plays, _) = current_play_counters();
    assert_eq!((plays, generated_plays), (3, 2));
    assert_no_card(&combat, "BLADE_OF_INK");
    assert_no_card(&combat, "SHIV");
}

#[test]
fn relic_hook_generation_beats_the_ambient_play() {
    let base = combat_fixture("GEN_RELIC_TEST");

    card_play_started("DISTRACTION", 0, 1, 0, 0);
    card_generated(7001, "", 0, 0);
    card_play_finished(0);
    // The generated copy plays as DISTRACTION; mid-play, NINJA_SCROLL's
    // resolution generates a SHIV — the relic context wins over the
    // ambient play.
    card_play_started("BLADE_OF_INK", 0, 1, 7001, 0);
    context_begin("NINJA_SCROLL", 1, 0);
    card_generated(7002, "", 0, 0);
    context_end();
    card_play_finished(0);
    card_play_started("SHIV", 0, 1, 7002, 0);
    damage_dealt(DamageDealt {
        total: 4,
        unblocked: 4,
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let ninja = card_row(&combat, "NINJA_SCROLL");
    assert_eq!(
        (ninja.kind, ninja.plays, ninja.damage_dealt),
        (SourceKind::Relic, 1, 4)
    );
    assert_eq!(current_play_counters().2, 1);
    let distraction = card_row(&combat, "DISTRACTION");
    assert_eq!(
        (
            distraction.kind,
            distraction.plays,
            distraction.damage_dealt
        ),
        (SourceKind::Card, 1, 0)
    );
    assert_no_card(&combat, "SHIV");
}

/// With only a potion fallback active, the play credits the potion.
#[test]
fn potion_generated_card_credits_the_potion_kind() {
    let base = combat_fixture("GEN_POTION_TEST");

    potion_context_begin("ATTACK_POTION", 0);
    card_generated(8001, "", 0, 0);
    potion_used("ATTACK_POTION", 0);
    card_play_started("PILLAGE", 0, 1, 8001, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    let potion = card_row(&combat, "ATTACK_POTION");
    assert_eq!(
        (potion.kind, potion.plays, potion.damage_dealt),
        (SourceKind::Potion, 1, 6)
    );
    assert_no_card(&combat, "PILLAGE");
}

#[test]
fn unplayed_generated_instances_contribute_nothing() {
    let base = combat_fixture("GEN_UNPLAYED_TEST");

    card_play_started("CLOAK_AND_DAGGER", 0, 1, 0, 0);
    block_gained(6, "CLOAK_AND_DAGGER", 0, 0);
    card_generated(7001, "", 0, 0);
    card_generated(7002, "", 0, 0);
    card_play_finished(0);

    combat_ended();

    let (combat, _) = read_combat(&base);
    assert_eq!(combat.cards.len(), 1);
    let cloak = card_row(&combat, "CLOAK_AND_DAGGER");
    assert_eq!(
        (cloak.kind, cloak.plays, cloak.damage_dealt),
        (SourceKind::Card, 1, 0)
    );
    assert_eq!(cloak.block_gained, 6);
}

/// The §1.2 shape: A pauses on a choice, B plays and finishes first, then
/// A resumes — each slot's play stack survives the other's whole play.
#[test]
fn interleaved_plays_keep_per_slot_play_stacks() {
    let base = combat_fixture("MP_INTERLEAVE_TEST");

    card_play_started("STRIKE", 0, 1, 0, 0);
    card_play_started("BASH", 0, 1, 0, 1);
    damage_dealt(DamageDealt {
        total: 10,
        unblocked: 10,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    card_play_finished(1);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[0].play_depth, 1);
        assert_eq!(
            state.per_player[0].active_play_source,
            Some(("STRIKE".to_owned(), SourceKind::Card))
        );
        assert_eq!(state.per_player[1].play_depth, 0);
    });
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();

    let (combat, _) = read_combat(&base);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    let bash = card_row(&combat, "BASH");
    assert_eq!(
        (bash.kind, bash.plays, bash.damage_dealt),
        (SourceKind::Card, 1, 10)
    );
}

/// P1 and P2 each play STRIKE: two rows, keyed (slot, id), never merged.
#[test]
fn same_owner_plays_produce_per_slot_rows() {
    combat_fixture("MP2_SAME_OWNER_TEST");

    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("STRIKE", 0, 1, 0, 1);
    damage_dealt(DamageDealt {
        total: 9,
        unblocked: 9,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    card_play_finished(1);
    combat_ended();

    let rows = current_rows();
    assert_eq!(rows.len(), 2, "one row per player, never merged");
    let p1 = rows
        .iter()
        .find(|c| c.player == 0 && c.id == "STRIKE")
        .expect("P1's STRIKE row");
    assert_eq!((p1.plays, p1.damage_dealt), (1, 6));
    let p2 = rows
        .iter()
        .find(|c| c.player == 1 && c.id == "STRIKE")
        .expect("P2's STRIKE row");
    assert_eq!((p2.plays, p2.damage_dealt), (1, 9));
}

/// P2 plays P1's generated instance: the row keys at the generator's slot.
#[test]
fn generated_card_play_credits_the_generators_slot() {
    combat_fixture("MP2_GENSLOT_TEST");

    card_play_started("INFERNAL_BLADE", 0, 1, 0, 0);
    card_generated(9001, "", 0, 0);
    card_play_finished(0);
    card_play_started("PILLAGE", 0, 1, 9001, 1);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    card_play_finished(1);
    combat_ended();

    let rows = current_rows();
    let blade = rows
        .iter()
        .find(|c| c.id == "INFERNAL_BLADE")
        .expect("generator row");
    assert_eq!(
        blade.player, 0,
        "the generator's slot, not the playing slot"
    );
    assert_eq!((blade.plays, blade.damage_dealt), (1, 6));
    assert!(rows.iter().all(|c| c.id != "PILLAGE"));
}
