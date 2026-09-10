//! Native fixtures supply event packets in pinned-game order. They verify ledger
//! behavior, not hook installation or execution of game methods.

use super::*;
use crate::data::state::CardStat;
use crate::source_kind::SourceKind;

const DIRECT: i32 = 0;
const ATTRIBUTED: i32 = 1;
const OUTGOING: i32 = 0;
const INCOMING: i32 = 1;
const SELF_DAMAGE: i32 = 2;
const OSTY_DEALT: i32 = 3;
const OSTY_ABSORBED: i32 = 4;
const PLAYER_A: (u64, i32, i32) = (1000, 0, 0);
const PLAYER_B: (u64, i32, i32) = (1001, 0, 1);
const ENEMY: (u64, i32, i32) = (900, 1, 4);

struct Scenario {
    state: State,
}

impl Scenario {
    fn new() -> Self {
        Self {
            state: State {
                current: Some(Combat {
                    seq: 7,
                    ..Combat::default()
                })
                .into(),
                ..State::default()
            },
        }
    }

    fn card(&mut self, instance: u64, id: &str, slot: i32) -> u64 {
        let source = self.state.source_capture(7, 1, instance, id, 0, slot, 0);
        assert_ne!(source, 0, "ordinary fixture cards have explicit identity");
        source
    }

    fn model(&mut self, id: &str, kind: i32, slot: i32) -> u64 {
        let source = self.state.source_capture(7, 4, 0, id, kind, slot, 0);
        assert_ne!(source, 0, "named fixture relics and potions are capturable");
        source
    }

    fn capture(&mut self, kind: i32, instance: u64) -> u64 {
        let source = self.state.source_capture(7, kind, instance, "", 2, 4, 0);
        assert_ne!(
            source, 0,
            "fixture capture has a valid epoch and lease capacity"
        );
        source
    }

    fn mix(&mut self, roots: &[(u64, u64)]) -> u64 {
        let transfer = self.state.source_transfer_begin(7);
        assert_ne!(transfer, 0);
        for (source, amount) in roots {
            assert_eq!(
                self.state.source_count(*source),
                1,
                "fixture weights name individual roots"
            );
            let destination = self.state.source_destination(*source, 0);
            let weight = self
                .state
                .source_weight(*source, 0)
                .checked_mul(*amount)
                .expect("small fixture weights fit u64");
            assert_eq!(
                self.state
                    .source_transfer_add(transfer, destination, weight),
                1
            );
        }
        assert_eq!(self.state.source_transfer_seal(transfer), 1);
        transfer
    }

    fn attach(
        &mut self,
        instance: u64,
        id: &str,
        owner: (u64, i32, i32),
        amount: i32,
        source: u64,
    ) {
        assert_eq!(
            self.state
                .power_attached(7, instance, id, owner.0, owner.1, owner.2, amount, source),
            1
        );
    }

    fn change(
        &mut self,
        instance: u64,
        id: &str,
        owner: (u64, i32, i32),
        amounts: (i32, i32),
        source: u64,
    ) {
        assert_eq!(
            self.state.power_amount_changed(
                7, instance, id, owner.0, owner.1, owner.2, amounts.0, amounts.1, source
            ),
            1
        );
    }

    fn play(
        &mut self,
        execution: u64,
        instance: u64,
        id: &str,
        slot: i32,
        generation: i32,
        source: u64,
    ) -> u64 {
        let play = self
            .state
            .card_play_started(7, execution, instance, id, slot, 0, 1, generation, source);
        assert_ne!(
            play, 0,
            "fixture plays have available bounded execution frames"
        );
        play
    }

    fn report(
        &mut self,
        source: u64,
        segment: i32,
        kind: i32,
        receiver: i32,
        total: i32,
        blocked: i32,
    ) {
        let role = if segment == DIRECT { 1 } else { 2 };
        let calculation = self
            .state
            .damage_calculation_begin(7, source, role, segment, 900);
        assert_ne!(calculation, 0);
        assert_eq!(
            self.state.damage_result_append(
                calculation,
                total,
                total - blocked,
                blocked,
                kind,
                receiver,
                0
            ),
            1
        );
        assert_eq!(self.state.damage_calculation_commit(calculation), 1);
    }

    fn combat(&self) -> &Combat {
        self.state
            .current
            .as_ref()
            .expect("scenario owns an active combat")
    }

    fn outgoing(
        &mut self,
        source: u64,
        role: i32,
        segment: i32,
        target: u64,
        hp: i32,
        blocked: i32,
    ) {
        let calculation = self
            .state
            .damage_calculation_begin(7, source, role, segment, target);
        assert_ne!(calculation, 0);
        assert_eq!(
            self.state.damage_result_append(
                calculation,
                hp + blocked,
                hp,
                blocked,
                OUTGOING,
                TEAM_SLOT.into(),
                0
            ),
            1
        );
        assert_eq!(self.state.damage_calculation_commit(calculation), 1);
    }

    fn assert_damage(&self, expected: &[(&str, SourceSlot, [i64; 3], i64)], hp: i64, blocked: i64) {
        let mut actual = Vec::new();
        let mut total_damage = 0;
        let mut total_blocked = 0;
        for row in &self.combat().cards {
            let segments = [row.dmg_direct, row.dmg_attributed, row.dmg_modifier];
            assert_eq!(row.damage_dealt, segments.iter().sum::<i64>());
            assert!((0..=row.damage_dealt).contains(&row.damage_blocked));
            total_damage += row.damage_dealt;
            total_blocked += row.damage_blocked;
            if row.damage_dealt != 0 {
                actual.push((row.id.as_str(), row.player, segments, row.damage_blocked));
            }
        }
        actual.sort_unstable();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(actual, expected);
        assert_eq!((total_damage, total_blocked), (hp + blocked, blocked));
    }

    fn row(&self, id: &str, slot: SourceSlot) -> &CardStat {
        self.combat()
            .cards
            .iter()
            .find(|row| row.id == id && row.player == slot)
            .expect("scenario effects create the expected credited row")
    }

    fn assert_source(&mut self, transfer: u64, expected: &[(&str, SourceSlot, u64)]) {
        let snapshot = self
            .state
            .source_snapshot(7, transfer)
            .expect("fixture transfer belongs to this combat");
        let actual: Vec<_> = snapshot
            .shares()
            .iter()
            .map(|share| {
                let (name, slot) = match share.destination() {
                    Destination::Row(index) => {
                        let row = &self.combat().cards[index];
                        (row.id.as_str(), row.player)
                    }
                    Destination::Unknown(slot) => ("UNATTRIBUTED", slot),
                };
                (name, slot, share.weight())
            })
            .collect();
        assert_eq!(actual.as_slice(), expected);
    }
}

#[test]
fn white_noise_storm_orb_retains_supplier_before_later_stacks() {
    let mut case = Scenario::new();
    let white_noise = case.card(10, "WHITE_NOISE", 0);
    assert_eq!(case.state.card_generated(7, 101, white_noise, 1), 1);
    let generated_storm = case.state.source_capture(7, 1, 101, "STORM", 0, 0, 1);
    case.attach(201, "STORM_POWER", PLAYER_A, 1, generated_storm);
    let saved_storm = case.capture(2, 201);
    let later_storm = case.card(102, "STORM", 0);
    case.change(201, "STORM_POWER", PLAYER_A, (1, 2), later_storm);
    let current_storm = case.capture(2, 201);
    case.assert_source(current_storm, &[("WHITE_NOISE", 0, 1), ("STORM", 0, 1)]);
    assert_eq!(case.state.orb_channeled(7, 301, saved_storm), 1);
    let orb = case.capture(3, 301);
    case.assert_source(orb, &[("WHITE_NOISE", 0, 1)]);
    case.report(orb, ATTRIBUTED, OUTGOING, 4, 7, 2);
    let row = case.row("WHITE_NOISE", 0);
    assert_eq!(
        (row.damage_dealt, row.dmg_attributed, row.damage_blocked),
        (7, 7, 2)
    );
    assert_eq!(case.row("STORM", 0).damage_dealt, 0);
}

