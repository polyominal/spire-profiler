use std::io;
use std::path::Path;

use profiler_core::test_util::emit_allocation_probe;

#[path = "support/allocation.rs"]
mod allocation_support;

#[test]
fn console_diagnostics_do_not_allocate_after_stderr_warmup() {
    // Nextest gives this binary's single test its own process, isolating
    // the global allocator from every other target.
    let path = Path::new("/spire-profiler/allocation-probe");
    let err = io::Error::from_raw_os_error(13);

    emit_allocation_probe("warm stderr", 7, path, &err);
    let counts = allocation_support::measure_and_drop(|| {
        emit_allocation_probe("steady state", 4_294_967_296, path, &err);
    });

    if !counts.all_zero() {
        eprintln!("allocation probe observed {counts:?}");
        std::process::exit(1);
    }
}
