use super::*;
use crate::data::state::CardStat;

const DIRECT: i32 = 0;
const ATTRIBUTED: i32 = 1;
const OUTGOING: i32 = 0;
const INCOMING: i32 = 1;
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
                }),
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
fn ally_soulbound_credits_supplier_instead_of_power_owner() {
    let mut case = Scenario::new();
    let supplier = case.card(10, "SOULBOUND", 0);
    case.attach(201, "SOULBOUND_POWER", PLAYER_B, 1, supplier);
    let soulbound = case.capture(2, 201);
    case.assert_source(soulbound, &[("SOULBOUND", 0, 1)]);
    case.report(soulbound, ATTRIBUTED, OUTGOING, 4, 5, 0);
    assert_eq!(case.state.buff_mitigation(7, soulbound, 3), 1);
    let row = case.row("SOULBOUND", 0);
    assert_eq!((row.dmg_attributed, row.mitigate_buff), (5, 3));
    assert!(case.combat().cards.iter().all(|row| row.player != 1));
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
    });
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