#[test]
fn ally_soulbound_souls_keep_lineage_through_draws_retention_and_transfer() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "SOULBOUND", 0);
    let supplier_play = case.play(501, 10, "SOULBOUND", 0, 0, supplier);
    case.attach(201, "SOULBOUND_POWER", PLAYER_B, 1, supplier);
    assert_eq!(case.state.card_play_finished(supplier_play), 1);
    let soulbound = case.capture(2, 201);
    case.assert_source(soulbound, &[("SOULBOUND", 0, 1)]);
    assert_eq!(case.state.card_generated(7, 101, soulbound, 2), 1);
    assert_eq!(case.state.card_generated(7, 102, soulbound, 2), 1);
    let cacophony = case.card(11, "CACOPHONY", 0);
    let speedster = case.card(12, "SPEEDSTER", 1);
    let haunt = case.card(13, "HAUNT", 1);
    case.attach(202, "CACOPHONY_POWER", PLAYER_A, 66, cacophony);
    case.attach(203, "SPEEDSTER_POWER", PLAYER_B, 2, speedster);
    case.attach(204, "HAUNT_POWER", PLAYER_B, 7, haunt);
    let cacophony = case.capture(2, 202);
    let speedster = case.capture(2, 203);
    let haunt = case.capture(2, 204);
    let soul = case.state.source_capture(7, 1, 101, "SOUL", 0, 1, 1);
    let play = case.play(502, 101, "SOUL", 1, 1, soul);
    // The first draw reaches Cacophony's threshold and kills its chosen enemy.
    case.outgoing(cacophony, 2, ATTRIBUTED, 901, 62, 4);
    case.outgoing(speedster, 2, ATTRIBUTED, 900, 1, 1);
    case.outgoing(speedster, 2, ATTRIBUTED, 900, 2, 0);
    assert_eq!(case.state.card_play_finished(play), 1);
    case.outgoing(haunt, 2, ATTRIBUTED, 900, 7, 0);
    let retained = case.state.source_capture(7, 1, 102, "SOUL", 0, 1, 1);
    case.assert_source(retained, &[("SOULBOUND", 0, 1)]);
    assert_eq!(case.state.turn_started(7), 1);
    let transferred = case.state.source_capture(7, 1, 102, "SOUL", 0, 2, 1);
    case.assert_source(transferred, &[("SOULBOUND", 0, 1)]);
    let transferred_play = case.play(503, 102, "SOUL", 2, 1, transferred);
    assert_eq!(case.state.card_play_finished(transferred_play), 1);
    case.assert_damage(
        &[
            ("CACOPHONY", 0, [0, 66, 0], 4),
            ("SPEEDSTER", 1, [0, 4, 0], 1),
            ("HAUNT", 1, [0, 7, 0], 0),
        ],
        72,
        5,
    );
    assert_eq!(case.row("SOULBOUND", 0).plays, 1);
    assert_eq!(case.combat().plays, 3);
    assert_eq!(case.combat().generated_plays, 2);
    assert_eq!(case.combat().generation_triggers, 0);
    assert_eq!(
        case.combat().cards.iter().map(|row| row.plays).sum::<u32>(),
        1
    );
    assert!(case.combat().cards.iter().all(|row| row.id != "SOUL"));
}

#[test]
fn two_players_demon_form_strength_instances_keep_distinct_suppliers() {
    let mut case = Scenario::new();
    let first = case.card(10, "DEMON_FORM", 0);
    let second = case.card(11, "DEMON_FORM", 1);
    case.attach(201, "DEMON_FORM_POWER", PLAYER_A, 1, first);
    case.attach(202, "DEMON_FORM_POWER", PLAYER_B, 1, second);
    let first_form = case.capture(2, 201);
    let second_form = case.capture(2, 202);
    case.attach(301, "STRENGTH_POWER", PLAYER_A, 3, first_form);
    case.attach(302, "STRENGTH_POWER", PLAYER_B, 5, second_form);
    let first_strength = case.capture(2, 301);
    let second_strength = case.capture(2, 302);
    case.assert_source(first_strength, &[("DEMON_FORM", 0, 1)]);
    case.assert_source(second_strength, &[("DEMON_FORM", 1, 1)]);
    let strike = case.card(12, "STRIKE", 1);
    let calculation = case
        .state
        .damage_calculation_begin(7, strike, 1, DIRECT, 900);
    assert_ne!(calculation, 0);
    assert_eq!(
        case.state
            .damage_modifier_contribution(calculation, first_strength, 3),
        1
    );
    assert_eq!(
        case.state
            .damage_modifier_contribution(calculation, second_strength, 5),
        1
    );
    assert_eq!(
        case.state
            .damage_result_append(calculation, 10, 6, 4, OUTGOING, 4, 0),
        1
    );
    assert_eq!(case.state.damage_calculation_commit(calculation), 1);
    assert_eq!(
        (
            case.row("DEMON_FORM", 0).dmg_modifier,
            case.row("DEMON_FORM", 0).damage_blocked
        ),
        (3, 1)
    );
    assert_eq!(
        (
            case.row("DEMON_FORM", 1).dmg_modifier,
            case.row("DEMON_FORM", 1).damage_blocked
        ),
        (5, 2)
    );
    assert_eq!(
        (
            case.row("STRIKE", 1).dmg_direct,
            case.row("STRIKE", 1).damage_blocked
        ),
        (2, 1)
    );
}

#[test]
fn mixed_weak_grants_consume_fifo_without_rounding_one_unit_sources() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST", 0);
    let second = case.card(11, "SECOND", 1);
    let later = case.card(12, "LATER", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    case.attach(201, "WEAK_POWER", ENEMY, 1, mixed);
    case.change(201, "WEAK_POWER", ENEMY, (1, 2), later);
    let all = case.capture(2, 201);
    case.assert_source(all, &[("FIRST", 0, 1), ("SECOND", 1, 1), ("LATER", 2, 2)]);
    let head = case.capture(5, 201);
    case.assert_source(head, &[("FIRST", 0, 1), ("SECOND", 1, 1)]);
    case.change(201, "WEAK_POWER", ENEMY, (2, 1), 0);
    let remaining = case.capture(2, 201);
    case.assert_source(remaining, &[("LATER", 2, 1)]);
    case.change(201, "WEAK_POWER", ENEMY, (1, 0), 0);
    let drained = case.capture(2, 201);
    case.assert_source(drained, &[("LATER", 2, 1)]);
    assert_eq!(case.state.power_removed(7, 201), 1);
    let removed = case.capture(2, 201);
    case.assert_source(removed, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
}

#[test]
fn dirty_power_recovery_credits_only_evidenced_new_grants() {
    let mut case = Scenario::new();
    let former = case.card(10, "FORMER", 0);
    let incoming = case.card(11, "INCOMING", 1);
    case.attach(201, "STRENGTH_POWER", PLAYER_A, 3, former);
    assert_eq!(case.state.power_provenance_invalidate(7, 201), 1);
    let dirty = case.capture(2, 201);
    case.assert_source(dirty, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    case.change(201, "STRENGTH_POWER", PLAYER_A, (3, 5), incoming);
    let recovered = case.capture(2, 201);
    case.assert_source(
        recovered,
        &[("UNATTRIBUTED", TEAM_SLOT, 3), ("INCOMING", 1, 2)],
    );
    assert_eq!(case.state.power_provenance_invalidate(7, 201), 1);
    case.change(201, "STRENGTH_POWER", PLAYER_A, (5, 5), former);
    let unchanged = case.capture(2, 201);
    case.assert_source(unchanged, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    case.change(201, "STRENGTH_POWER", PLAYER_A, (9, 10), incoming);
    let mismatched = case.capture(2, 201);
    case.assert_source(
        mismatched,
        &[("UNATTRIBUTED", TEAM_SLOT, 9), ("INCOMING", 1, 1)],
    );
}

#[test]
fn misery_clone_attachment_credits_its_new_supplier() {
    let mut case = Scenario::new();
    let original = case.card(10, "ORIGINAL", 0);
    let misery = case.card(11, "MISERY", 1);
    case.attach(201, "STRENGTH_POWER", PLAYER_A, 3, original);
    case.attach(202, "STRENGTH_POWER", PLAYER_B, 3, misery);
    let clone = case.capture(2, 202);
    let old = case.capture(2, 201);
    case.assert_source(clone, &[("MISERY", 1, 1)]);
    case.assert_source(old, &[("ORIGINAL", 0, 1)]);
    case.change(202, "STRENGTH_POWER", PLAYER_B, (3, 3), original);
    let same_amount = case.capture(2, 202);
    case.assert_source(same_amount, &[("MISERY", 1, 1)]);
}

#[test]
fn generated_unavailable_overrides_prior_native_ancestry() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_GENERATOR", 0);
    let second = case.card(11, "SECOND_GENERATOR", 1);
    assert_eq!(case.state.card_generated(7, 101, first, 1), 1);
    let recorded = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 1);
    case.assert_source(recorded, &[("FIRST_GENERATOR", 0, 1)]);
    let unavailable = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 2);
    case.assert_source(unavailable, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    let stale_recorded = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 1);
    case.assert_source(stale_recorded, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    assert_eq!(case.state.card_generated(7, 101, second, 1), 1);
    let replaced = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 1);
    case.assert_source(replaced, &[("SECOND_GENERATOR", 1, 1)]);
    let inconsistent = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 0);
    case.assert_source(inconsistent, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    assert!(case.combat().cards.iter().all(|row| row.id != "GENERATED"));
}

