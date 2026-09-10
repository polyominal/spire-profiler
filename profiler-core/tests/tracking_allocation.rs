#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::events;
use profiler_core::data::state::caps;
use profiler_core::test_util;

#[test]
fn full_grant_orb_capture_and_reset_cycles_make_no_allocator_calls() {
    let dir = test_util::unique_dir("retained-tracking-allocation");
    events::test_reset();
    events::init(&dir);
    for _ in 0..3 {
        let counts = allocation_support::measure_and_drop(|| {
            let epoch = events::combat_started("", "");
            let a = events::source_capture(epoch, 1, 5000, "A", 0, 0, 0);
            let b = events::source_capture(epoch, 1, 5001, "B", 0, 0, 0);
            assert_ne!(a, 0);
            assert_ne!(b, 0);
            for instance in 1..=caps::POWER_INSTANCES as u64 {
                assert_eq!(
                    events::power_attached(epoch, instance, "POWER", instance + 1000, 1, 4, 1, a),
                    1
                );
            }
            for instance in 1..=5 {
                let layers = if instance == 5 { 5 } else { 64 };
                for old in 1..layers {
                    let source = if old % 2 == 1 { b } else { a };
                    assert_eq!(
                        events::power_amount_changed(
                            epoch,
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
            let captured = events::source_capture(epoch, 2, 5, "", 2, 4, 0);
            assert_eq!(events::source_count(captured), 2);
            assert_eq!(
                events::power_amount_changed(epoch, 5, "POWER", 1005, 1, 4, 5, 6, b),
                1
            );
            assert_eq!(events::power_removed(epoch, 1), 1);
            assert_eq!(
                events::power_attached(epoch, 1000, "POWER", 2000, 1, 4, 1, b),
                1
            );
            assert_eq!(events::source_weight(captured, 0), 3);
            assert_eq!(events::source_weight(captured, 1), 2);
            for instance in 1..=caps::ORB_SOURCES as u64 {
                assert_eq!(events::orb_channeled(epoch, instance, a), 1);
            }
            assert_eq!(events::orb_channeled(epoch, 1000, a), 0);
            assert_eq!(events::orb_channeled(epoch, 1, b), 1);
            assert_eq!(events::orb_channeled(epoch, 1, u64::MAX), 0);
            assert_eq!(events::orb_context_begin(epoch, 1, 0, 0), 0);
            let finished = test_util::finish_combat_for_allocation_probe(epoch)
                .expect("active combat stages into retained storage");
            drop(finished);
            events::run_suspended();
            assert_eq!(events::source_count(captured), -1);
        });
        assert!(counts.all_zero(), "power/orb/reset cycle: {counts:?}");
    }
}

#[test]
fn play_and_generated_rejection_release_and_reset_make_no_allocator_calls() {
    let dir = test_util::unique_dir("retained-tracking-admission");
    events::test_reset();
    events::init(&dir);
    let epoch = events::combat_started("", "");
    let source = events::source_capture(epoch, 1, 5000, "SOURCE", 0, 0, 0);
    for instance in 1..=caps::GENERATED_INSTANCES as u64 {
        assert_eq!(events::card_generated(epoch, instance, source, 1), 1);
    }
    let mut plays = Vec::new();
    for owner in 0..caps::MAX_PLAYER_SLOTS as i32 {
        for count in 0..caps::ACTIVE_PLAYS_PER_SLOT {
            let execution = 1 + (owner as usize * caps::ACTIVE_PLAYS_PER_SLOT + count) as u64;
            let play =
                events::card_play_started(epoch, execution, 5000, "SOURCE", owner, 0, 1, 0, source);
            assert_ne!(play, 0);
            plays.push(play);
        }
    }
    let counts = allocation_support::measure_and_drop(|| {
        assert_eq!(events::card_generated(epoch, 1000, source, 1), 0);
        assert_eq!(events::card_generated(epoch, 1, u64::MAX, 1), 0);
        for owner in 0..caps::MAX_PLAYER_SLOTS as i32 {
            assert_eq!(
                events::card_play_started(epoch, 1000, 5000, "SOURCE", owner, 0, 1, 0, source),
                0
            );
        }
        for &play in &plays {
            assert_eq!(events::card_play_finished(play), 1);
        }
        assert_eq!(events::card_play_finished(plays[0]), 0);
        let finished = test_util::finish_combat_for_allocation_probe(epoch)
            .expect("occupied provenance resets into retained storage");
        drop(finished);
        events::run_suspended();
    });
    assert!(counts.all_zero(), "admission/release/reset: {counts:?}");
}

#[test]
fn reduction_admission_failure_and_reset_make_no_allocator_calls() {
    let dir = test_util::unique_dir("retained-reduction-admission");
    events::test_reset();
    events::init(&dir);
    let epoch = events::combat_started("", "");
    let source = events::source_capture(epoch, 1, 5000, "SOURCE", 0, 0, 0);
    let counts = allocation_support::measure_and_drop(|| {
        for instance in 1..=caps::STR_REDUCTIONS as u64 {
            assert_eq!(
                events::power_attached(
                    epoch,
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
                events::power_amount_changed(
                    epoch,
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
            events::power_attached(epoch, 100, "STRENGTH_POWER", 1100, 1, 4, 10, source),
            1
        );
        assert_eq!(
            events::power_amount_changed(epoch, 100, "STRENGTH_POWER", 1100, 1, 4, 10, 9, source),
            0
        );
        let finished = test_util::finish_combat_for_allocation_probe(epoch)
            .expect("occupied reductions reset into retained storage");
        drop(finished);
        events::run_suspended();
    });
    assert!(counts.all_zero(), "reduction admission/reset: {counts:?}");
}
