//! Deterministic combat attribution. The managed host owns game observations,
//! run identity, clocks, storage, and Godot objects. Each native engine owns one
//! [`data::state::State`]; source handles never expose weighted provenance.
//! Observations update accounting under a versioned policy. JSON snapshots
//! contain measured totals, assigned source credit, and capture coverage.
//! Only [`abi`] contains unsafe code, for C strings and caller-owned buffers.

#![deny(unsafe_code)]
#![deny(unreachable_pub)]
#![deny(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]
#![deny(clippy::print_stdout, clippy::print_stderr)]

#[allow(unsafe_code)]
pub mod abi;
pub mod data;
mod source_kind;

// The integration tests link the crate as a library (cfg(test) off), so the
// `test-support` feature re-opens the test-only helpers for them.
#[cfg(any(test, feature = "test-support"))]
pub mod test_util;
