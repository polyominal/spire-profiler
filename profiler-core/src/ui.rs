//! The UI layer — the GDExtension panels and their shared plumbing.
//!
//! Pure/impure split: [`panel`], [`run_panel`], [`panel_body`],
//! [`panel_common`], [`panel_replay`], [`tooltip`], and [`theme`] touch
//! the FFI (through the safe [`crate::engine::object::Object`] newtype
//! and the engine layer's loaders); [`chart_layout`], [`palette`],
//! [`run_layout`], [`scroll`], [`snapshot`], and [`ui_model`] never do
//! and stay unit-testable without an engine (the tooltip's
//! shaping/placement is pure too — only its [`tooltip::draw`] rides the
//! FFI).

pub mod chart_layout;
pub mod palette;
pub mod panel;
pub(crate) mod panel_body;
pub mod panel_common;
pub mod panel_replay;
pub mod run_layout;
pub mod run_panel;
pub mod scroll;
pub mod snapshot;
pub mod theme;
pub mod tooltip;
pub mod ui_model;
