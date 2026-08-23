//! The attribution mechanics' unit tests: each test drives the ledger helpers
//! against a fresh core and asserts the resulting state.

use super::*;
use crate::data::state::{DebuffLayer, PowerSourceEntry};
use crate::test_util::temp_dir;

fn reset_state() {
    STATE.with(|cell| *cell.borrow_mut() = state::State::default());
}

fn temp_log_dir(label: &str) {
    let dir = temp_dir(&format!("ledger-{label}"));
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.log_path_full = dir.join("profiler.log");
    });
}

fn start_combat() {
    STATE.with(|cell| {
        cell.borrow_mut().current = Some(Combat::default());
    });
}

fn hit_card(id: &str, amount: i64) -> CardStat {
    CardStat {
        id: id.to_owned(),
        kind: 0,
        damage_dealt: amount,
        dmg_direct: amount,
        ..CardStat::default()
    }
}

fn assert_card(index: usize, id: &str, kind: SourceKind) {
    let card = STATE.with(|cell| {
        let state = cell.borrow();
        state.current.as_ref().expect("combat exists").cards[index].clone()
    });
    assert_eq!(card.id, id);
    assert_eq!(card.kind, kind as u8);
}

fn resolve(explicit_id: &str, slot: i32, explicit_slot: i32) -> Option<(usize, SourceSlot)> {
    STATE.with(|cell| {
        resolve_card_in(
            &mut cell.borrow_mut(),
            explicit_id,
            slot,
            explicit_slot,
            AsyncFallback::Allow,
        )
    })
}

fn push_chunk(id: &str, kind: SourceKind, base: i64) {
    STATE.with(|cell| block_pool_push_in(&mut cell.borrow_mut(), id, kind, base, 0, 0));
}

fn consume_chunk(blocked: i64) -> i64 {
    STATE.with(|cell| block_pool_consume_in(&mut cell.borrow_mut(), blocked, 0))
}

fn resolve_damage_source(
    explicit_id: &str,
    receiver_hash: u64,
    total: i64,
    slot: i32,
    explicit_slot: i32,
) -> Option<DamageRoute> {
    STATE.with(|cell| {
        let mut logs = Vec::new();
        resolve_damage_source_in(
            &mut cell.borrow_mut(),
            explicit_id,
            receiver_hash,
            total,
            slot,
            explicit_slot,
            &mut logs,
        )
    })
}

#[test]
fn u64_from_hash_sign_extends() {
    assert_eq!(u64_from_hash(0), 0);
    assert_eq!(u64_from_hash(1), 1);
    assert_eq!(u64_from_hash(-1), u64::MAX);
    assert_eq!(u64_from_hash(i32::MIN), 0xFFFF_FFFF_8000_0000);
}

/// Potion/osty (3/4) round-trip exactly, unlike `SourceKind::from_c`.
#[test]
fn source_kind_from_u8_round_trips_all_kinds() {
    for kind in [
        SourceKind::Card,
        SourceKind::Relic,
        SourceKind::Power,
        SourceKind::Potion,
        SourceKind::Osty,
    ] {
        assert_eq!(source_kind_from_u8(kind as u8), kind);
    }
}

#[test]
fn consume_debuff_layers_consumes_fifo_and_removes_exhausted() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let layer = |source: &str, duration: i64| DebuffLayer {
            creature_hash: 1,
            power_id: "VULNERABLE_POWER".to_owned(),
            source_id: source.to_owned(),
            kind: SourceKind::Card,
            player: 0,
            duration,
        };
        state.debuff_layers.push(layer("A", 2));
        state.debuff_layers.push(layer("B", 3));
        // Other creature and other power must not be touched.
        state.debuff_layers.push(DebuffLayer {
            creature_hash: 2,
            power_id: "VULNERABLE_POWER".to_owned(),
            source_id: "C".to_owned(),
            kind: SourceKind::Card,
            player: 0,
            duration: 5,
        });
        state.debuff_layers.push(DebuffLayer {
            creature_hash: 1,
            power_id: "POISON_POWER".to_owned(),
            source_id: "D".to_owned(),
            kind: SourceKind::Card,
            player: 0,
            duration: 4,
        });
    });
    STATE.with(|cell| consume_debuff_layers_in(&mut cell.borrow_mut(), 1, "VULNERABLE_POWER", 4));
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.debuff_layers.len(), 3);
        assert_eq!(state.debuff_layers[0].source_id, "B");
        assert_eq!(state.debuff_layers[0].duration, 1);
        assert_eq!(state.debuff_layers[1].source_id, "C");
        assert_eq!(state.debuff_layers[1].duration, 5);
        assert_eq!(state.debuff_layers[2].source_id, "D");
        assert_eq!(state.debuff_layers[2].duration, 4);
    });
}