#[test]
fn failed_regeneration_cannot_revive_released_source() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_GENERATOR", 0);
    let old = case.card(11, "OLD_GENERATOR", 1);
    assert_eq!(case.state.card_generated(7, 101, old, 1), 1);
    assert_eq!(case.state.source_transfer_release(first), 1);
    assert_eq!(case.state.card_generated(7, 101, first, 1), 0);
    let generated = case.state.source_capture(7, 1, 101, "GENERATED", 0, 2, 1);
    case.assert_source(generated, &[("UNATTRIBUTED", TEAM_SLOT, 1)]);
    assert_eq!(case.combat().generation_triggers, 0);
}

#[test]
fn generation_counts_distinct_noncard_participants_and_actual_plays() {
    let mut case = Scenario::new();
    let relic = case.model("GENERATOR_RELIC", 1, 0);
    let potion = case.model("GENERATOR_POTION", 3, 1);
    let card = case.card(10, "WHITE_NOISE", 2);
    let mixture = case.mix(&[(relic, 2), (relic, 3), (potion, 1), (card, 4)]);
    assert_eq!(case.state.card_generated(7, 101, mixture, 2), 1);
    assert_eq!(case.combat().generation_triggers, 2);
    assert_eq!(case.row("GENERATOR_RELIC", 0).plays, 1);
    assert_eq!(case.row("GENERATOR_POTION", 1).plays, 1);
    assert_eq!(case.row("WHITE_NOISE", 2).plays, 0);
    let generated = case.state.source_capture(7, 1, 101, "GENERATED", 0, 3, 1);
    let plays = [
        case.play(501, 10, "WHITE_NOISE", 2, 0, card),
        case.play(502, 101, "GENERATED", 3, 1, generated),
        case.play(503, 102, "UNAVAILABLE", 3, 2, 0),
        case.play(504, 103, "UNCLASSIFIED", 3, 3, 0),
    ];
    for play in plays {
        assert_eq!(case.state.card_play_finished(play), 1);
    }
    let combat = case.combat();
    assert_eq!(
        (
            combat.plays,
            combat.generated_plays,
            combat.generation_triggers
        ),
        (4, 2, 2)
    );
    assert_eq!(case.row("WHITE_NOISE", 2).plays, 1);
    assert_eq!(case.row("UNATTRIBUTED", TEAM_SLOT).plays, 1);
    assert_eq!(
        combat.cards.iter().map(|row| row.plays).sum::<u32>() + combat.generated_plays,
        combat.plays + combat.generation_triggers
    );
}

#[test]
fn nested_same_owner_execution_cleanup_preserves_outer_orb_flags() {
    let mut case = Scenario::new();
    let outer_source = case.card(10, "OUTER", 0);
    let inner_source = case.card(11, "INNER", 0);
    assert_eq!(case.state.orb_channeled(7, 301, outer_source), 1);
    let outer = case.play(501, 10, "OUTER", 0, 0, outer_source);
    assert_eq!(case.state.orb_context_begin(7, 301, outer, 0), 1);
    let inner = case.play(502, 11, "INNER", 0, 0, inner_source);
    assert_eq!(case.state.orb_context_begin(7, 301, inner, 0), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, inner, 0), 2);
    assert_eq!(case.state.card_execution_ended(7, 502), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, inner, 0), 0);
    assert_eq!(case.state.card_execution_ended(7, 503), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, outer, 0), 2);
    assert_eq!(case.state.card_play_finished(outer), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, 0, 0), 1);
    assert_eq!(
        (case.row("OUTER", 0).plays, case.row("INNER", 0).plays),
        (1, 1)
    );
}

#[test]
fn missing_orb_capture_advances_only_its_matching_owner_play() {
    let mut case = Scenario::new();
    let source = case.card(10, "HIBERNATE", 1);
    let play = case.play(501, 10, "HIBERNATE", 1, 0, source);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 0), 0);
    assert_eq!(case.state.orb_context_begin(7, 301, source, 1), 0);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 0);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 2);
    assert_eq!(case.state.orb_channeled(7, 301, source), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 2);
    assert_eq!(case.state.card_execution_ended(7, 501), 1);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 0);
}

#[test]
fn stale_cleanup_cannot_close_replacement_combat_lifetimes() {
    let mut case = Scenario::new();
    let old_source = case.card(10, "OLD", 0);
    let old_play = case.play(501, 10, "OLD", 0, 0, old_source);
    case.attach(201, "STORM_POWER", PLAYER_A, 1, old_source);
    let old_calculation = case
        .state
        .damage_calculation_begin(7, old_source, 1, DIRECT, 900);
    assert_ne!(old_calculation, 0);
    case.state.current = Some(Combat {
        seq: 8,
        ..Combat::default()
    })
    .into();
    let new_source = case.state.source_capture(8, 1, 11, "NEW", 0, 0, 0);
    let new_play = case
        .state
        .card_play_started(8, 502, 11, "NEW", 0, 0, 1, 0, new_source);
    assert_ne!(new_play, 0);
    assert_eq!(case.state.orb_channeled(8, 302, new_source), 1);
    assert_eq!(case.state.card_execution_ended(7, 502), 0);
    assert_eq!(case.state.card_play_finished(old_play), 0);
    assert_eq!(case.state.power_removed(7, 201), 0);
    assert_eq!(case.state.source_transfer_release(old_source), 0);
    assert_eq!(case.state.damage_calculation_abort(old_calculation), 0);
    assert_eq!(case.state.orb_context_begin(8, 302, new_play, 0), 1);
    assert_eq!(case.state.orb_context_begin(8, 302, new_play, 0), 2);
    assert_eq!(case.row("NEW", 0).plays, 1);
    assert!(case.combat().cards.iter().all(|row| row.id != "OLD"));
}

#[test]
fn ally_block_preserves_mixed_suppliers_through_small_hits() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_SHIELD", 0);
    let second = case.card(11, "SECOND_SHIELD", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    assert_eq!(case.state.block_gained(7, 2, mixed, 1), 1);
    case.report(0, ATTRIBUTED, INCOMING, 0, 1, 1);
    assert_eq!(case.row("FIRST_SHIELD", 0).block_effective, 0);
    assert_eq!(case.row("SECOND_SHIELD", 2).block_effective, 0);
    for _ in 0..2 {
        case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    }
    assert_eq!(
        (
            case.row("FIRST_SHIELD", 0).block_gained,
            case.row("FIRST_SHIELD", 0).block_effective
        ),
        (1, 1)
    );
    assert_eq!(
        (
            case.row("SECOND_SHIELD", 2).block_gained,
            case.row("SECOND_SHIELD", 2).block_effective
        ),
        (1, 1)
    );
    assert_eq!(case.combat().block_total, 2);
    assert_eq!(case.combat().damage_received, 3);
}

