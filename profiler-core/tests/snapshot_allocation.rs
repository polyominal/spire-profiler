#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::events;
use profiler_core::test_util;

#[test]
fn initialized_snapshot_and_prefix_owners_retain_storage_through_failure_and_reuse() {
    let dir = test_util::unique_dir("snapshot-allocation");
    events::test_reset();
    events::init(&dir);
    let mut exercise = test_util::snapshot_allocation_fixture();
    for pass in 0..4 {
        let counts = allocation_support::measure_and_drop(&mut exercise);
        assert!(counts.all_zero(), "snapshot/prefix pass {pass}: {counts:?}");
    }
}