#[test]
fn attribute_debuff_damage_splits_proportionally() {
    reset_state();
    temp_log_dir("debuff-attr");
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat::default());
        for source in ["A", "B"] {
            state.debuff_layers.push(DebuffLayer {
                creature_hash: 1,
                power_id: "POISON_POWER".to_owned(),
                source_id: source.to_owned(),
                kind: SourceKind::Card,
                player: 0,
                duration: 5,
            });
        }
    });
    assert!(attribute_debuff_damage(1, "POISON_POWER", 10));
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        assert_eq!(combat.cards.len(), 2);
        for (i, source) in ["A", "B"].iter().enumerate() {
            assert_eq!(combat.cards[i].id, *source);
            assert_eq!(combat.cards[i].damage_dealt, 5);
            assert_eq!(combat.cards[i].dmg_attributed, 5);
            assert_eq!(
                combat.cards[i].damage_dealt - combat.cards[i].damage_blocked,
                5
            );
        }
    });
    assert!(!attribute_debuff_damage(9, "POISON_POWER", 10));
}

#[test]
fn resolve_card_priority_chain() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat::default());
        state.context_stack.push(ContextEntry {
            id: "CRACKED_CORE".to_owned(),
            kind: SourceKind::Relic,
            slot: 0,
        });
        state.orb_sources.push(OrbSource {
            hash: 7,
            id: "ZAP".to_owned(),
            kind: SourceKind::Card,
        });
        state.orb_sources.push(OrbSource {
            hash: 0,
            id: "FIRE_POTION".to_owned(),
            kind: SourceKind::Potion,
        });
    });
    assert_card(
        resolve("STRIKE", 0, 0).expect("explicit").0,
        "STRIKE",
        SourceKind::Card,
    );
    STATE.with(|cell| {
        cell.borrow_mut().slot_state_mut(0).active_play_source =
            Some(("DEFEND".to_owned(), SourceKind::Card))
    });
    assert_card(
        resolve("", 0, 0).expect("active card").0,
        "DEFEND",
        SourceKind::Card,
    );
    STATE.with(|cell| cell.borrow_mut().slot_state_mut(0).active_play_source = None);
    assert_card(
        resolve("", 0, 0).expect("context").0,
        "CRACKED_CORE",
        SourceKind::Relic,
    );
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.context_stack.pop();
        state.slot_state_mut(0).orb_fallback = Some(0);
    });
    assert_card(
        resolve("", 0, 0).expect("orb fallback").0,
        "ZAP",
        SourceKind::Card,
    );
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.slot_state_mut(0).orb_fallback = None;
        state.slot_state_mut(0).potion_fallback = Some(1);
    });
    assert_card(
        resolve("", 0, 0).expect("potion fallback").0,
        "FIRE_POTION",
        SourceKind::Potion,
    );
    STATE.with(|cell| clear_fallbacks_in(&mut cell.borrow_mut(), 0));
    assert!(resolve("", 0, 0).is_none());
}

/// The Doom catch-all's Skip variant ignores it: an unrecorded kill stays
/// neutral rather than crediting an earlier hook.
#[test]
fn resolve_card_last_source_is_the_async_gap_fallback() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat::default());
        state.last_source = Some(ContextEntry {
            id: "CRIMSON_MANTLE_POWER".to_owned(),
            kind: SourceKind::Power,
            slot: 0,
        });
    });
    assert_card(
        resolve("", 0, 0).expect("last_source").0,
        "CRIMSON_MANTLE_POWER",
        SourceKind::Power,
    );
    STATE.with(|cell| {
        assert!(resolve_card_in(&mut cell.borrow_mut(), "", 0, 0, AsyncFallback::Skip).is_none());
    });
}

#[test]
fn resolve_card_play_source_override_is_kind_aware() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat::default());
        state.slot_state_mut(0).active_play_source =
            Some(("JOSS_PAPER".to_owned(), SourceKind::Relic));
    });
    assert_card(
        resolve("", 0, 0).expect("relic play source").0,
        "JOSS_PAPER",
        SourceKind::Relic,
    );
}

