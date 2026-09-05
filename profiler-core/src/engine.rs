//! The engine-facing layer — the only modules that know the engine exists.
//!
//!   * [`gdext`] — the hand-rolled GDExtension FFI over the vendored Godot 4.5.1 header
//!     (`vendor/gdextension_interface.h`); home of the third unsafe relaxation
//!   * [`math`] — local Vector2/Rect2/Color stand-ins for the Godot value types the panel plumbing
//!     needs
//!   * [`object`] — safe method dispatch on engine [`object::Object`] pointers (unsafe-free)

// The third relaxation of the crate-root deny (after abi.rs and
// registration.rs): the hand-rolled GDExtension FFI resolves engine function
// pointers by name and dereferences raw engine pointers (see gdext.rs's
// header for the full unsafety contract).
#[allow(unsafe_code)]
pub mod gdext;
pub mod math;
pub mod object;
