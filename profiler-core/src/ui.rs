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
//!
//! # Allocation scope
//!
//! Snapshot filtering and row construction, `chart_layout::build`,
//! `run_layout::build_run_layout`, and tooltip shaping and placement are
//! computation scopes. The target requires their command, hit-test, row, and
//! text storage to be retained by the owner after setup; output cloning and
//! formatting in those functions count as computation until that storage is
//! supplied. `ui_snapshot_rows` includes its state read and filtered dataset,
//! so its clone path is measured as computation.
//!
//! Panel `refresh` and rebuild paths are mixed. View reads, fingerprints,
//! snapshot construction, and layout work stay in scope; viewport and mouse
//! queries, resource resolution, redraw requests, and draw replay cross the
//! engine adapter boundary. `draw`, `draw_body`, and `draw_overlay` are
//! adapter-facing entrypoints, but any native computation they perform before
//! an engine call remains measured. Panel creation and teardown own the
//! retained buffers and are setup or teardown, not implicit warmups.

pub mod chart_layout;
pub mod palette;
pub(crate) mod panel;
pub(crate) mod panel_body;
pub mod panel_common;
pub mod panel_replay;
pub mod run_layout;
pub(crate) mod run_panel;
pub mod scroll;
pub mod snapshot;
pub mod theme;
pub mod tooltip;
pub mod ui_model;