#[test]
fn resolve_damage_source_route_then_poison_then_last_source() {
    reset_state();
    start_combat();
    let route = resolve_damage_source("STRIKE", 5, 9, 0, 0).expect("explicit route");
    assert!(route.via_card);
    assert!(!route.indirect);
    assert_card(route.card_index, "STRIKE", SourceKind::Card);
    STATE.with(|cell| {
        cell.borrow_mut().debuff_layers.push(DebuffLayer {
            creature_hash: 9,
            power_id: "POISON_POWER".to_owned(),
            source_id: "A".to_owned(),
            kind: SourceKind::Card,
            player: 0,
            duration: 3,
        });
    });
    assert!(resolve_damage_source("", 9, 5, 0, 0).is_none());
    // 3. last_source catches async continuations; a power context is indirect.
    STATE.with(|cell| {
        cell.borrow_mut().last_source = Some(ContextEntry {
            id: "NOXIOUS_FUMES_POWER".to_owned(),
            kind: SourceKind::Power,
            slot: 0,
        });
    });
    let route = resolve_damage_source("", 10, 5, 0, 0).expect("last source route");
    assert!(!route.via_card);
    assert!(route.indirect);
    assert_card(route.card_index, "NOXIOUS_FUMES_POWER", SourceKind::Power);
}

#[test]
fn block_pool_push_merges_modifier_free_chunks() {
    reset_state();
    start_combat();
    push_chunk("DEFEND", SourceKind::Card, 10);
    push_chunk("DEFEND", SourceKind::Card, 5);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[0].block_pool.len(), 1);
        assert_eq!(state.per_player[0].block_pool[0].remaining, 15);
        assert_eq!(state.per_player[0].block_pool[0].base_original, 15);
    });
    // A queued modifier contribution blocks the merge.
    STATE.with(|cell| {
        cell.borrow_mut()
            .slot_state_mut(0)
            .pending_block_contribs
            .push(PendingContrib {
                id: "DEX".to_owned(),
                kind: SourceKind::Power,
                player: 0,
                amount: 5,
            });
    });
    push_chunk("IRON_WAVE", SourceKind::Card, 5);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[0].block_pool.len(), 2);
        assert_eq!(state.per_player[0].block_pool[1].id, "IRON_WAVE");
        assert_eq!(state.per_player[0].block_pool[1].remaining, 10);
        assert_eq!(state.per_player[0].block_pool[1].mods.len(), 1);
        assert_eq!(state.per_player[0].block_pool[1].mods[0].original, 5);
        assert!(state.per_player[0].pending_block_contribs.is_empty());
    });
}

#[test]
fn block_pool_consume_splits_fifo_proportionally() {
    reset_state();
    start_combat();
    push_chunk("DEFEND", SourceKind::Card, 10);
    push_chunk("DEFEND", SourceKind::Card, 5);
    STATE.with(|cell| {
        cell.borrow_mut()
            .slot_state_mut(0)
            .pending_block_contribs
            .push(PendingContrib {
                id: "DEX".to_owned(),
                kind: SourceKind::Power,
                player: 0,
                amount: 5,
            });
    });
    push_chunk("IRON_WAVE", SourceKind::Card, 5);

    assert_eq!(consume_chunk(12), 12);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[0].block_pool.len(), 2);
        assert_eq!(state.per_player[0].block_pool[0].remaining, 3);
        assert_eq!(state.per_player[0].block_pool[0].base_consumed, 12);
    });
    // 3 more from DEFEND, then 5 of IRON_WAVE's 10: half consumed leaves
    // base 3 / mod 2 (residue on the base), 5 remaining.
    assert_eq!(consume_chunk(8), 8);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.per_player[0].block_pool.len(), 1);
        assert_eq!(state.per_player[0].block_pool[0].id, "IRON_WAVE");
        assert_eq!(state.per_player[0].block_pool[0].remaining, 5);
        assert_eq!(state.per_player[0].block_pool[0].base_consumed, 3);
        assert_eq!(state.per_player[0].block_pool[0].mods[0].consumed, 2);
        let combat = state.current.as_ref().expect("combat exists");
        let defend = combat
            .cards
            .iter()
            .find(|card| card.id == "DEFEND")
            .expect("defend card");
        assert_eq!(defend.block_effective, 15);
        let wave = combat
            .cards
            .iter()
            .find(|card| card.id == "IRON_WAVE")
            .expect("wave card");
        assert_eq!(wave.block_effective, 3);
        let dex = combat
            .cards
            .iter()
            .find(|card| card.id == "DEX")
            .expect("dex card");
        assert_eq!(dex.blk_modifier, 2);
    });
}

