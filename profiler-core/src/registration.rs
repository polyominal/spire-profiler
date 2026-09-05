//! Composition root for the engine registration: the one module that knows
//! both the FFI's [`EngineClass`] shape and the concrete panel types, so the
//! engine layer stays free of profiler-specific types. All instance casts
//! concentrate here.
//!
//! The callbacks dispatch on the instance-state pointer `create` returned
//! (`Box::into_raw` — never null), and the engine-side null guards live at
//! the FFI boundary in [`crate::engine::gdext`]'s callbacks, so no defensive
//! null checks are needed here.

use std::ffi::c_void;

use crate::engine::gdext::EngineClass;
use crate::engine::object::Object;
use crate::ui::panel::SpireProfilerPanel;
use crate::ui::panel_body::{self, ChildRole, PanelBody};
use crate::ui::run_panel::SpireProfilerRunPanel;

pub(crate) fn engine_classes() -> [EngineClass; 3] {
    [
        EngineClass::new(
            c"SpireProfilerPanel",
            panel_create,
            panel_free,
            panel_draw,
            Some(panel_refresh),
        ),
        EngineClass::new(
            c"SpireProfilerRunPanel",
            run_panel_create,
            run_panel_free,
            run_panel_draw,
            Some(run_panel_refresh),
        ),
        EngineClass::new(
            panel_body::CLASS_NAME,
            body_create,
            body_free,
            body_draw,
            None,
        ),
    ]
}

/// # Safety
/// The engine passes a live `object`; the returned boxed state stays paired
/// with this class's callbacks until free.
unsafe fn panel_create(object: Object) -> *mut c_void {
    let panel = Box::into_raw(Box::new(SpireProfilerPanel::new(object)));
    // Safety: the boxed state address is stable until `panel_free`; the
    // children validate their owner tokens before dispatching back here.
    unsafe { (*panel).attach_children() };
    panel.cast()
}

/// The retained font storage is freed without destroying the engine Ref.
///
/// # Safety
/// `state` is the matching create pointer and has not been freed.
unsafe fn panel_free(state: *mut c_void) {
    // SAFETY: state is the create-owned pointer reconstructed exactly once.
    drop(unsafe { Box::from_raw(state.cast::<SpireProfilerPanel>()) });
}

/// # Safety
/// `state` is the live matching create pointer.
unsafe fn panel_draw(state: *mut c_void) {
    // SAFETY: the engine runs this callback only while the object lives, so
    // the create-owned box is not yet freed.
    unsafe { (*state.cast::<SpireProfilerPanel>()).draw() };
}

/// # Safety
/// `state` is the live matching create pointer.
unsafe fn panel_refresh(state: *mut c_void) {
    // SAFETY: the engine runs this callback only while the object lives, so
    // the create-owned box is not yet freed.
    unsafe { (*state.cast::<SpireProfilerPanel>()).refresh() };
}

/// # Safety
/// The engine passes a live `object`; the returned boxed state stays paired
/// with this class's callbacks until free.
unsafe fn run_panel_create(object: Object) -> *mut c_void {
    let panel = Box::into_raw(Box::new(SpireProfilerRunPanel::new(object)));
    // Safety: the pointer names boxed run-panel state valid until
    // `run_panel_free`; child callbacks validate their owner token first.
    unsafe { (*panel).attach_children() };
    panel.cast()
}

/// # Safety
/// `state` is the matching create pointer and has not been freed.
unsafe fn run_panel_free(state: *mut c_void) {
    // SAFETY: state is the create-owned pointer reconstructed exactly once.
    drop(unsafe { Box::from_raw(state.cast::<SpireProfilerRunPanel>()) });
}

/// # Safety
/// `state` is the live matching create pointer.
unsafe fn run_panel_draw(state: *mut c_void) {
    // SAFETY: the engine runs this callback only while the object lives, so
    // the create-owned box is not yet freed.
    unsafe { (*state.cast::<SpireProfilerRunPanel>()).draw() };
}

/// # Safety
/// `state` is the live matching create pointer.
unsafe fn run_panel_refresh(state: *mut c_void) {
    // SAFETY: the engine runs this callback only while the object lives, so
    // the create-owned box is not yet freed.
    unsafe { (*state.cast::<SpireProfilerRunPanel>()).refresh() };
}

/// A child instantiated outside its panel (the class name is public
/// ClassDB surface) stays valid but draws nothing — degrade, never a
/// failed engine create.
///
/// # Safety
/// The engine passes a live `object`; the returned boxed state stays paired
/// with this class's callbacks until free.
unsafe fn body_create(object: Object) -> *mut c_void {
    let target = panel_body::take_pending_child();
    if target.is_none() {
        crate::warn!("panel body instantiated outside its panel; drawing disabled");
    }
    Box::into_raw(Box::new(PanelBody::new(object, target))).cast()
}

/// # Safety
/// `state` is the matching create pointer and has not been freed.
unsafe fn body_free(state: *mut c_void) {
    // SAFETY: state is the create-owned pointer reconstructed exactly once.
    drop(unsafe { Box::from_raw(state.cast::<PanelBody>()) });
}

/// # Safety
/// `state` is the live body-create pointer; the owner is generation-checked
/// before it is dereferenced.
unsafe fn body_draw(state: *mut c_void) {
    // SAFETY: state is the live body-create pointer.
    let body = unsafe { &*state.cast::<PanelBody>() };
    let Some(target) = body.live_target() else {
        return;
    };
    let object = body.object();
    match (target.owner_ref(), target.role()) {
        // SAFETY: the owner generation was checked before this dispatch.
        (panel_body::OwnerRef::Combat(panel), ChildRole::Rows) => unsafe {
            (*panel).draw_body(object)
        },
        // SAFETY: the owner generation was checked before this dispatch.
        (panel_body::OwnerRef::Combat(panel), ChildRole::Overlay) => unsafe {
            (*panel).draw_overlay(object)
        },
        // SAFETY: the owner generation was checked before this dispatch.
        (panel_body::OwnerRef::Run(panel), ChildRole::Rows) => unsafe {
            (*panel).draw_body(object)
        },
        // SAFETY: the owner generation was checked before this dispatch.
        (panel_body::OwnerRef::Run(panel), ChildRole::Overlay) => unsafe {
            (*panel).draw_overlay(object)
        },
    }
}
