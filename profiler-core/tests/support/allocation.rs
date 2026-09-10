//! The test binary's pass-through allocator records calls on the current
//! thread. Measurements take before/after snapshots, so setup allocations are
//! excluded by scope placement rather than by disabling the callbacks. Nested
//! measurements naturally contribute to their enclosing measurement; a
//! returned value is dropped inside `measure_and_drop` when its destruction
//! belongs to the measured event. Counters are modulo `usize::MAX + 1`; each
//! sample must contain fewer than that many calls of any one operation. The
//! explicit callback work has no allocation, formatting, logging, panic, or
//! re-entry through the registered global allocator. TLS runtime work, if
//! needed, may use `System` directly and is outside this counter.

#![deny(unsafe_op_in_unsafe_fn)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts calls made through the test binary's pass-through global allocator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllocationCounts {
    pub alloc: usize,
    pub alloc_zeroed: usize,
    pub realloc: usize,
    pub dealloc: usize,
}

impl AllocationCounts {
    pub fn all_zero(self) -> bool {
        self == Self::default()
    }

    fn snapshot() -> Self {
        ACCOUNTING.with(Cell::get)
    }

    fn wrapping_sub(self, start: Self) -> Self {
        Self {
            alloc: self.alloc.wrapping_sub(start.alloc),
            alloc_zeroed: self.alloc_zeroed.wrapping_sub(start.alloc_zeroed),
            realloc: self.realloc.wrapping_sub(start.realloc),
            dealloc: self.dealloc.wrapping_sub(start.dealloc),
        }
    }
}

struct CountingAllocator;

#[derive(Clone, Copy)]
enum Operation {
    Alloc,
    AllocZeroed,
    Realloc,
    Dealloc,
}

thread_local! {
    static ACCOUNTING: Cell<AllocationCounts> = const { Cell::new(AllocationCounts {
        alloc: 0,
        alloc_zeroed: 0,
        realloc: 0,
        dealloc: 0,
    }) };
}

impl Operation {
    fn record(self) {
        let _ = ACCOUNTING.try_with(|cell| {
            let mut counts = cell.get();
            match self {
                Self::Alloc => counts.alloc = counts.alloc.wrapping_add(1),
                Self::AllocZeroed => counts.alloc_zeroed = counts.alloc_zeroed.wrapping_add(1),
                Self::Realloc => counts.realloc = counts.realloc.wrapping_add(1),
                Self::Dealloc => counts.dealloc = counts.dealloc.wrapping_add(1),
            }
            cell.set(counts);
        });
    }
}

// SAFETY:
// The explicit counter update performs no allocation, formatting, logging, or
// panic and cannot re-enter this registered allocator. `try_with` accesses the
// const/no-drop TLS cell; if TLS runtime initialization is needed, the pinned
// `LocalKey` contract permits it through `System`, outside this counter. Each
// method forwards the same allocator-valid arguments to `System`: allocation
// methods return its pointer or null result unchanged, and deallocation
// preserves its ownership pair. The `GlobalAlloc` caller contract supplies
// layout validity and allocation/deallocation pairing.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Operation::Alloc.record();
        // SAFETY: The `GlobalAlloc` caller supplies a valid nonzero layout.
        // This wrapper forwards it unchanged to `System`, whose pointer or
        // null result is returned unchanged and remains System-owned.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Operation::AllocZeroed.record();
        // SAFETY: The `GlobalAlloc` caller supplies a valid nonzero layout.
        // This wrapper forwards it unchanged to `System`, whose pointer or
        // null result is returned unchanged and remains System-owned.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Operation::Realloc.record();
        // SAFETY: The `GlobalAlloc` caller supplies the live pointer and exact
        // layout from this allocator. Every allocation path here delegates
        // unchanged to `System`, so that pointer is System-owned. The caller
        // also supplies positive `new_size` with a rounded size no greater
        // than `isize::MAX`; both result and null ownership are unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        Operation::Dealloc.record();
        // SAFETY: The `GlobalAlloc` caller supplies the matching live pointer
        // and exact layout from this allocator. All wrapper allocation paths
        // delegate unchanged to `System`, so this deallocation preserves its
        // ownership pair.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

/// Measures a closure using before/after snapshots on the calling thread.
/// A panic leaves no harness state to restore. Allocations made while
/// destroying the returned value happen after this function returns and are
/// therefore outside its measurement.
#[allow(dead_code)] // some focused binaries only need the zero-check helper
pub fn measure<T>(operation: impl FnOnce() -> T) -> (T, AllocationCounts) {
    let start = AllocationCounts::snapshot();
    let value = operation();
    let counts = AllocationCounts::snapshot().wrapping_sub(start);
    (value, counts)
}

/// Measures a closure, dropping its return value before taking the snapshot.
/// This is the form to use when return-value destruction belongs to the event.
/// A panic propagates after no measurement state needs to be restored.
pub fn measure_and_drop<T>(operation: impl FnOnce() -> T) -> AllocationCounts {
    let start = AllocationCounts::snapshot();
    let value = operation();
    drop(value);
    AllocationCounts::snapshot().wrapping_sub(start)
}

/// Exercises every direct `GlobalAlloc` operation with live memory and paired
/// destruction, so optimization cannot discard an unused allocation result.
#[allow(dead_code)] // calibration runs in the dedicated baseline binary
pub fn calibration_counts() -> AllocationCounts {
    measure_and_drop(|| {
        // The two layouts are nonzero (64 and 128 bytes), use the same u64
        // alignment, and their rounded sizes are far below `isize::MAX`.
        let layout = Layout::array::<u64>(8).expect("calibration layout is representable");
        let grown_layout = Layout::array::<u64>(16).expect("grown layout is representable");
        // SAFETY:
        // `layout` and `grown_layout` are valid nonzero layouts with identical
        // alignment and rounded sizes below `isize::MAX`. `ptr` comes from the
        // preceding `GLOBAL.alloc` delegation to `System` and remains live
        // with `layout`; its `realloc` new size is the positive 128-byte
        // `grown_layout.size()`. A null result is checked before access and
        // retains the old allocation under `GlobalAlloc`'s contract; a
        // successful result is deallocated with the exact layout used by that
        // result. The zeroed pointer is likewise checked and deallocated with
        // `layout`.
        unsafe {
            let ptr = GLOBAL.alloc(layout);
            assert!(!ptr.is_null(), "calibration allocation must succeed");
            std::ptr::write_bytes(ptr, 0xA5, layout.size());
            std::hint::black_box(ptr);

            let ptr = GLOBAL.realloc(ptr, layout, grown_layout.size());
            assert!(!ptr.is_null(), "calibration reallocation must succeed");
            std::ptr::write_bytes(ptr.add(layout.size()), 0x5A, layout.size());
            std::hint::black_box(ptr);
            GLOBAL.dealloc(ptr, grown_layout);

            let zeroed = GLOBAL.alloc_zeroed(layout);
            assert!(
                !zeroed.is_null(),
                "calibration zeroed allocation must succeed"
            );
            let value = std::ptr::read(zeroed.cast::<u64>());
            assert_eq!(
                value, 0,
                "alloc_zeroed must initialize the calibration word"
            );
            std::hint::black_box(value);
            GLOBAL.dealloc(zeroed, layout);
        }
    })
}

/// A safe return value whose destructor performs the direct calibration.
#[allow(dead_code)] // only the baseline binary constructs this probe
pub struct CalibrationOnDrop;

impl Drop for CalibrationOnDrop {
    fn drop(&mut self) {
        let _ = calibration_counts();
    }
}