#[test]
fn split_over_appliers_splits_proportionally_and_falls_back() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.power_sources.push(PowerSourceEntry {
            power_id: "STRENGTH_POWER".to_owned(),
            source_id: "INFLAME".to_owned(),
            kind: SourceKind::Card,
            player: 0,
            amount: 2,
        });
        state.power_sources.push(PowerSourceEntry {
            power_id: "STRENGTH_POWER".to_owned(),
            source_id: "VAJRA".to_owned(),
            kind: SourceKind::Relic,
            player: 0,
            amount: 1,
        });
        state.power_sources.push(PowerSourceEntry {
            power_id: "DEXTERITY_POWER".to_owned(),
            source_id: "OTHER".to_owned(),
            kind: SourceKind::Card,
            player: 0,
            amount: 9,
        });
    });
    // 10 over a 2:1 record → 6 to INFLAME, residue 4 to VAJRA.
    let shares = STATE.with(|cell| {
        split_over_appliers_in(&cell.borrow(), "STRENGTH_POWER", SourceKind::Power, 10, 0)
    });
    assert_eq!(shares.len(), 2);
    assert_eq!(shares[0].id, "INFLAME");
    assert_eq!(shares[0].kind, SourceKind::Card);
    assert_eq!(shares[0].amount, 6);
    assert_eq!(shares[1].id, "VAJRA");
    assert_eq!(shares[1].kind, SourceKind::Relic);
    assert_eq!(shares[1].amount, 4);
    // No recorded appliers: the modifier itself takes the whole amount.
    let fallback = STATE.with(|cell| {
        split_over_appliers_in(&cell.borrow(), "VULNERABLE_POWER", SourceKind::Power, 7, 0)
    });
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].id, "VULNERABLE_POWER");
    assert_eq!(fallback[0].kind, SourceKind::Power);
    assert_eq!(fallback[0].amount, 7);
}

#[test]
fn apply_pending_contribs_shifts_modifier_share() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat {
            cards: vec![hit_card("STRIKE", 10)],
            ..Combat::default()
        });
        state
            .slot_state_mut(0)
            .pending_contribs
            .push(PendingContrib {
                id: "INFLAME".to_owned(),
                kind: SourceKind::Card,
                player: 0,
                amount: 3,
            });
    });
    STATE.with(|cell| {
        let mut logs = Vec::new();
        apply_pending_contribs_in(&mut cell.borrow_mut(), 0, 0, &mut logs);
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        let strike = &combat.cards[0];
        assert_eq!(strike.damage_dealt, 7);
        assert_eq!(strike.dmg_direct, 7);
        assert_eq!(strike.damage_dealt - strike.damage_blocked, 7);
        let inflame = combat
            .cards
            .iter()
            .find(|card| card.id == "INFLAME")
            .expect("inflame card");
        assert_eq!(inflame.damage_dealt, 3);
        assert_eq!(inflame.dmg_modifier, 3);
        assert_eq!(inflame.damage_dealt - inflame.damage_blocked, 3);
        assert!(state.per_player[0].pending_contribs.is_empty());
    });
}

/// The carve takes from the segment the hit grew, so the decomposition
/// stays exact.
#[test]
fn apply_pending_contribs_carves_attributed_hits_from_dmg_attributed() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat {
            cards: vec![CardStat {
                id: "ZAP".to_owned(),
                kind: 0,
                damage_dealt: 5,
                dmg_attributed: 5,
                ..CardStat::default()
            }],
            ..Combat::default()
        });
        state
            .slot_state_mut(0)
            .pending_contribs
            .push(PendingContrib {
                id: "INFLAME".to_owned(),
                kind: SourceKind::Card,
                player: 0,
                amount: 3,
            });
    });
    STATE.with(|cell| {
        let mut logs = Vec::new();
        apply_pending_contribs_in(&mut cell.borrow_mut(), 0, 0, &mut logs);
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        let zap = &combat.cards[0];
        assert_eq!(zap.damage_dealt, 2);
        assert_eq!(zap.dmg_direct, 0);
        assert_eq!(zap.dmg_attributed, 2);
        assert_eq!(zap.damage_dealt - zap.damage_blocked, 2);
        let inflame = combat
            .cards
            .iter()
            .find(|card| card.id == "INFLAME")
            .expect("inflame card");
        assert_eq!(inflame.dmg_modifier, 3);
    });
}