#[test]
fn block_modifier_mixture_keeps_both_roots_across_consumption() {
    let mut case = Scenario::new();
    let producer = case.card(10, "GUARD", 1);
    let first = case.card(11, "FIRST_DEXTERITY", 0);
    let second = case.card(12, "SECOND_DEXTERITY", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    assert_eq!(case.state.block_modifier_contribution(7, mixed, 2, 1), 1);
    assert_eq!(case.state.block_gained(7, 4, producer, 1), 1);
    for _ in 0..2 {
        case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    }
    assert_eq!(case.row("FIRST_DEXTERITY", 0).blk_modifier, 0);
    assert_eq!(case.row("SECOND_DEXTERITY", 2).blk_modifier, 1);
    for _ in 0..2 {
        case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    }
    assert_eq!(case.row("FIRST_DEXTERITY", 0).blk_modifier, 1);
    assert_eq!(case.row("SECOND_DEXTERITY", 2).blk_modifier, 1);
    assert_eq!(
        (
            case.row("GUARD", 1).block_gained,
            case.row("GUARD", 1).block_effective
        ),
        (4, 2)
    );
}

#[test]
fn osty_lifo_mixtures_keep_physical_owner_and_creditor_separate() {
    let mut case = Scenario::new();
    let oldest = case.card(10, "OLDER_SUMMON", 0);
    let first = case.card(11, "FIRST_SUMMON", 0);
    let second = case.card(12, "SECOND_SUMMON", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    assert_eq!(case.state.osty_summoned(7, oldest, 2, 1), 1);
    assert_eq!(case.state.osty_summoned(7, mixed, 2, 1), 1);
    case.report(0, ATTRIBUTED, OSTY_ABSORBED, 0, 1, 0);
    assert_eq!(case.row("OSTY", TEAM_SLOT).block_effective, 1);
    for _ in 0..2 {
        case.report(0, ATTRIBUTED, OSTY_ABSORBED, 1, 1, 0);
    }
    assert_eq!(case.row("FIRST_SUMMON", 0).block_effective, 1);
    assert_eq!(case.row("SECOND_SUMMON", 2).block_effective, 1);
    assert_eq!(case.row("OLDER_SUMMON", 0).block_effective, 0);
    case.report(0, ATTRIBUTED, OSTY_ABSORBED, 1, 2, 0);
    assert_eq!(case.row("OLDER_SUMMON", 0).block_effective, 2);
    assert_eq!(case.row("OSTY", TEAM_SLOT).block_effective, 1);
}

#[test]
fn osty_kill_requires_matching_play_owner_and_uses_full_source() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_SUMMON", 0);
    let second = case.card(11, "SECOND_SUMMON", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    assert_eq!(case.state.osty_summoned(7, mixed, 4, 1), 1);
    let wrong_owner = case.play(501, 10, "FIRST_SUMMON", 0, 0, first);
    assert_eq!(case.state.osty_killed(7, 1, wrong_owner), 0);
    assert_eq!(case.row("FIRST_SUMMON", 0).block_effective, 0);
    assert_eq!(case.state.card_generated(7, 101, mixed, 1), 1);
    let matching = case.play(502, 101, "GENERATED_SUMMON", 1, 1, mixed);
    assert_eq!(case.state.osty_killed(7, 1, matching), 1);
    assert_eq!(case.row("FIRST_SUMMON", 0).block_effective, -2);
    assert_eq!(case.row("SECOND_SUMMON", 2).block_effective, -2);
    assert_eq!(case.state.osty_killed(7, 1, matching), 1);
    assert_eq!(case.row("FIRST_SUMMON", 0).block_effective, -2);
}

#[test]
fn forge_generated_supplier_survives_another_players_active_play() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "SUPPLIER", 0);
    let trigger = case.card(11, "TRIGGER", 2);
    assert_eq!(case.state.card_generated(7, 101, supplier, 1), 1);
    let generated = case
        .state
        .source_capture(7, 1, 101, "SOVEREIGN_BLADE", 0, 2, 1);
    let play = case.play(501, 11, "TRIGGER", 2, 0, trigger);
    assert_eq!(case.state.forge(7, generated, 5), 1);
    assert_eq!(case.row("SUPPLIER", 0).forge, 5);
    assert_eq!(case.row("TRIGGER", 2).forge, 0);
    assert_eq!(case.state.card_play_finished(play), 1);
    assert!(
        case.combat()
            .cards
            .iter()
            .all(|row| row.id != "SOVEREIGN_BLADE")
    );
}

#[test]
fn frozen_strength_and_weak_credit_survives_later_cancellation() {
    let mut case = Scenario::new();
    let disarm = case.card(10, "DISARM", 0);
    let first_weak = case.card(11, "FIRST_WEAK", 1);
    let later_weak = case.card(12, "LATER_WEAK", 2);
    case.attach(201, "STRENGTH_POWER", ENEMY, -4, disarm);
    case.attach(202, "WEAK_POWER", ENEMY, 2, first_weak);
    case.change(202, "WEAK_POWER", ENEMY, (2, 4), later_weak);
    let weak = case.capture(5, 202);
    let calculation = case
        .state
        .damage_calculation_begin(7, 0, 2, ATTRIBUTED, 1000);
    assert_ne!(calculation, 0);
    assert_eq!(
        case.state
            .damage_calculation_enemy_hit(calculation, 900, 10, -4),
        1
    );
    assert_eq!(
        case.state.damage_calculation_weak_source(calculation, weak),
        1
    );
    case.change(201, "STRENGTH_POWER", ENEMY, (-4, 0), 0);
    case.change(202, "WEAK_POWER", ENEMY, (4, 2), 0);
    for _ in 0..2 {
        assert_eq!(
            case.state
                .damage_result_append(calculation, 3, 3, 0, INCOMING, 0, 1),
            1
        );
    }
    assert_eq!(case.state.damage_calculation_commit(calculation), 1);
    assert_eq!(case.row("DISARM", 0).mitigate_str, 4);
    assert_eq!(case.row("FIRST_WEAK", 1).mitigate_debuff, 2);
    assert_eq!(case.row("LATER_WEAK", 2).mitigate_debuff, 0);
    assert_eq!(case.combat().damage_received, 6);
    let live_weak = case.capture(5, 202);
    case.assert_source(live_weak, &[("LATER_WEAK", 2, 1)]);
    case.report(0, ATTRIBUTED, INCOMING, 0, 3, 0);
    assert_eq!(case.row("DISARM", 0).mitigate_str, 4);
}

#[test]
fn doom_batch_retains_removed_suppliers_and_explicit_uncovered_hp() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_DOOM", 0);
    let second = case.card(11, "SECOND_DOOM", 1);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    case.attach(201, "DOOM_POWER", ENEMY, 3, mixed);
    let batch = case.state.doom_batch_begin(7);
    assert_ne!(batch, 0);
    assert_eq!(case.state.doom_target_capture(batch, 900, 201, 5), 1);
    assert_eq!(case.state.power_removed(7, 201), 1);
    assert_eq!(case.state.doom_kills_completed(batch), 1);
    assert_eq!(case.row("FIRST_DOOM", 0).dmg_attributed, 1);
    assert_eq!(case.row("SECOND_DOOM", 1).dmg_attributed, 2);
    assert_eq!(case.row("UNATTRIBUTED", TEAM_SLOT).dmg_attributed, 2);
    assert_eq!(case.state.doom_kills_completed(batch), 0);
    assert_eq!(
        case.combat()
            .cards
            .iter()
            .map(|row| row.damage_dealt)
            .sum::<i64>(),
        5
    );
}

#[test]
fn unavailable_play_identity_counts_once_and_preserves_outer_frame() {
    let mut case = Scenario::new();
    let source = case.card(10, "OUTER", 0);
    assert_eq!(case.state.orb_channeled(7, 301, source), 1);
    let outer = case.play(501, 10, "OUTER", 0, 0, source);
    assert_eq!(case.state.orb_context_begin(7, 301, outer, 0), 1);
    assert_eq!(
        case.state
            .card_play_started(7, 0, 0, "UNCLASSIFIED", 0, 0, 1, 3, 0),
        0
    );
    assert_eq!((case.combat().plays, case.combat().generated_plays), (2, 0));
    assert_eq!(case.row("OUTER", 0).plays, 1);
    assert_eq!(case.row("UNATTRIBUTED", TEAM_SLOT).plays, 1);
    assert_eq!(case.state.card_execution_ended(7, 0), 0);
    assert_eq!(case.state.orb_context_begin(7, 301, outer, 0), 2);
    assert_eq!(case.state.card_play_finished(outer), 1);
}

