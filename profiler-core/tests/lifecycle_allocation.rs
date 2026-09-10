#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::events;
use profiler_core::data::state::{STATE, SourceKind, caps};
use profiler_core::test_util::{self, SourceFixture};

#[test]
fn rejected_model_ids_preserve_creditor_and_valid_prefix_identity() {
    let dir = test_util::unique_dir("bounded-model-admission");
    events::test_reset();
    events::init(&dir);
    events::combat_started("", "");
    let prefix = "X".repeat(caps::MODEL_ID_BYTES);
    for id in [format!("{prefix}A"), format!("{prefix}B"), "X\0Y".into()] {
        SourceFixture::card(&id, 2).deal(3, 0);
    }
    SourceFixture::card(&prefix, 2).deal(7, 0);
    SourceFixture::card("RECOVERED", 1).deal(5, 0);
    STATE.with(|cell| {
        let state = cell.borrow();
        let rows = &state.current.as_ref().expect("combat remains active").cards;
        assert_eq!(rows.len(), 3);
        let unknown = rows
            .iter()
            .find(|row| row.kind == SourceKind::Unknown)
            .expect("invalid IDs retain known-slot Unknown");
        assert_eq!(unknown.player, 2);
        assert_eq!(unknown.damage_dealt, 9);
        assert!(
            rows.iter()
                .any(|row| row.id.as_str() == prefix && row.damage_dealt == 7)
        );
        assert!(
            rows.iter()
                .any(|row| row.id == "RECOVERED" && row.player == 1 && row.damage_dealt == 5)
        );
    });
}

#[test]
fn invalid_metadata_cannot_close_or_interrupt_the_active_run() {
    let dir = test_util::unique_dir("bounded-lifecycle-admission");
    events::test_reset();
    events::init(&dir);
    events::run_started("IRONCLAD", 0, "Standard", "ORIGINAL", 0, "1", 123);
    let epoch = events::combat_started("ORIGINAL", "Monster");
    SourceFixture::card("STRIKE", 0).deal(6, 0);
    let bad_model = "X".repeat(caps::MODEL_ID_BYTES + 1);
    let bad_seed = "S".repeat(caps::SEED_BYTES + 1);
    let bad_label = "L".repeat(caps::LABEL_BYTES + 1);
    let bad_net = "1".repeat(caps::NET_ID_BYTES + 1);
    for (character, mode, seed, net) in [
        ("IRONCLAD", "Standard", bad_seed.as_str(), "1"),
        (bad_model.as_str(), "Standard", "NEXT", "1"),
        ("IRONCLAD", bad_label.as_str(), "NEXT", "1"),
        ("IRONCLAD", "Standard", "NEXT", bad_net.as_str()),
        ("IRON\0CLAD", "Standard", "NEXT", "1"),
        ("IRONCLAD", "Standard", "N\0EXT", "1"),
        ("IRONCLAD", "Standard", "NEXT", "1\0"),
    ] {
        let counts = allocation_support::measure_and_drop(|| {
            events::run_started(character, 0, mode, seed, 1, net, 124);
        });
        assert!(counts.all_zero(), "run metadata rejection: {counts:?}");
        STATE.with(|cell| {
            let state = cell.borrow();
            assert_eq!(
                state.run_ctx.as_ref().expect("prior run survives").run.seed,
                "ORIGINAL"
            );
            assert_eq!(
                u64::from(state.current.as_ref().expect("prior combat survives").seq),
                epoch
            );
            let combat = state.current.as_ref().expect("prior combat survives");
            assert_eq!(combat.cards.len(), 1);
            assert_eq!(combat.cards[0].id, "STRIKE");
            assert_eq!(combat.cards[0].damage_dealt, 6);
        });
        assert!(!dir.join("runs.jsonl").exists());
        assert!(!dir.join("runs/1/1.json").exists());
    }
    for (id, kind) in [
        (bad_model.as_str(), "Monster"),
        ("NEXT", bad_label.as_str()),
        ("N\0EXT", "Monster"),
    ] {
        let (result, counts) = allocation_support::measure(|| events::combat_started(id, kind));
        assert_eq!(result, 0);
        assert!(counts.all_zero(), "combat metadata rejection: {counts:?}");
        assert_eq!(test_util::combat_epoch(), epoch);
        assert!(!dir.join("runs/1/1.json").exists());
    }
    assert_eq!(events::combat_ended(epoch), 1);
    events::run_started("DEFECT", 0, "Standard", "RECOVERED", 0, "2", 125);
    STATE.with(|cell| {
        assert_eq!(
            cell.borrow()
                .run_ctx
                .as_ref()
                .expect("valid replacement succeeds")
                .run
                .seed,
            "RECOVERED"
        )
    });
}

