//! Combat facts and immutable supplier provenance. Runtime producer, credited
//! destination, and damage segment are independent. Supplier identity includes
//! its player slot and survives generation, application, and delayed effects.
//! [`state`] defines player ownership; [`persistence`] defines the disk schema.

pub mod events;
pub mod ledger;
pub mod persistence;
pub mod records;
pub mod run_history;
mod source;
pub mod state;

mod text;
