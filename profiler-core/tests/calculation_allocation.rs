#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::events;
use profiler_core::data::state::caps;
use profiler_core::test_util;

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One initialized owner spans setup, maximum occupancy, and measured reuse"
)]
fn calculation_capture_and_release_reuse_maximum_fanout_storage() {
    let dir = test_util::unique_dir("calculation-allocation");
    events::test_reset();
    events::init(&dir);
    let epoch = events::combat_started("CALCULATIONS", "Monster");
    let mut destinations = Vec::with_capacity(caps::SOURCE_DESTINATIONS);
    for index in 0..caps::SOURCE_DESTINATIONS {
        let source = events::source_capture(
            epoch,
            1,
            index as u64 + 1,
            &format!("SOURCE_{index}"),
            0,
            0,
            0,
        );
        assert_ne!(source, 0);
        destinations.push(events::source_destination(source, 0));
        assert_eq!(events::source_transfer_release(source), 1);
    }
    let source = events::source_transfer_begin(epoch);
    assert_ne!(source, 0);
    for destination in destinations {
        assert_eq!(events::source_transfer_add(source, destination, 1), 1);
    }
    assert_eq!(events::source_transfer_seal(source), 1);
    for index in 0..caps::STR_REDUCTIONS {
        let power = index as u64 + 1;
        assert_eq!(
            events::power_attached(epoch, power, "STRENGTH_POWER", 800, 1, 4, 0, source),
            1
        );
        assert_eq!(
            events::power_amount_changed(epoch, power, "STRENGTH_POWER", 800, 1, 4, 0, -1, source),
            1
        );
    }
    let mut calculations = Vec::with_capacity(caps::DAMAGE_CALCULATIONS);
    for transfer in [source, 0, source] {
        let counts = allocation_support::measure_and_drop(|| {
            for _ in 0..caps::DAMAGE_CALCULATIONS {
                let calculation = events::damage_calculation_begin(epoch, transfer, 1, 0, 800);
                assert_ne!(calculation, 0);
                calculations.push(calculation);
                for _ in 0..caps::DAMAGE_MODIFIERS {
                    assert_eq!(
                        events::damage_modifier_contribution(calculation, transfer, 1),
                        1
                    );
                }
                assert_eq!(
                    events::damage_calculation_weak_source(calculation, transfer),
                    1
                );
                assert_eq!(
                    events::damage_calculation_enemy_hit(calculation, 800, 100, -64),
                    1
                );
                for _ in 0..caps::DAMAGE_RESULTS {
                    assert_eq!(
                        events::damage_result_append(calculation, 1, 1, 0, 1, 0, 0),
                        1
                    );
                }
            }
            assert_eq!(
                events::damage_calculation_begin(epoch, transfer, 1, 0, 800),
                0
            );
            assert_eq!(
                events::damage_modifier_contribution(calculations[0], transfer, 1),
                0
            );
            assert_eq!(
                events::damage_result_append(calculations[1], 1, 1, 0, 1, 0, 0),
                0
            );
            for calculation in calculations.drain(..) {
                assert_eq!(events::damage_calculation_abort(calculation), 1);
                assert_eq!(events::damage_calculation_abort(calculation), 0);
            }
        });
        assert!(
            counts.all_zero(),
            "capture and release with transfer {transfer}: {counts:?}"
        );
    }
    assert_eq!(events::source_transfer_release(source), 1);
}