#[test]
fn exact_utf8_metadata_limits_survive_combat_roster_copy() {
    let dir = test_util::unique_dir("bounded-metadata-exact");
    let character = "é".repeat(caps::MODEL_ID_BYTES / 2);
    let characters = [character.as_str(); caps::MAX_PLAYERS].join(",");
    let net = "9".repeat(caps::NET_ID_BYTES);
    let nets = [net.as_str(); caps::MAX_PLAYERS].join(",");
    let mode = "é".repeat(caps::LABEL_BYTES / 2);
    let seed = "é".repeat(caps::SEED_BYTES / 2);
    assert_eq!(characters.len(), caps::RUN_CHARACTER_BYTES);
    events::test_reset();
    events::init(&dir);
    events::run_started(&characters, 0, &mode, &seed, 0, &nets, 123);
    let epoch = events::combat_started(&character, &mode);
    assert_ne!(epoch, 0);
    STATE.with(|cell| {
        let state = cell.borrow();
        let run = state.run_ctx.as_ref().expect("exact limits are admitted");
        let combat = state
            .current
            .as_ref()
            .expect("combat copies the full roster");
        assert_eq!(run.run.character.as_str(), characters);
        assert_eq!(run.run.seed.as_str(), seed);
        assert_eq!(combat.encounter_id.as_str(), character);
        assert_eq!(combat.encounter_type.as_str(), mode);
        assert_eq!(combat.players.len(), caps::MAX_PLAYERS);
        for player in &combat.players {
            assert_eq!(player.character.as_str(), character);
            assert_eq!(player.net_id.as_str(), net);
        }
    });
}

#[test]
fn initialized_combat_lifecycle_retains_buffers_without_allocator_operations() {
    let dir = test_util::unique_dir("retained-combat-lifecycle");
    events::test_reset();
    events::init(&dir);
    let initial = test_util::lifecycle_storage_snapshot();
    assert!(
        initial
            .iter()
            .all(|buffer| buffer.is_some_and(|(_, capacity)| capacity > 0))
    );
    let repeated_init = allocation_support::measure_and_drop(|| events::init(&dir));
    assert!(repeated_init.all_zero(), "repeated init: {repeated_init:?}");
    for _ in 0..3 {
        let (epoch, start_counts) = allocation_support::measure(|| events::combat_started("", ""));
        assert_ne!(epoch, 0);
        assert!(start_counts.all_zero(), "combat start: {start_counts:?}");
        let (staged, finish_counts) =
            allocation_support::measure(|| test_util::finish_combat_for_allocation_probe(epoch));
        assert!(
            finish_counts.all_zero(),
            "combat staging: {finish_counts:?}"
        );
        let staged = staged.expect("active combat stages into the reserved owner");
        assert_eq!(u64::from(staged.seq), epoch);
        let drop_counts = allocation_support::measure_and_drop(|| drop(staged));
        assert!(
            drop_counts.all_zero(),
            "combat lease return: {drop_counts:?}"
        );
        let suspend_counts = allocation_support::measure_and_drop(events::run_suspended);
        assert!(
            suspend_counts.all_zero(),
            "empty-source discard: {suspend_counts:?}"
        );
        assert_eq!(test_util::lifecycle_storage_snapshot(), initial);
    }
    events::run_started("IRONCLAD,DEFECT", 0, "Standard", "FIRST", 0, "1,2", 123);
    let (epoch, start_counts) =
        allocation_support::measure(|| events::combat_started("ROSTER", "Monster"));
    assert_ne!(epoch, 0);
    assert!(
        start_counts.all_zero(),
        "first roster copy: {start_counts:?}"
    );
    assert_eq!(test_util::lifecycle_storage_snapshot(), initial);
    events::run_started("SILENT", 0, "Standard", "SECOND", 0, "3", 124);
    assert_eq!(test_util::lifecycle_storage_snapshot(), initial);
}

