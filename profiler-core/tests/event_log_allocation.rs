use std::path::Path;

use profiler_core::test_util::{bind_event_log_probe, emit_event_log_probe};

#[path = "support/allocation.rs"]
mod allocation_support;

#[test]
fn event_log_does_not_allocate_after_sink_warmup() {
    let dir = profiler_core::test_util::unique_dir("event-log-allocation");
    let path = dir.join("profiler.log");
    let long = "A".repeat(4096);
    bind_event_log_probe(&path);

    emit_event_log_probe(format_args!("warm {long}"));
    let counts = allocation_support::measure_and_drop(|| {
        emit_event_log_probe(format_args!("literal"));
        emit_event_log_probe(format_args!("integer {}", i64::MAX));
        emit_event_log_probe(format_args!(
            "path {}",
            Path::new("/spire-profiler/event-log").display()
        ));
        emit_event_log_probe(format_args!("truncated {long}"));
    });

    if !counts.all_zero() {
        eprintln!("event log observed {counts:?}");
        std::process::exit(1);
    }
}
