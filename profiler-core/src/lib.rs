//! Spire Profiler — the native core of a per-source combat profiler for
//! Slay the Spire 2.
//!
//! The game's mod loader accepts only .NET assemblies, so a generated C#
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
//! and the player-slot model in [`data::state`]'s.
//!
//! # Layers
//!
//!   * [`abi`] — the `spire_profiler_*` C export surface; one of the three unsafe relaxations of
//!     the crate-root deny
//!   * [`registration`] — the composition root: the two panel classes and their per-panel casts;
//!     the second relaxation
//!   * [`data`] — combat facts and the persisted JSON model, engine-free
//!   * [`engine`] — the hand-rolled GDExtension FFI (the third relaxation) and the local
//!     Vector2/Rect2/Color stand-ins
//!   * [`ui`] — the two panels and their shared plumbing
//!
//! # Standing contracts
//!
//! The game must never crash because of the mod: every export routes
//! through `contain`, which catches a panic and logs it, and wire values
//! clamp-and-log instead of panicking. All game state lives in one
//! thread-local `RefCell<State>` because the game's logic loop is
//! single-threaded. Unsafe Rust is quarantined in the three modules above,
//! each with its reason documented. Specs live in the module docs, not in
//! `docs/`; environment content (toolchain, headless testing, platform
//! layout) lives in `docs/pitfalls.md`. A self-test entry point lets the
//! host verify the bridge end-to-end under the headless gate.
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
// Module docs are spec documentation, so a broken intra-doc link is a doc
// bug: fail the build rather than warn. Private links stay allowed — the
// crate is not a public library and deliberately links `pub(crate)` items,
// which only resolve under `--document-private-items`.
#![deny(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]

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
pub mod registration;
pub mod source_kind;
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