#[test]
fn failed_initialization_disables_events_and_never_retries_reservation() {
    let dir = test_util::unique_dir("retained-init-failure");
    events::test_reset();
    test_util::fail_lifecycle_initialization();
    events::init(&dir);
    let failed = test_util::lifecycle_storage_snapshot();
    assert!(
        failed
            .iter()
            .any(|buffer| buffer.is_some_and(|(_, capacity)| capacity > 0))
    );
    assert!(
        failed
            .iter()
            .any(|buffer| buffer.is_none_or(|(_, capacity)| capacity == 0))
    );
    let repeated = allocation_support::measure_and_drop(|| events::init(&dir));
    assert!(
        repeated.all_zero(),
        "disabled init must not reserve again: {repeated:?}"
    );
    assert_eq!(test_util::lifecycle_storage_snapshot(), failed);
    assert_eq!(events::combat_started("REJECTED", "Monster"), 0);
    events::run_started("IRONCLAD", 0, "Standard", "REJECTED", 0, "1", 123);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert!(state.current.is_none());
        assert!(state.run_ctx.is_none());
    });
    assert!(!dir.join("profiler.log").exists());
    assert!(!dir.join("runs.jsonl").exists());
}

#[test]
fn run_publication_and_finish_reuse_roster_and_accumulator_storage() {
    use profiler_core::data::state::{CardStat, RunOutcome, RunPlayer, RunSnapshot};
    let dir = test_util::unique_dir("retained-run-lifecycle");
    let run = RunSnapshot {
        seq: 1,
        seed: test_util::text("RETAINED"),
        ..RunSnapshot::default()
    };
    let players = [RunPlayer {
        slot: 0,
        character: test_util::text("IRONCLAD"),
        net_id: test_util::text("1"),
    }];
    events::test_reset();
    events::init(&dir);
    let initial = test_util::lifecycle_storage_snapshot();
    for roster in [&players[..], &[][..], &players[..]] {
        let (published, publish_counts) = allocation_support::measure(|| {
            test_util::replace_run_for_allocation_probe(&run, roster)
        });
        assert!(published);
        assert!(
            publish_counts.all_zero(),
            "run publication: {publish_counts:?}"
        );
        STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            assert!(state.run_cards.is_empty());
            assert_eq!(
                state
                    .run_ctx
                    .as_ref()
                    .expect("published run is active")
                    .players,
                roster
            );
            state.run_cards.push(CardStat {
                id: test_util::text("STRIKE"),
                damage_dealt: 5,
                ..CardStat::default()
            });
        });
        let (staged, finish_counts) = allocation_support::measure(|| {
            test_util::finish_run_for_allocation_probe(RunOutcome::Victory)
        });
        assert!(finish_counts.all_zero(), "run staging: {finish_counts:?}");
        let staged = staged.expect("active run stages into the retained owner");
        assert_eq!(staged.context.run.seed, "RETAINED");
        assert_eq!(staged.context.players, roster);
        let drop_counts = allocation_support::measure_and_drop(|| drop(staged));
        assert!(drop_counts.all_zero(), "run lease return: {drop_counts:?}");
        assert_eq!(test_util::lifecycle_storage_snapshot(), initial);
    }
}

#[test]
fn unused_invalid_wire_id_preserves_captured_card_provenance() {
    let dir = test_util::unique_dir("bounded-captured-provenance");
    events::test_reset();
    events::init(&dir);
    let epoch = events::combat_started("", "");
    let supplier = SourceFixture::card("CAPTURED", 2);
    let oversized = "X".repeat(caps::MODEL_ID_BYTES + 1);
    for (index, id) in [oversized.as_str(), "N\0UL"].into_iter().enumerate() {
        let identity = index as u64 + 1;
        let play = supplier.with_transfer(|transfer| {
            events::card_play_started(epoch, identity, identity, id, 2, 0, 1, 0, transfer)
        });
        assert_ne!(play, 0);
        assert_eq!(events::card_play_finished(play), 1);
    }
    STATE.with(|cell| {
        let state = cell.borrow();
        let combat = state
            .current
            .as_ref()
            .expect("combat retains credited plays");
        assert_eq!(combat.plays, 2);
        assert_eq!(combat.cards.len(), 1);
        assert_eq!(combat.cards[0].id, "CAPTURED");
        assert_eq!(combat.cards[0].player, 2);
        assert_eq!(combat.cards[0].plays, 2);
    });
}
