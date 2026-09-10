//! Spire Profiler — the native core of a per-source combat profiler for
//! Slay the Spire 2.
//!
//! The game's mod loader accepts only .NET assemblies, so a C#
//! shim owns the entry point and the Harmony patches and stays dumb: every
//! line of profiler logic lives in this crate. The shim forwards game
//! events across the `spire_profiler_*` C exports; the core records,
//! attributes, persists, and renders them.
//!
//! # What it records
//!
//! During every combat the mod tracks what each card and source
//! contributes — damage (direct/attributed/modifier splits, plus
//! self-damage), defense (block gained/effective, debuff/buff/strength
//! mitigation, Osty), and forge — rolls the numbers up per run, renders
//! them in two in-game panels (the combat chart and the run-history
//! summary, both game-native), and persists the results as JSON under
//! `<mod_data>/spire-profiler/`. The attribution model lives in
//! [`data`]'s module doc, the on-disk schema in [`data::persistence`]'s,
//! and live-state ownership and the player-slot model in [`data::state`]'s.
//!
//! # Standing contracts
//!
//! The game must never crash because of the mod: every export routes
//! through [`crate::abi::contain`], which catches a panic and logs it,
//! and wire values clamp-and-log instead of panicking. Unsafe Rust is
//! quarantined in exactly [`abi`], [`registration`], and [`engine::gdext`],
//! each with its reason documented.
//! Specs live in the module docs, not in
//! `docs/`; environment content (building, verification, GDExtension
//! interop, platform layout) lives in the `docs/` guides. A self-test entry point lets the
//! host verify the bridge end-to-end under the headless gate.
//!
//! # Allocation contract
//!
//! The narrower target is allocation-free native computation after explicit
//! initialization. Its scope is Rust heap activity in attribution, state
//! mutation, retained-state reset, snapshots, pure history matching and
//! roll-ups, layout construction, and tooltip shaping. A computation prefix
//! stays in scope when its public event later invokes a persistence or engine
//! adapter. Production code allocates in these paths; this section defines the
//! scope of the claim and its evidence.
//!
//! Initialization is an explicit setup interval for each lifetime owner.
//! [`data::events::init`] establishes profiler state and storage paths;
//! [`engine::gdext::gdextension_entry`] resolves the extension, and the
//! registration constructors establish panel-owned state. Heap allocation and
//! fallible capacity reservation are allowed during setup. A measured
//! operation begins only after setup returns. There are no implicit warmups:
//! first use after setup is part of the measured operation. New combats and
//! runs, and a repeated `init` call, do not open another gameplay setup
//! interval.
//!
//! The target contract gives every retained table a named cardinality and byte
//! bound. The existing `caps` are cardinality bounds; they do not by
//! themselves establish byte bounds or zero allocator calls. Admission checks
//! a cap before writing; an overflow returns the operation's failure value and
//! reports it. Under this target, a reset qualifies as zero-allocation only
//! when it clears logical occupancy and recycles capacity. It does not replace
//! an owner or free nested storage. Deallocation belongs to final owner
//! teardown, outside a computation scope.
//!
//! An allocation check observes the four [`std::alloc::GlobalAlloc`]
//! operations: `alloc`, `alloc_zeroed`, `realloc`, and `dealloc`. A
//! zero-operation assertion passes only when the count delta for all four is
//! zero. The counter records calls routed through the registered allocator and
//! remains active over nested Rust calls and expected failure paths on the
//! calling thread; work on spawned threads is outside that sample. TLS
//! runtime initialization may use `System` directly on some targets, so that
//! runtime work is outside the counter even on a measured thread. Fixture
//! construction, setup, result formatting, and process teardown are outside a
//! computation zero-operation delta, though setup and adapter samples may be
//! reported separately. The baseline accounting test records all four
//! operations for named lifecycle, attribution, UI, persistence, and history
//! samples; its nonzero gameplay counts are measurements, not zero-operation
//! assertions. Mixed lifecycle rows are whole-call measurements; pure finish
//! and merge samples do not imply that a resume rebuild is physically split.
//! Sink probes assert zero only for their warmed diagnostic paths.
//!
//! Expected failure means a bounded result such as a rejected wire packet,
//! stale token, full table, arithmetic overflow, malformed input, or failed
//! adapter call. It preserves the state contract and never requires a panic.
//! A panic is unexpected control flow, not an allowed no-allocation result.
//! [`abi::contain`] catches it before the host boundary, but recovery and panic
//! payload handling are outside this computation guarantee; allocator methods
//! themselves never log or panic.
//!
//! Persistence and engine adapters are explicit exceptions described in
//! [`data::persistence`] and [`engine`]. Their owned serialization,
//! filesystem, and host-call spans must be separated from a computation
//! zero check. Computation before, inside, or after an adapter wrapper, such as
//! run merging before a combat write, stays in scope. A zero assertion names a
//! separated computation span and cannot cover a whole mixed event merely
//! because it invokes I/O. This is a measurement requirement, not a claim
//! that every mixed event has physical split instrumentation. No guarantee
//! covers allocations made by Godot, the filesystem, foreign allocators, or
//! panic-recovery runtime code. Rust standard-library allocations used by
//! in-scope computation are counted.
//!
//! Console diagnostics stream `fmt::Arguments` into stderr and are
//! allocation-free after the first stderr lock. They are health reports for
//! the person running the game, not gameplay records. OS errors print kind
//! and raw code instead of `strerror`; valid UTF-8 paths are the tested
//! path profile.
//! TODO: stream non-UTF-8 paths lossily if diagnostics ever need the
//! allocation guarantee on every host path.
//!
//! Logging has two deliberate outputs: the stderr diagnostics above and the
//! unlevelled gameplay event trace in `profiler.log`. A record goes to
//! exactly one output; the event trace may report sink failure through a
//! stderr diagnostic, but diagnostics are never copied into the file.