#[test]
fn defy_block_and_weak_trigger_juggernaut_and_sleight_suppliers() {
    let mut case = Scenario::new();
    let sleight = case.card(10, "SLEIGHT_OF_FLESH", 0);
    let juggernaut = case.card(11, "JUGGERNAUT", 0);
    let defy = case.card(12, "DEFY", 0);
    case.attach(201, "SLEIGHT_OF_FLESH_POWER", PLAYER_A, 9, sleight);
    case.attach(202, "JUGGERNAUT_POWER", PLAYER_A, 6, juggernaut);
    let play = case.play(501, 12, "DEFY", 0, 0, defy);
    assert_eq!(case.state.block_gained(7, 6, defy, 0), 1);
    let block_proc = case.capture(2, 202);
    case.outgoing(block_proc, 2, ATTRIBUTED, 900, 4, 2);
    case.attach(203, "WEAK_POWER", (901, 1, 4), 1, defy);
    let weak = case.capture(2, 203);
    case.assert_source(weak, &[("DEFY", 0, 1)]);
    let debuff_proc = case.capture(2, 201);
    case.outgoing(debuff_proc, 2, ATTRIBUTED, 901, 6, 3);
    case.assert_damage(
        &[
            ("JUGGERNAUT", 0, [0, 6, 0], 2),
            ("SLEIGHT_OF_FLESH", 0, [0, 9, 0], 3),
        ],
        10,
        5,
    );
    assert_eq!(case.row("DEFY", 0).block_gained, 6);
    assert_eq!(case.row("DEFY", 0).plays, 1);
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn upgraded_generated_sleight_stacks_split_both_deathbringer_procs() {
    let mut case = Scenario::new();
    let ordinary = case.card(10, "SLEIGHT_OF_FLESH", 0);
    let discovery = case.card(11, "DISCOVERY", 0);
    let deathbringer = case.card(12, "DEATHBRINGER", 0);
    assert_eq!(case.state.card_generated(7, 101, discovery, 1), 1);
    let upgraded = case
        .state
        .source_capture(7, 1, 101, "SLEIGHT_OF_FLESH", 0, 0, 1);
    case.attach(201, "SLEIGHT_OF_FLESH_POWER", PLAYER_A, 9, ordinary);
    case.change(201, "SLEIGHT_OF_FLESH_POWER", PLAYER_A, (9, 22), upgraded);
    let play = case.play(501, 12, "DEATHBRINGER", 0, 0, deathbringer);
    case.attach(202, "DOOM_POWER", ENEMY, 21, deathbringer);
    let first_proc = case.capture(2, 201);
    case.assert_source(
        first_proc,
        &[("SLEIGHT_OF_FLESH", 0, 9), ("DISCOVERY", 0, 13)],
    );
    case.outgoing(first_proc, 2, ATTRIBUTED, 900, 10, 12);
    case.attach(203, "WEAK_POWER", ENEMY, 1, deathbringer);
    let second_proc = case.capture(2, 201);
    case.outgoing(second_proc, 2, ATTRIBUTED, 900, 22, 0);
    case.assert_damage(
        &[
            ("SLEIGHT_OF_FLESH", 0, [0, 18, 0], 4),
            ("DISCOVERY", 0, [0, 26, 0], 8),
        ],
        32,
        12,
    );
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn sleight_ignores_potion_relic_and_nested_envenom_trigger_suppliers() {
    let mut case = Scenario::new();
    let sleight = case.card(10, "SLEIGHT_OF_FLESH", 0);
    case.attach(201, "SLEIGHT_OF_FLESH_POWER", PLAYER_A, 9, sleight);
    let potion = case.model("WEAK_POTION", 3, 0);
    case.attach(202, "WEAK_POWER", ENEMY, 3, potion);
    let potion_proc = case.capture(2, 201);
    case.outgoing(potion_proc, 2, ATTRIBUTED, 900, 5, 4);
    let relic = case.model("BAG_OF_MARBLES", 1, 0);
    case.attach(203, "VULNERABLE_POWER", ENEMY, 1, relic);
    let relic_proc = case.capture(2, 201);
    case.outgoing(relic_proc, 2, ATTRIBUTED, 900, 9, 0);
    let envenom = case.card(11, "ENVENOM", 0);
    case.attach(204, "ENVENOM_POWER", PLAYER_A, 1, envenom);
    let strike = case.card(12, "STRIKE", 0);
    let play = case.play(501, 12, "STRIKE", 0, 0, strike);
    case.outgoing(strike, 1, DIRECT, 901, 3, 3);
    let poison_supplier = case.capture(2, 204);
    case.attach(205, "POISON_POWER", (901, 1, 4), 1, poison_supplier);
    let poison = case.capture(2, 205);
    case.assert_source(poison, &[("ENVENOM", 0, 1)]);
    let nested_proc = case.capture(2, 201);
    case.outgoing(nested_proc, 2, ATTRIBUTED, 901, 9, 0);
    case.assert_damage(
        &[
            ("SLEIGHT_OF_FLESH", 0, [0, 27, 0], 4),
            ("STRIKE", 0, [6, 0, 0], 3),
        ],
        26,
        7,
    );
    assert_eq!(case.row("WEAK_POTION", 0).kind, SourceKind::Potion);
    assert_eq!(case.row("BAG_OF_MARBLES", 0).kind, SourceKind::Relic);
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn offering_self_damage_and_draws_keep_inferno_and_speedster_separate() {
    let mut case = Scenario::new();
    let inferno = case.card(10, "INFERNO", 0);
    let speedster = case.card(11, "SPEEDSTER", 0);
    let offering = case.card(12, "OFFERING", 0);
    case.attach(201, "INFERNO_POWER", PLAYER_A, 6, inferno);
    case.attach(202, "SPEEDSTER_POWER", PLAYER_A, 2, speedster);
    let inferno = case.capture(2, 201);
    let speedster = case.capture(2, 202);
    let play = case.play(501, 12, "OFFERING", 0, 0, offering);
    case.report(offering, DIRECT, SELF_DAMAGE, 0, 6, 0);
    case.outgoing(inferno, 2, ATTRIBUTED, 900, 0, 6);
    case.outgoing(inferno, 2, ATTRIBUTED, 901, 4, 2);
    for (hp, blocked) in [(1, 1), (2, 0), (2, 0)] {
        case.outgoing(speedster, 2, ATTRIBUTED, 900, hp, blocked);
        case.outgoing(speedster, 2, ATTRIBUTED, 901, 2, 0);
    }
    case.assert_damage(
        &[
            ("INFERNO", 0, [0, 12, 0], 8),
            ("SPEEDSTER", 0, [0, 12, 0], 1),
        ],
        15,
        9,
    );
    assert_eq!(case.row("OFFERING", 0).self_damage, 6);
    assert_eq!(case.combat().damage_received, 6);
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn black_hole_star_gain_and_after_play_spend_preserve_supplier() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "BLACK_HOLE", 0);
    case.attach(201, "BLACK_HOLE_POWER", PLAYER_A, 3, supplier);
    let black_hole = case.capture(2, 201);
    let glow = case.card(11, "GLOW", 0);
    let gain_play = case.play(501, 11, "GLOW", 0, 0, glow);
    case.outgoing(black_hole, 2, ATTRIBUTED, 900, 2, 1);
    case.outgoing(black_hole, 2, ATTRIBUTED, 901, 1, 2);
    assert_eq!(case.state.card_play_finished(gain_play), 1);
    let cloak = case.card(12, "CLOAK_OF_STARS", 0);
    let spend_play = case.play(502, 12, "CLOAK_OF_STARS", 0, 0, cloak);
    assert_eq!(case.state.block_gained(7, 7, cloak, 0), 1);
    case.assert_damage(&[("BLACK_HOLE", 0, [0, 6, 0], 3)], 3, 3);
    assert_eq!(case.state.card_play_finished(spend_play), 1);
    case.outgoing(black_hole, 2, ATTRIBUTED, 900, 3, 0);
    case.outgoing(black_hole, 2, ATTRIBUTED, 901, 3, 0);
    case.assert_damage(&[("BLACK_HOLE", 0, [0, 12, 0], 3)], 9, 3);
    assert_eq!(case.row("CLOAK_OF_STARS", 0).block_gained, 7);
}

#[test]
fn boost_away_status_generation_triggers_smokestack_without_taking_block() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "SMOKESTACK", 0);
    let boost = case.card(11, "BOOST_AWAY", 0);
    case.attach(201, "SMOKESTACK_POWER", PLAYER_A, 5, supplier);
    let play = case.play(501, 11, "BOOST_AWAY", 0, 0, boost);
    assert_eq!(case.state.block_gained(7, 6, boost, 0), 1);
    assert_eq!(case.state.card_generated(7, 101, boost, 1), 1);
    let dazed = case.state.source_capture(7, 1, 101, "DAZED", 0, 0, 1);
    case.assert_source(dazed, &[("BOOST_AWAY", 0, 1)]);
    let smokestack = case.capture(2, 201);
    case.outgoing(smokestack, 2, ATTRIBUTED, 900, 3, 2);
    case.outgoing(smokestack, 2, ATTRIBUTED, 901, 5, 0);
    case.assert_damage(&[("SMOKESTACK", 0, [0, 10, 0], 2)], 8, 2);
    assert_eq!(case.row("BOOST_AWAY", 0).block_gained, 6);
    assert_eq!(case.combat().generation_triggers, 0);
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn sacrifice_osty_hp_loss_credits_necro_mastery_and_preserves_block_owner() {
    let mut case = Scenario::new();
    let mastery = case.card(10, "NECRO_MASTERY", 0);
    let sacrifice = case.card(11, "SACRIFICE", 0);
    case.attach(201, "NECRO_MASTERY_POWER", PLAYER_A, 1, mastery);
    assert_eq!(case.state.osty_summoned(7, mastery, 5, 0), 1);
    let play = case.play(501, 11, "SACRIFICE", 0, 0, sacrifice);
    assert_eq!(case.state.osty_killed(7, 0, play), 1);
    let proc = case.capture(2, 201);
    case.outgoing(proc, 2, ATTRIBUTED, 900, 5, 0);
    case.outgoing(proc, 2, ATTRIBUTED, 901, 3, 0);
    assert_eq!(case.state.block_gained(7, 15, sacrifice, 0), 1);
    case.assert_damage(&[("NECRO_MASTERY", 0, [0, 8, 0], 0)], 8, 0);
    assert_eq!(case.row("SACRIFICE", 0).block_gained, 15);
    assert_eq!(case.row("SACRIFICE", 0).block_effective, -5);
    assert_eq!(case.row("NECRO_MASTERY", 0).block_effective, 0);
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn pending_attack_thorns_and_inferno_keep_parent_modifier_budget() {
    let mut case = Scenario::new();
    let strike = case.card(10, "STRIKE", 0);
    let demon_form = case.card(11, "DEMON_FORM", 0);
    let inferno = case.card(12, "INFERNO", 0);
    let defend = case.card(13, "DEFEND", 0);
    case.attach(201, "STRENGTH_POWER", PLAYER_A, 3, demon_form);
    case.attach(202, "INFERNO_POWER", PLAYER_A, 6, inferno);
    case.attach(203, "THORNS_POWER", ENEMY, 3, 0);
    let strength = case.capture(2, 201);
    let inferno = case.capture(2, 202);
    let thorns = case.capture(2, 203);
    assert_eq!(case.state.block_gained(7, 1, defend, 0), 1);
    let play = case.play(501, 10, "STRIKE", 0, 0, strike);
    let parent = case
        .state
        .damage_calculation_begin(7, strike, 1, DIRECT, 900);
    assert_ne!(parent, 0);
    assert_eq!(
        case.state.damage_modifier_contribution(parent, strength, 3),
        1
    );
    let retaliation = case
        .state
        .damage_calculation_begin(7, thorns, 2, ATTRIBUTED, 1000);
    assert_ne!(retaliation, 0);
    assert_eq!(
        case.state
            .damage_result_append(retaliation, 3, 2, 1, INCOMING, 0, 0),
        1
    );
    assert_eq!(case.state.damage_calculation_commit(retaliation), 1);
    case.outgoing(inferno, 2, ATTRIBUTED, 900, 0, 6);
    case.outgoing(inferno, 2, ATTRIBUTED, 901, 2, 0);
    case.assert_damage(&[("INFERNO", 0, [0, 8, 0], 6)], 2, 6);
    assert_eq!(case.combat().damage_received, 3);
    assert_eq!(case.row("DEFEND", 0).block_effective, 1);
    assert_eq!(
        case.state
            .damage_result_append(parent, 9, 5, 4, OUTGOING, 4, 0),
        1
    );
    assert_eq!(case.state.damage_calculation_commit(parent), 1);
    case.assert_damage(
        &[
            ("INFERNO", 0, [0, 8, 0], 6),
            ("DEMON_FORM", 0, [0, 0, 3], 1),
            ("STRIKE", 0, [6, 0, 0], 3),
        ],
        7,
        10,
    );
    assert_eq!(case.state.card_play_finished(play), 1);
}

#[test]
fn generated_anger_and_deck_copy_split_rows_through_clone_and_transfer() {
    let mut case = Scenario::new();
    let discovery = case.card(10, "DISCOVERY", 0);
    let generator_play = case.play(501, 10, "DISCOVERY", 0, 0, discovery);
    assert_eq!(case.state.card_generated(7, 101, discovery, 1), 1);
    assert_eq!(case.state.card_play_finished(generator_play), 1);
    let generated = case.state.source_capture(7, 1, 101, "ANGER", 0, 1, 1);
    let ordinary = case.card(102, "ANGER", 1);
    let play = case.play(502, 101, "ANGER", 1, 1, generated);
    case.outgoing(generated, 1, DIRECT, 900, 4, 2);
    assert_eq!(case.state.card_generated(7, 103, generated, 1), 1);
    assert_eq!(case.state.card_play_finished(play), 1);
    let own_play = case.play(503, 102, "ANGER", 1, 0, ordinary);
    case.outgoing(ordinary, 1, DIRECT, 900, 6, 0);
    assert_eq!(case.state.card_generated(7, 104, ordinary, 1), 1);
    assert_eq!(case.state.card_play_finished(own_play), 1);
    assert_eq!(case.state.turn_started(7), 1);
    let clone = case.state.source_capture(7, 1, 103, "ANGER", 0, 2, 1);
    case.assert_source(clone, &[("DISCOVERY", 0, 1)]);
    let transferred_play = case.play(504, 103, "ANGER", 2, 1, clone);
    case.outgoing(clone, 1, DIRECT, 901, 5, 1);
    assert_eq!(case.state.card_generated(7, 105, clone, 1), 1);
    assert_eq!(case.state.card_play_finished(transferred_play), 1);
    case.assert_damage(
        &[("DISCOVERY", 0, [12, 0, 0], 3), ("ANGER", 1, [6, 0, 0], 0)],
        15,
        3,
    );
    assert_eq!(case.row("DISCOVERY", 0).plays, 1);
    assert_eq!(case.row("ANGER", 1).plays, 1);
    assert_eq!((case.combat().plays, case.combat().generated_plays), (4, 2));
    assert_eq!(case.combat().generation_triggers, 0);
    assert!(case.combat().cards.iter().all(|row| row.player != 2));
}

#[test]
fn saved_serpent_and_enemy_strangle_exclude_stacks_added_during_play() {
    let mut case = Scenario::new();
    let serpent = case.card(10, "SERPENT_FORM", 0);
    let discovery = case.card(11, "DISCOVERY", 0);
    let strangle = case.card(12, "STRANGLE", 0);
    case.attach(201, "SERPENT_FORM_POWER", PLAYER_A, 4, serpent);
    case.attach(202, "STRANGLE_POWER", ENEMY, 2, strangle);
    assert_eq!(case.state.card_generated(7, 101, discovery, 1), 1);
    let generated = case
        .state
        .source_capture(7, 1, 101, "SERPENT_FORM", 0, 0, 1);
    let play = case.play(501, 101, "SERPENT_FORM", 0, 1, generated);
    let saved_serpent = case.capture(2, 201);
    let saved_strangle = case.capture(2, 202);
    case.change(201, "SERPENT_FORM_POWER", PLAYER_A, (4, 8), generated);
    assert_eq!(case.state.card_play_finished(play), 1);
    case.outgoing(saved_serpent, 2, ATTRIBUTED, 900, 3, 1);
    case.outgoing(saved_strangle, 2, ATTRIBUTED, 900, 2, 0);
    case.assert_damage(
        &[
            ("SERPENT_FORM", 0, [0, 4, 0], 1),
            ("STRANGLE", 0, [0, 2, 0], 0),
        ],
        5,
        1,
    );
    let defy = case.card(13, "DEFY", 0);
    let next = case.play(502, 13, "DEFY", 0, 0, defy);
    let next_serpent = case.capture(2, 201);
    let next_strangle = case.capture(2, 202);
    assert_eq!(case.state.block_gained(7, 6, defy, 0), 1);
    case.attach(203, "WEAK_POWER", ENEMY, 1, defy);
    assert_eq!(case.state.card_play_finished(next), 1);
    case.outgoing(next_serpent, 2, ATTRIBUTED, 900, 8, 0);
    case.outgoing(next_strangle, 2, ATTRIBUTED, 900, 2, 0);
    case.assert_damage(
        &[
            ("SERPENT_FORM", 0, [0, 8, 0], 1),
            ("DISCOVERY", 0, [0, 4, 0], 0),
            ("STRANGLE", 0, [0, 4, 0], 0),
        ],
        15,
        1,
    );
    assert!(
        case.combat()
            .cards
            .iter()
            .all(|row| row.player != TEAM_SLOT)
    );
}

#[test]
fn fifth_completed_card_triggers_panache_without_claiming_its_attacks() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "PANACHE", 0);
    let strike = case.card(11, "STRIKE", 0);
    case.attach(201, "PANACHE_POWER", PLAYER_A, 10, supplier);
    for execution in 501..506 {
        let play = case.play(execution, 11, "STRIKE", 0, 0, strike);
        case.outgoing(strike, 1, DIRECT, 900, 6, 0);
        assert_eq!(case.state.card_play_finished(play), 1);
        if execution == 505 {
            let panache = case.capture(2, 201);
            case.outgoing(panache, 2, ATTRIBUTED, 900, 10, 0);
            case.outgoing(panache, 2, ATTRIBUTED, 901, 7, 3);
        }
    }
    case.assert_damage(
        &[("STRIKE", 0, [30, 0, 0], 0), ("PANACHE", 0, [0, 20, 0], 3)],
        47,
        3,
    );
    assert_eq!(case.row("STRIKE", 0).plays, 5);
    assert_eq!(case.combat().plays, 5);
}

