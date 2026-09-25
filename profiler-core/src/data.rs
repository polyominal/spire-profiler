//! Combat observations and immutable supplier provenance. The host owns run
//! aggregation and persistence; the engine owns all source-weight arithmetic.

#[cfg(any(test, feature = "test-support"))]
pub mod events;
pub mod ledger;
pub(crate) mod modifiers;
pub(crate) mod observation;
mod source;
pub mod state;
mod summary;