/// On a mixed-history card, dmg_direct first, then dmg_attributed.
#[test]
fn apply_pending_contribs_carves_mixed_hits_direct_first() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat {
            cards: vec![CardStat {
                id: "ZAP".to_owned(),
                kind: 0,
                damage_dealt: 9,
                dmg_direct: 4,
                dmg_attributed: 5,
                ..CardStat::default()
            }],
            ..Combat::default()
        });
        state
            .slot_state_mut(0)
            .pending_contribs
            .push(PendingContrib {
                id: "INFLAME".to_owned(),
                kind: SourceKind::Card,
                player: 0,
                amount: 6,
            });
    });
    STATE.with(|cell| {
        let mut logs = Vec::new();
        apply_pending_contribs_in(&mut cell.borrow_mut(), 0, 0, &mut logs);
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        let zap = &combat.cards[0];
        assert_eq!(zap.damage_dealt, 3);
        assert_eq!(zap.dmg_direct, 0);
        assert_eq!(zap.dmg_attributed, 3);
        let inflame = combat
            .cards
            .iter()
            .find(|card| card.id == "INFLAME")
            .expect("inflame card");
        assert_eq!(inflame.dmg_modifier, 6);
    });
}

#[test]
fn apply_upgrade_split_damage_credits_upgrader_per_hit() {
    reset_state();
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.current = Some(Combat {
            cards: vec![hit_card("STRIKE", 10)],
            ..Combat::default()
        });
        let slot = state.slot_state_mut(0);
        slot.pending_upgrade_dmg = 4;
        slot.pending_upgrade_source = "ARMAMENTS".to_owned();
        slot.pending_upgrade_kind = 0;
    });
    STATE.with(|cell| {
        let mut logs = Vec::new();
        apply_upgrade_split_damage_in(&mut cell.borrow_mut(), 0, 0, &mut logs);
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        assert_eq!(combat.cards[0].damage_dealt, 6);
        assert_eq!(combat.cards[0].dmg_direct, 6);
        assert_eq!(
            combat.cards[0].damage_dealt - combat.cards[0].damage_blocked,
            6
        );
        let arm = combat
            .cards
            .iter()
            .find(|card| card.id == "ARMAMENTS")
            .expect("upgrader card");
        assert_eq!(arm.damage_dealt, 4);
        assert_eq!(arm.dmg_upgrade, 4);
        assert_eq!(arm.damage_dealt - arm.damage_blocked, 4);
    });
    // The pending bonus is not consumed; a second hit splits it again.
    STATE.with(|cell| {
        let mut logs = Vec::new();
        apply_upgrade_split_damage_in(&mut cell.borrow_mut(), 0, 0, &mut logs);
    });
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state.current.as_ref().expect("combat exists");
        assert_eq!(combat.cards[0].dmg_direct, 2);
        let arm = combat
            .cards
            .iter()
            .find(|card| card.id == "ARMAMENTS")
            .expect("upgrader card");
        assert_eq!(arm.dmg_upgrade, 8);
    });
}

#[test]
fn find_upgrade_locates_delta_record() {
    reset_state();
    STATE.with(|cell| {
        cell.borrow_mut().upgrade_deltas.push(UpgradeDelta {
            hash: 42,
            damage: 3,
            block: 2,
            source_id: "ARMAMENTS".to_owned(),
            kind: SourceKind::Card,
            player: 0,
        });
    });
    let delta = STATE
        .with(|cell| find_upgrade_in(&cell.borrow(), 42))
        .expect("record found");
    assert_eq!(delta.damage, 3);
    assert_eq!(delta.block, 2);
    assert_eq!(delta.source_id, "ARMAMENTS");
    assert_eq!(delta.kind, SourceKind::Card);
    assert!(
        STATE
            .with(|cell| find_upgrade_in(&cell.borrow(), 1))
            .is_none()
    );
}

#[test]
fn resolve_card_is_none_without_a_combat() {
    reset_state();
    assert!(resolve("STRIKE", 0, 0).is_none());
    assert!(STATE.with(|cell| cell.borrow().current.is_none()));
}