#[test]
fn dualcast_first_later_orb_hits_and_thunder_keep_three_supplier_routes() {
    let mut case = Scenario::new();
    let channeler = case.card(10, "ZAP", 0);
    let supplier = case.card(11, "THUNDER", 0);
    let dualcast = case.card(12, "DUALCAST", 0);
    case.attach(201, "THUNDER_POWER", PLAYER_A, 8, supplier);
    assert_eq!(case.state.orb_channeled(7, 301, channeler), 1);
    assert_eq!(case.state.orb_channeled(7, 302, channeler), 1);
    let play = case.play(501, 12, "DUALCAST", 0, 0, dualcast);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 0), 1);
    let first = case.capture(3, 301);
    case.outgoing(first, 5, ATTRIBUTED, 900, 3, 5);
    let thunder = case.capture(2, 201);
    case.outgoing(thunder, 2, ATTRIBUTED, 900, 8, 0);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 0), 2);
    case.outgoing(dualcast, 5, DIRECT, 901, 6, 2);
    case.outgoing(thunder, 2, ATTRIBUTED, 901, 8, 0);
    assert_eq!(case.state.card_play_finished(play), 1);
    assert_eq!(case.state.orb_context_begin(7, 302, 0, 0), 1);
    let outside_play = case.capture(3, 302);
    case.outgoing(outside_play, 5, ATTRIBUTED, 902, 3, 0);
    case.assert_damage(
        &[
            ("ZAP", 0, [0, 11, 0], 5),
            ("THUNDER", 0, [0, 16, 0], 0),
            ("DUALCAST", 0, [8, 0, 0], 2),
        ],
        28,
        7,
    );
    assert_eq!(case.row("DUALCAST", 0).plays, 1);
}

