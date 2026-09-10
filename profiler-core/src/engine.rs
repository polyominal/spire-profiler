//! The engine-facing layer — the only modules that know the engine exists.
//!
//!   * [`gdext`] — the hand-rolled GDExtension FFI over the vendored Godot 4.5.1 header
//!     (`vendor/gdextension_interface.h`); home of the third unsafe relaxation
//!   * [`math`] — local Vector2/Rect2/Color stand-ins for the Godot value types the panel plumbing
//!     needs
//!   * [`object`] — safe method dispatch on engine [`object::Object`] pointers (unsafe-free)
//!
//! # Allocation boundary
//!
//! `gdext` owns the engine adapter: `gdextension_entry`, initialization and
//! deinitialization callbacks, StringName setup, class registration and
//! instantiation, and the `Variant` and `RetainedVariant` wrappers. The
//! object adapter owns `Object` calls, resource loading, style-box
//! construction, drawing, viewport and mouse queries. Godot and its native
//! libraries may allocate across every one of those calls.
//!
//! The Rust wrappers also allocate: `Variant::nil`, typed Variant
//! constructors, `string_variant`, and instance boxes use `Box` or
//! `CString`; retained resources and their teardown own those allocations.
//! These operations are explicit engine-boundary work. Native snapshot,
//! layout, and tooltip computation that prepares their arguments remains in
//! scope and cannot be hidden by wrapping a whole panel callback.

// The third relaxation of the crate-root deny (after abi.rs and
// registration.rs): the hand-rolled GDExtension FFI resolves engine function
// pointers by name and dereferences raw engine pointers (see gdext.rs's
// header for the full unsafety contract).
#[allow(unsafe_code)]
pub mod gdext;
pub mod math;
pub mod object;
