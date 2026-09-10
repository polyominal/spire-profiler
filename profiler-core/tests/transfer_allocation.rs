#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::events;
use profiler_core::data::state::caps;
use profiler_core::test_util;

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "measure first use, saturation, rejection, and reset in one initialized owner"
)]
fn transfer_capture_saturation_and_epoch_reset_reuse_initialized_storage() {
    let directory = test_util::unique_dir("retained-transfers");
    let ids: Vec<_> = (0..=caps::SOURCE_DESTINATIONS)
        .map(|index| format!("ROOT_{index}"))
        .collect();
    events::test_reset();
    events::init(&directory);
    let epoch = events::combat_started("TRANSFERS", "Monster");
    let (destinations, captured) = allocation_support::measure(|| {
        let mut destinations = [0; caps::SOURCE_DESTINATIONS + 1];
        for (index, id) in ids.iter().enumerate() {
            let transfer = events::source_capture(epoch, 1, index as u64 + 1, id, 0, 2, 0);
            assert_ne!(transfer, 0);
            destinations[index] = events::source_destination(transfer, 0);
            assert_eq!(events::source_transfer_release(transfer), 1);
        }
        destinations
    });
    assert!(
        captured.all_zero(),
        "first capture and release: {captured:?}"
    );

    for _ in 0..3 {
        let full = allocation_support::measure_and_drop(|| {
            let mut leases = [0; caps::SOURCE_TRANSFERS];
            for lease in &mut leases {
                *lease = events::source_transfer_begin(epoch);
                assert_ne!(*lease, 0);
                for destination in &destinations[..caps::SOURCE_DESTINATIONS] {
                    assert_eq!(events::source_transfer_add(*lease, *destination, 2), 1);
                }
                assert_eq!(events::source_transfer_add(*lease, destinations[0], 2), 1);
                assert_eq!(events::source_transfer_seal(*lease), 1);
                assert_eq!(
                    events::source_count(*lease),
                    caps::SOURCE_DESTINATIONS as i32
                );
                assert_eq!(events::source_weight(*lease, 0), 2);
                assert_eq!(events::source_weight(*lease, 1), 1);
            }
            assert_eq!(events::source_transfer_begin(epoch), 0);
            let stale = leases[0];
            assert_eq!(events::source_transfer_release(stale), 1);
            leases[0] = events::source_transfer_begin(epoch);
            assert_ne!(leases[0], 0);
            assert_ne!(leases[0], stale);
            assert_eq!(events::source_count(stale), -1);
            for destination in &destinations[..caps::SOURCE_DESTINATIONS] {
                assert_eq!(events::source_transfer_add(leases[0], *destination, 1), 1);
            }
            assert_eq!(
                events::source_transfer_add(leases[0], destinations[caps::SOURCE_DESTINATIONS], 1),
                0
            );
            assert_eq!(events::source_count(leases[0]), -1);
            assert_eq!(events::source_transfer_seal(leases[0]), 0);
            for lease in leases {
                assert_eq!(events::source_transfer_release(lease), 1);
            }
        });
        assert!(
            full.all_zero(),
            "transfer saturation and recovery: {full:?}"
        );
    }

    let transfer = events::source_transfer_begin(epoch);
    for destination in &destinations[..caps::SOURCE_DESTINATIONS] {
        assert_eq!(events::source_transfer_add(transfer, *destination, 1), 1);
    }
    assert_eq!(events::source_transfer_seal(transfer), 1);
    let reset = allocation_support::measure_and_drop(|| {
        let finished = test_util::finish_combat_for_allocation_probe(epoch)
            .expect("populated transfer storage does not prevent combat finishing");
        drop(finished);
        let next_epoch = events::combat_started("NEXT", "Monster");
        assert_ne!(next_epoch, epoch);
        assert_eq!(events::source_count(transfer), -1);
        let unknown = events::source_capture(next_epoch, 0, 0, "", 0, 4, 0);
        assert_ne!(unknown, 0);
        assert_eq!(events::source_count(unknown), 1);
        assert_eq!(events::source_transfer_release(unknown), 1);
    });
    assert!(reset.all_zero(), "populated transfer reset: {reset:?}");
}