#[test]
fn lethal_modifier_budget_cannot_borrow_from_previous_large_hit() {
    let mut case = Scenario::new();
    let bludgeon = case.card(10, "BLUDGEON", 0);
    let first_play = case.play(501, 10, "BLUDGEON", 0, 0, bludgeon);
    case.outgoing(bludgeon, 1, DIRECT, 900, 24, 8);
    assert_eq!(case.state.card_play_finished(first_play), 1);
    let demon_form = case.card(11, "DEMON_FORM", 0);
    case.attach(201, "STRENGTH_POWER", PLAYER_A, 3, demon_form);
    let strength = case.capture(2, 201);
    let second_play = case.play(502, 10, "BLUDGEON", 0, 0, bludgeon);
    let lethal = case
        .state
        .damage_calculation_begin(7, bludgeon, 1, DIRECT, 901);
    assert_ne!(lethal, 0);
    assert_eq!(
        case.state.damage_modifier_contribution(lethal, strength, 3),
        1
    );
    assert_eq!(
        case.state
            .damage_result_append(lethal, 2, 1, 1, OUTGOING, 4, 0),
        1
    );
    assert_eq!(case.state.damage_calculation_commit(lethal), 1);
    case.assert_damage(
        &[
            ("BLUDGEON", 0, [32, 0, 0], 8),
            ("DEMON_FORM", 0, [0, 0, 2], 1),
        ],
        25,
        9,
    );
    assert_eq!(case.state.card_play_finished(second_play), 1);
}

#[test]
fn ordinary_hit_on_poisoned_target_does_not_consume_poison_suppliers() {
    let mut case = Scenario::new();
    let first = case.card(10, "POISONED_STAB", 0);
    let second = case.model("POISON_POTION", 3, 1);
    case.attach(201, "POISON_POWER", ENEMY, 2, first);
    case.change(201, "POISON_POWER", ENEMY, (2, 5), second);
    let strike = case.card(11, "STRIKE", 0);
    let play = case.play(501, 11, "STRIKE", 0, 0, strike);
    case.outgoing(strike, 1, DIRECT, 900, 4, 2);
    case.assert_damage(&[("STRIKE", 0, [6, 0, 0], 2)], 4, 2);
    assert_eq!(case.state.card_play_finished(play), 1);
    let poison = case.capture(2, 201);
    case.assert_source(poison, &[("POISONED_STAB", 0, 2), ("POISON_POTION", 1, 3)]);
    case.outgoing(poison, 2, ATTRIBUTED, 900, 5, 0);
    case.change(201, "POISON_POWER", ENEMY, (5, 4), 0);
    let next_tick = case.capture(2, 201);
    case.assert_source(
        next_tick,
        &[("POISONED_STAB", 0, 1), ("POISON_POTION", 1, 3)],
    );
    case.outgoing(next_tick, 2, ATTRIBUTED, 900, 4, 0);
    case.assert_damage(
        &[
            ("STRIKE", 0, [6, 0, 0], 2),
            ("POISONED_STAB", 0, [0, 3, 0], 0),
            ("POISON_POTION", 1, [0, 6, 0], 0),
        ],
        13,
        2,
    );
}

