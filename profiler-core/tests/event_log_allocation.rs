use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use profiler_core::test_util::{bind_event_log_probe, emit_event_log_probe};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn event_log_does_not_allocate_after_sink_warmup() {
    let dir = profiler_core::test_util::unique_dir("event-log-allocation");
    let path = dir.join("profiler.log");
    let long = "A".repeat(4096);
    bind_event_log_probe(&path);

    emit_event_log_probe(format_args!("warm {long}"));
    ALLOCATIONS.store(0, Ordering::Relaxed);
    emit_event_log_probe(format_args!("literal"));
    emit_event_log_probe(format_args!("integer {}", i64::MAX));
    emit_event_log_probe(format_args!(
        "path {}",
        Path::new("/spire-profiler/event-log").display()
    ));
    emit_event_log_probe(format_args!("truncated {long}"));

    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    if allocations != 0 {
        eprintln!("event log allocated {allocations} time(s)");
        std::process::exit(1);
    }
}