#![deny(unsafe_code)]
#![deny(unreachable_pub)]
// Module docs are spec documentation, so a broken intra-doc link is a doc
// bug: fail the build rather than warn. Private links stay allowed — the
// crate is not a public library and deliberately links `pub(crate)` items,
// which only resolve under `--document-private-items`.
#![deny(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]
// `emit` below is the single sanctioned console writer; the deny keeps every
// other path off the player's terminal.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::cell::Cell;
use std::fmt;
use std::io::{self, Write};

// The relaxations of the crate-root deny. Unsafe Rust is quarantined in
// three places: the C ABI surface (raw C pointer reads, no_mangle extern
// fns, and the catch_unwind panic-containment contract — see abi.rs's
// header), the registration layer, which owns the per-panel instance casts
// the FFI callbacks route into, and gdext.rs, which carries its own allow
// inside engine.rs. Keep any future unsafe requirement behind a safe helper
// in one of these.
#[allow(unsafe_code)]
pub mod abi;
pub mod data;
pub mod engine;
#[allow(unsafe_code)]
mod registration;
mod source_kind;
pub mod ui;

// The integration tests link the crate as a library (cfg(test) off), so the
// `test-support` feature re-opens the test-only helpers for them.
#[cfg(any(test, feature = "test-support"))]
pub mod test_util;

fn emit(level: &str, args: fmt::Arguments<'_>) {
    let mut stderr = io::stderr().lock();
    // A diagnostic sink must not turn a broken game-side pipe into a panic
    // across the C ABI.
    let _ = writeln!(stderr, "[SpireProfiler] {level}: {args}");
}

/// A persistently corrupt wire value is one bug, not one log line per event;
/// the first occurrence still reports.
pub(crate) fn fail_once(
    gate: &'static std::thread::LocalKey<Cell<bool>>,
    args: fmt::Arguments<'_>,
) {
    gate.with(|logged| {
        if !logged.replace(true) {
            emit("ERROR", args);
        }
    });
}

macro_rules! fail_log {
    ($($arg:tt)*) => {
        $crate::emit("ERROR", format_args!($($arg)*))
    };
}

macro_rules! warn_log {
    ($($arg:tt)*) => {
        $crate::emit("WARNING", format_args!($($arg)*))
    };
}

macro_rules! marker_log {
    ($($arg:tt)*) => {
        $crate::emit("INFO", format_args!($($arg)*))
    };
}

pub(crate) use fail_log as fail;
pub(crate) use marker_log as marker;
pub(crate) use warn_log as warn;