#[test]
fn doom_partial_batches_debit_ordered_grants_only_after_acceptance() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_DOOM", 0);
    let second = case.card(11, "SECOND_DOOM", 1);
    let third = case.card(12, "THIRD_DOOM", 2);
    let last = case.card(13, "LAST_DOOM", 3);
    let mixed = case.mix(&[(second, 1), (third, 1)]);
    case.attach(201, "DOOM_POWER", ENEMY, 3, first);
    case.change(201, "DOOM_POWER", ENEMY, (3, 7), mixed);
    case.change(201, "DOOM_POWER", ENEMY, (7, 9), last);
    let aborted = case.state.doom_batch_begin(7);
    assert_ne!(aborted, 0);
    assert_eq!(case.state.doom_target_capture(aborted, 900, 201, 8), 1);
    assert_eq!(case.state.doom_batch_abort(aborted), 1);
    case.assert_damage(&[], 0, 0);
    let first_batch = case.state.doom_batch_begin(7);
    assert_ne!(first_batch, 0);
    assert_eq!(case.state.doom_target_capture(first_batch, 900, 201, 2), 1);
    assert_eq!(case.state.doom_kills_completed(first_batch), 1);
    case.assert_damage(&[("FIRST_DOOM", 0, [0, 2, 0], 0)], 2, 0);
    let remaining = case.capture(2, 201);
    case.assert_source(
        remaining,
        &[
            ("FIRST_DOOM", 0, 1),
            ("SECOND_DOOM", 1, 2),
            ("THIRD_DOOM", 2, 2),
            ("LAST_DOOM", 3, 2),
        ],
    );
    let second_batch = case.state.doom_batch_begin(7);
    assert_ne!(second_batch, 0);
    assert_eq!(case.state.doom_target_capture(second_batch, 900, 201, 4), 1);
    assert_eq!(case.state.doom_kills_completed(second_batch), 1);
    case.assert_damage(
        &[
            ("FIRST_DOOM", 0, [0, 3, 0], 0),
            ("SECOND_DOOM", 1, [0, 1, 0], 0),
            ("THIRD_DOOM", 2, [0, 2, 0], 0),
        ],
        6,
        0,
    );
    let third_batch = case.state.doom_batch_begin(7);
    assert_ne!(third_batch, 0);
    assert_eq!(case.state.doom_target_capture(third_batch, 900, 201, 5), 1);
    assert_eq!(case.state.doom_kills_completed(third_batch), 1);
    case.assert_damage(
        &[
            ("FIRST_DOOM", 0, [0, 3, 0], 0),
            ("SECOND_DOOM", 1, [0, 1, 0], 0),
            ("THIRD_DOOM", 2, [0, 3, 0], 0),
            ("LAST_DOOM", 3, [0, 2, 0], 0),
            ("UNATTRIBUTED", TEAM_SLOT, [0, 2, 0], 0),
        ],
        11,
        0,
    );
}

#[test]
fn sentry_mode_generated_osty_attack_keeps_direct_supplier_and_ignores_modifiers() {
    let mut case = Scenario::new();
    let sentry = case.card(10, "SENTRY_MODE", 0);
    let sentry_play = case.play(501, 10, "SENTRY_MODE", 0, 0, sentry);
    case.attach(201, "SENTRY_MODE_POWER", PLAYER_A, 1, sentry);
    assert_eq!(case.state.card_play_finished(sentry_play), 1);
    let generation = case.capture(2, 201);
    assert_eq!(case.state.turn_started(7), 1);
    assert_eq!(case.state.card_generated(7, 101, generation, 2), 1);
    let gaze = case
        .state
        .source_capture(7, 1, 101, "SWEEPING_GAZE", 0, 0, 1);
    case.assert_source(gaze, &[("SENTRY_MODE", 0, 1)]);
    let play = case.play(502, 101, "SWEEPING_GAZE", 0, 1, gaze);
    let modifier = case.card(11, "INFLAME", 0);
    case.attach(202, "STRENGTH_POWER", PLAYER_A, 2, modifier);
    let strength = case.capture(2, 202);
    let calculation = case.state.damage_calculation_begin(7, gaze, 1, DIRECT, 900);
    assert_ne!(calculation, 0);
    // The specialized Osty branch excludes even an extra captured modifier.
    assert_eq!(
        case.state
            .damage_modifier_contribution(calculation, strength, 2),
        1
    );
    assert_eq!(
        case.state
            .damage_result_append(calculation, 10, 7, 3, OSTY_DEALT, 4, 0),
        1
    );
    assert_eq!(case.state.damage_calculation_commit(calculation), 1);
    assert_eq!(case.state.card_play_finished(play), 1);
    case.assert_damage(&[("SENTRY_MODE", 0, [10, 0, 0], 3)], 7, 3);
    assert_eq!((case.combat().plays, case.combat().generated_plays), (2, 1));
    assert_eq!(case.row("SENTRY_MODE", 0).plays, 1);
    assert_eq!(case.combat().generation_triggers, 0);
    assert!(
        case.combat()
            .cards
            .iter()
            .all(|row| row.id != "SWEEPING_GAZE")
    );
}

#[test]
fn hibernate_frost_first_and_later_evoke_feed_both_players_block_pools() {
    let mut case = Scenario::new();
    let discovery = case.card(10, "DISCOVERY", 0);
    assert_eq!(case.state.card_generated(7, 101, discovery, 1), 1);
    let hibernate = case.state.source_capture(7, 1, 101, "HIBERNATE", 0, 1, 1);
    let hibernate_play = case.play(501, 101, "HIBERNATE", 1, 1, hibernate);
    case.attach(201, "HIBERNATE_POWER", PLAYER_B, 1, hibernate);
    assert_eq!(case.state.orb_channeled(7, 301, hibernate), 1);
    assert_eq!(case.state.orb_channeled(7, 302, hibernate), 1);
    assert_eq!(case.state.card_play_finished(hibernate_play), 1);
    let dualcast = case.card(11, "DUALCAST", 1);
    let play = case.play(502, 11, "DUALCAST", 1, 0, dualcast);
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 1);
    let frost = case.capture(3, 301);
    case.assert_source(frost, &[("DISCOVERY", 0, 1)]);
    for receiver in [0, 1] {
        assert_eq!(case.state.block_gained(7, 5, frost, receiver), 1);
    }
    assert_eq!(case.state.orb_context_begin(7, 301, play, 1), 2);
    for receiver in [0, 1] {
        assert_eq!(case.state.block_gained(7, 5, dualcast, receiver), 1);
    }
    assert_eq!(case.state.card_play_finished(play), 1);
    case.report(0, ATTRIBUTED, INCOMING, 0, 7, 7);
    case.report(0, ATTRIBUTED, INCOMING, 1, 3, 3);
    assert_eq!(case.row("DISCOVERY", 0).block_effective, 8);
    assert_eq!(case.row("DUALCAST", 1).block_effective, 2);
    case.report(0, ATTRIBUTED, INCOMING, 0, 3, 3);
    case.report(0, ATTRIBUTED, INCOMING, 1, 7, 7);
    assert_eq!(case.row("DISCOVERY", 0).block_gained, 10);
    assert_eq!(case.row("DISCOVERY", 0).block_effective, 10);
    assert_eq!(case.row("DUALCAST", 1).block_gained, 10);
    assert_eq!(case.row("DUALCAST", 1).block_effective, 10);
    assert_eq!(
        (case.combat().block_total, case.combat().damage_received),
        (20, 20)
    );
    case.assert_damage(&[], 0, 0);
}

#[test]
fn equal_source_block_merge_after_partial_consumption_keeps_prefix() {
    let mut case = Scenario::new();
    let first = case.card(10, "FIRST_SHIELD", 0);
    let second = case.card(11, "SECOND_SHIELD", 2);
    let mixed = case.mix(&[(first, 1), (second, 1)]);
    assert_eq!(case.state.block_gained(7, 4, mixed, 1), 1);
    case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    assert_eq!(case.row("FIRST_SHIELD", 0).block_effective, 0);
    assert_eq!(case.row("SECOND_SHIELD", 2).block_effective, 1);
    assert_eq!(case.state.block_gained(7, 2, mixed, 1), 1);
    case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    assert_eq!(case.row("FIRST_SHIELD", 0).block_effective, 1);
    assert_eq!(case.row("SECOND_SHIELD", 2).block_effective, 1);
    for _ in 0..4 {
        case.report(0, ATTRIBUTED, INCOMING, 1, 1, 1);
    }
    assert_eq!(case.row("FIRST_SHIELD", 0).block_gained, 3);
    assert_eq!(case.row("SECOND_SHIELD", 2).block_gained, 3);
    assert_eq!(case.row("FIRST_SHIELD", 0).block_effective, 3);
    assert_eq!(case.row("SECOND_SHIELD", 2).block_effective, 3);
    assert_eq!(
        (case.combat().block_total, case.combat().damage_received),
        (6, 6)
    );
}
