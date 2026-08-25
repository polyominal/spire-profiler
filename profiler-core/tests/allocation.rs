use std::alloc::{GlobalAlloc, Layout, System};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use profiler_core::test_util::emit_allocation_probe;

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
fn console_diagnostics_do_not_allocate_after_stderr_warmup() {
    // Nextest gives this binary's single test its own process, isolating
    // the global allocator from every other target.
    let path = Path::new("/spire-profiler/allocation-probe");
    let err = io::Error::from_raw_os_error(13);

    emit_allocation_probe("warm stderr", 7, path, &err);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    emit_allocation_probe("steady state", 4_294_967_296, path, &err);

    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    if allocations != 0 {
        eprintln!("allocation probe logged {allocations} allocation(s)");
        std::process::exit(1);
    }
}
