use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use profiler_core::abi::*;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);
static PEAK_COUNT: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct CountingAllocator;

// SAFETY: all allocation operations forward unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the allocator contract supplies a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            let count = LIVE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            PEAK_COUNT.fetch_max(count, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: pointer and layout came from this allocator unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[derive(Debug)]
struct Measure {
    count: usize,
    bytes: usize,
    retained: isize,
    retained_count: isize,
    peak_count: usize,
    peak: usize,
    micros: u128,
}
impl Measure {
    fn run<T>(operation: impl FnOnce() -> T) -> (T, Self) {
        let before_count = ALLOCATIONS.load(Ordering::Relaxed);
        let before_bytes = BYTES.load(Ordering::Relaxed);
        let before_live = LIVE.load(Ordering::Relaxed);
        PEAK.store(before_live, Ordering::Relaxed);
        let before_live_count = LIVE_COUNT.load(Ordering::Relaxed);
        PEAK_COUNT.store(before_live_count, Ordering::Relaxed);
        let start = Instant::now();
        let value = operation();
        (
            value,
            Self {
                retained_count: LIVE_COUNT.load(Ordering::Relaxed) as isize
                    - before_live_count as isize,
                peak_count: PEAK_COUNT
                    .load(Ordering::Relaxed)
                    .saturating_sub(before_live_count),
                count: ALLOCATIONS.load(Ordering::Relaxed) - before_count,
                bytes: BYTES.load(Ordering::Relaxed) - before_bytes,
                retained: LIVE.load(Ordering::Relaxed) as isize - before_live as isize,
                peak: PEAK.load(Ordering::Relaxed).saturating_sub(before_live),
                micros: start.elapsed().as_micros(),
            },
        )
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one test keeps the global allocation counter isolated"
)]
fn reproducible_engine_allocation_profile() {
    let mut baseline = None;
    for rows in [32, 256, 500] {
        let (fixture, construction) = Measure::run(|| {
            let engine = spire_profiler_engine_create();
            // SAFETY: literals are immutable terminated buffers.
            unsafe {
                spire_profiler_combat_started(
                    engine,
                    1,
                    c"PROFILE".as_ptr(),
                    c"normal".as_ptr(),
                    1,
                    1,
                );
            }
            let mut first = 0;
            for row in 0..rows {
                let id =
                    CString::new(format!("CARD_{row:04}")).expect("fixture IDs contain no NUL");
                // SAFETY: CString owns the terminated identifier through capture.
                let source = unsafe {
                    spire_profiler_source_capture(engine, 1, 1, row + 1, id.as_ptr(), 0, 0, 0)
                };
                if row == 0 {
                    first = source;
                }
            }
            (engine, first)
        });
        let (engine, source) = fixture;
        let (_, consumption) = Measure::run(|| {
            for _ in 0..1000 {
                let group = spire_profiler_damage_calculation_begin(engine, 1, source, 1, 0, 999);
                assert_eq!(
                    spire_profiler_damage_result_append(engine, group, 7, 5, 2, 0, 4, 0),
                    1
                );
                assert_eq!(spire_profiler_damage_calculation_commit(engine, group), 1);
            }
        });
        eprintln!("rows={rows} construction={construction:?} consumption={consumption:?}");
        assert!(construction.count > 0 && construction.bytes > 0 && construction.retained > 0);
        assert!(construction.peak >= construction.retained as usize);
        assert!(consumption.count > 0 && consumption.bytes > 0 && consumption.micros > 0);
        let costs = (consumption.count, consumption.bytes, consumption.peak);
        if let Some(expected) = baseline {
            assert_eq!(
                costs, expected,
                "single-row updates must not copy untouched rows"
            );
        } else {
            baseline = Some(costs);
        }
        spire_profiler_engine_destroy(engine);
    }
    for (exports, active, sweep) in [
        (1_000, 8, 1),
        (10_000, 32, 1),
        (20_000, 128, 1),
        (10_000, 32, 128),
    ] {
        let ((engine, sources), construction) = Measure::run(|| {
            let engine = spire_profiler_engine_create();
            // SAFETY: immutable literals own all terminated input strings.
            let (a, b) = unsafe {
                spire_profiler_combat_started(
                    engine,
                    1,
                    c"SOURCE_PROFILE".as_ptr(),
                    c"normal".as_ptr(),
                    1,
                    1,
                );
                (
                    spire_profiler_source_capture(engine, 1, 1, 1, c"A".as_ptr(), 0, 0, 0),
                    spire_profiler_source_capture(engine, 1, 1, 2, c"B".as_ptr(), 0, 0, 0),
                )
            };
            let mut sources = std::collections::VecDeque::with_capacity(active);
            let mut dead = Vec::with_capacity(sweep);
            for index in 0..exports {
                if sources.len() == active {
                    let source = sources.pop_front().expect("active window is full");
                    dead.push(source);
                    if dead.len() == sweep {
                        for source in dead.drain(..) {
                            assert_eq!(spire_profiler_source_release(engine, source), 1);
                        }
                    }
                }
                let source = spire_profiler_source_accumulate(engine, 1, a, 1, b, index + 2);
                assert_ne!(source, 0);
                sources.push_back(source);
            }
            for source in dead {
                assert_eq!(spire_profiler_source_release(engine, source), 1);
            }
            (engine, sources)
        });
        let (_, consumption) = Measure::run(|| {
            for index in 0..1_000 {
                let source = sources[index % active];
                let group = spire_profiler_damage_calculation_begin(engine, 1, source, 1, 0, 99);
                assert_eq!(
                    spire_profiler_damage_result_append(engine, group, 7, 5, 2, 0, 4, 0),
                    1
                );
                assert_eq!(spire_profiler_damage_calculation_commit(engine, group), 1);
            }
        });
        eprintln!(
            "source exports={exports} active={active} sweep={sweep} construction={construction:?} consumption={consumption:?}"
        );
        assert!(
            construction.retained_count > 0
                && construction.peak_count >= construction.retained_count as usize
        );
        spire_profiler_engine_destroy(engine);
    }
    let engine = spire_profiler_engine_create();
    let (_, modifier_credits) = Measure::run(|| {
        for _ in 0..10_000 {
            for _ in 0..8 {
                assert_eq!(
                    spire_profiler_modifier_credit(engine, 10, 0, 15, 1_u64 << 48, 20, 0, 2),
                    5
                );
            }
        }
    });
    eprintln!("80k decimal modifier credits={modifier_credits:?}");
    spire_profiler_engine_destroy(engine);
}
