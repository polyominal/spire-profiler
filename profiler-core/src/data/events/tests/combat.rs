//! Combat tests: damage/block attribution (hook contexts, modifier
//! decomposition, segments, block pools, self damage, async-gap credits)
//! and the per-slot co-op shapes.

use super::*;
use crate::data::records::CombatRec;
use crate::data::state::RunOutcome;
use crate::source_kind::SourceKind;
use crate::test_util::scratch_dir;

#[test]
fn relic_and_power_contexts_attribute_damage_and_block() {
    let base = scratch_dir("spire-profiler-test-context");
    test_reset();
    init(&base);
    combat_started("CONTEXT_TEST", "test");

    context_begin("MERCURY_HOURGLASS", 1, 0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        ..DamageDealt::default()
    });
    context_end();

    context_begin("VAJRA", 1, 0);
    card_play_started("STRIKE", 0, 1, 0, 0);
    context_begin("STRENGTH_POWER", 2, 0);
    damage_dealt(DamageDealt {
        total: 8,
        unblocked: 8,
        ..DamageDealt::default()
    });
    context_end();
    card_play_finished(0);
    context_end();

    damage_dealt(DamageDealt {
        total: 5,
        unblocked: 5,
        ..DamageDealt::default()
    });

    combat_ended();

    let (combat, _) = read_combat(&base);
    let mercury = card_row(&combat, "MERCURY_HOURGLASS");
    assert_eq!((mercury.kind, mercury.damage_dealt), (SourceKind::Relic, 3));
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 8)
    );
    assert_no_card(&combat, "VAJRA");
    let strength = card_row(&combat, "STRENGTH_POWER");
    assert_eq!(
        (strength.kind, strength.plays, strength.damage_dealt),
        (SourceKind::Power, 0, 5)
    );
}

#[test]
fn power_source_decomposition_splits_modifier_damage_across_appliers() {
    let base = scratch_dir("spire-profiler-test-decomp");
    test_reset();
    init(&base);
    combat_started("DECOMP_TEST", "test");

    card_play_started("DEMON_FORM", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 3, 0, 1, 0);
    card_play_finished(0);
    card_play_started("INFLAME", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 2, 0, 1, 0);
    card_play_finished(0);

    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_modifier_contribution("STRENGTH_POWER", 2, 5, 0);
    damage_dealt(DamageDealt {
        total: 11,
        unblocked: 11,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();

    let (combat, _) = read_combat(&base);
    let demon = card_row(&combat, "DEMON_FORM");
    assert_eq!(
        (demon.kind, demon.plays, demon.damage_dealt),
        (SourceKind::Card, 1, 3)
    );
    let inflame = card_row(&combat, "INFLAME");
    assert_eq!(
        (inflame.kind, inflame.plays, inflame.damage_dealt),
        (SourceKind::Card, 1, 2)
    );
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    assert_eq!(strike.damage_dealt - strike.damage_blocked, 6);
}

#[test]
fn damage_segments_split_direct_modifier_and_attributed() {
    let base = scratch_dir("spire-profiler-test-segments");
    test_reset();
    init(&base);
    combat_started("SEGMENT_TEST", "test");

    card_play_started("INFLAME", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 2, 0, 1, 0);
    card_play_finished(0);
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_modifier_contribution("STRENGTH_POWER", 2, 2, 0);
    damage_dealt(DamageDealt {
        total: 8,
        unblocked: 8,
        card_source_id: "STRIKE",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    card_play_started("BOUNCING_FLASK", 0, 1, 0, 0);
    power_applied("POISON_POWER", 3, 42, 0, 0);
    card_play_finished(0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        receiver_hash: 42,
        ..DamageDealt::default()
    });

    card_play_started("ARMAMENTS", 0, 1, 0, 0);
    card_play_finished(0);
    card_play_started("BASH", 0, 1, 6001, 0);
    damage_dealt(DamageDealt {
        total: 10,
        unblocked: 10,
        card_source_id: "BASH",
        ..DamageDealt::default()
    });
    card_play_finished(0);

    combat_ended();

    let (combat, doc) = read_combat(&base);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    assert_eq!(card_json(&doc, "STRIKE")["dmg_direct"], 6);
    assert_eq!(card_json(&doc, "INFLAME")["dmg_modifier"], 2);
    assert_eq!(card_json(&doc, "BOUNCING_FLASK")["dmg_attributed"], 3);
    let bash = card_row(&combat, "BASH");
    assert_eq!(
        (bash.kind, bash.plays, bash.damage_dealt),
        (SourceKind::Card, 1, 10)
    );
}

#[test]
fn block_pool_splits_modifier_portions_on_consumption() {
    let base = scratch_dir("spire-profiler-test-blocksplit");
    test_reset();
    init(&base);
    combat_started("BLOCKSPLIT_TEST", "test");

    card_play_started("FOOTWORK", 0, 1, 0, 0);
    power_applied("DEXTERITY_POWER", 2, 0, 1, 0);
    card_play_finished(0);
    card_play_started("DEFEND", 0, 1, 0, 0);
    block_modifier_contribution("DEXTERITY_POWER", 2, 2, 0);
    block_gained(5, "DEFEND", 0, 0);
    card_play_finished(0);

    // First hit absorbs 4: base 3*4/5=2, mod 2*4/5=1, residue on base.
    damage_dealt(DamageDealt {
        total: 4,
        blocked: 4,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });
    damage_dealt(DamageDealt {
        total: 1,
        blocked: 1,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });
    combat_ended();

    let (combat, doc) = read_combat(&base);
    let defend = card_row(&combat, "DEFEND");
    assert_eq!((defend.kind, defend.plays), (SourceKind::Card, 1));
    assert_eq!(defend.block_effective, 3);
    assert_eq!(card_json(&doc, "FOOTWORK")["blk_modifier"], 2);
}

/// The Crimson Mantle shape: a proc fires after its hook's first await.
#[test]
fn block_after_a_hooks_async_gap_credits_the_hooks_source() {
    let base = scratch_dir("spire-profiler-test-asyncgap");
    test_reset();
    init(&base);
    combat_started("ASYNC_GAP_TEST", "test");
    turn_started();

    context_begin("CRIMSON_MANTLE_POWER", 2, 0);
    damage_dealt(DamageDealt {
        total: 1,
        unblocked: 1,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 999,
        ..DamageDealt::default()
    });
    context_end();
    block_gained(7, "", 0, 0);
    combat_ended();

    let (combat, _) = read_combat(&base);
    let mantle = card_row(&combat, "CRIMSON_MANTLE_POWER");
    assert_eq!((mantle.kind, mantle.block_gained), (SourceKind::Power, 7));
    assert_eq!(mantle.self_damage, 1);
}

#[test]
fn self_damage_credits_the_sources_red_segment() {
    let base = scratch_dir("spire-profiler-test-selfdmg");
    test_reset();
    init(&base);
    combat_started("SELFDMG_TEST", "test");

    card_play_started("BLOODLETTING", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        card_source_id: "BLOODLETTING",
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 999,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("OFFERING", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        card_source_id: "OFFERING",
        to_player: 1,
        receiver_hash: 999,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    damage_dealt(DamageDealt {
        total: 10,
        unblocked: 8,
        blocked: 2,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });
    combat_ended();

    let (combat, doc) = read_combat(&base);
    assert_eq!(card_json(&doc, "BLOODLETTING")["self_damage"], 3);
    assert_eq!(card_json(&doc, "OFFERING")["self_damage"], 6);
    assert_eq!(combat.damage_received, 19);
    let cards = doc[0]["cards"].as_array().expect("combat cards array");
    assert!(
        cards.iter().all(|c| c["self_damage"] != 8),
        "the enemy hit must not be credited as self damage"
    );
}

/// `block_pool_clear` for one slot never touches another's pool.
#[test]
fn block_pools_consume_and_clear_per_slot() {
    let base = scratch_dir("spire-profiler-test-mp-blockpool");
    test_reset();
    init(&base);
    combat_started("MP_BLOCKPOOL_TEST", "test");

    card_play_started("DEFEND", 0, 1, 0, 0);
    block_gained(5, "DEFEND", 0, 0);
    card_play_finished(0);
    card_play_started("DEFEND", 0, 1, 0, 1);
    block_gained(7, "DEFEND", 1, 1);
    card_play_finished(1);
    block_pool_clear(0);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(state.per_player[0].block_pool.is_empty());
        assert_eq!(state.per_player[1].block_pool.len(), 1);
        assert_eq!(state.per_player[1].block_pool[0].remaining, 7);
    });
    damage_dealt(DamageDealt {
        total: 4,
        blocked: 4,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        receiver_slot: 1,
        ..DamageDealt::default()
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[1].block_pool[0].remaining, 3);
    });
    combat_ended();

    // P1's DEFEND kept its whole 5; P2's row carries the 7 gained and the
    // 4 consumed from its own pool.
    let rows = current_rows();
    let defend0 = rows
        .iter()
        .find(|c| c.player == 0 && c.id == "DEFEND")
        .expect("P1 DEFEND row");
    assert_eq!((defend0.block_gained, defend0.block_effective), (5, 0));
    let defend1 = rows
        .iter()
        .find(|c| c.player == 1 && c.id == "DEFEND")
        .expect("P2 DEFEND row");
    assert_eq!((defend1.block_gained, defend1.block_effective), (7, 4));
}

#[test]
fn pending_modifier_contribs_apply_per_slot() {
    let base = scratch_dir("spire-profiler-test-mp-contrib");
    test_reset();
    init(&base);
    combat_started("MP_CONTRIB_TEST", "test");

    card_play_started("STRIKE", 0, 1, 0, 0);
    card_play_started("BASH", 0, 1, 0, 1);
    damage_modifier_contribution("STRENGTH_POWER", 2, 3, 1);
    damage_dealt(DamageDealt {
        total: 9,
        unblocked: 9,
        dealer_slot: 1,
        ..DamageDealt::default()
    });
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_finished(1);
    combat_ended();

    let (combat, doc) = read_combat(&base);
    let bash = card_row(&combat, "BASH");
    assert_eq!(
        (bash.kind, bash.plays, bash.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    let strength = card_row(&combat, "STRENGTH_POWER");
    assert_eq!(
        (strength.kind, strength.damage_dealt),
        (SourceKind::Power, 3)
    );
    assert_eq!(card_json(&doc, "STRENGTH_POWER")["dmg_modifier"], 3);
    let strike = card_row(&combat, "STRIKE");
    assert_eq!(
        (strike.kind, strike.plays, strike.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
}

/// A partial death stays "completed"; a full wipe is "defeat".
#[test]
fn team_defeat_requires_every_slot_dead() {
    let base = scratch_dir("spire-profiler-test-mp-defeat");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 0, "Standard", "SEED_MP_DEFEAT", 0, "", 0);
    combat_started("MP_DEFEAT_TEST", "test");
    card_play_started("DEFEND", 0, 1, 0, 1);
    card_play_finished(1);
    player_died(0);
    combat_ended();

    combat_started("MP_DEFEAT_TEST", "test");
    card_play_started("DEFEND", 0, 1, 0, 1);
    card_play_finished(1);
    player_died(0);
    player_died(1);
    player_died(1);
    combat_ended();
    run_ended(RunOutcome::Defeat);

    let combats: Vec<CombatRec> = read_all_combats(&base)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert_eq!(combats.len(), 2, "both combat records persisted");
    assert_eq!(
        combats[0].result, "completed",
        "a surviving teammate keeps the team alive"
    );
    assert_eq!(
        combats[1].result, "defeat",
        "all players dead is a team defeat"
    );
}

/// The turn boundary is team-wide.
#[test]
fn turn_started_clears_every_slots_fallbacks() {
    let base = scratch_dir("spire-profiler-test-mp-turn");
    test_reset();
    init(&base);
    combat_started("MP_TURN_TEST", "test");

    card_play_started("ZAP", 0, 1, 0, 0);
    orb_channeled(1001, 0);
    orb_context_begin(1001, 0);
    card_play_finished(0);
    potion_context_begin("FIRE_POTION", 1);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(state.per_player[0].orb_fallback.is_some());
        assert!(state.per_player[1].potion_fallback.is_some());
    });
    turn_started();
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(state.per_player[0].orb_fallback.is_none());
        assert!(state.per_player[0].potion_fallback.is_none());
        assert!(state.per_player[1].orb_fallback.is_none());
        assert!(state.per_player[1].potion_fallback.is_none());
    });
    combat_ended();
}
