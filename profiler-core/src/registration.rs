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

unsafe fn panel_create(object: Object) -> *mut c_void {
    let panel = Box::into_raw(Box::new(SpireProfilerPanel::new(object)));
    // Safety: the boxed state address is stable until `panel_free`; the
    // children validate their owner tokens before dispatching back here.
    unsafe { (*panel).attach_children() };
    panel.cast()
}

/// The retained font storage is freed without destroying the engine Ref.
unsafe fn panel_free(state: *mut c_void) {
    drop(unsafe { Box::from_raw(state.cast::<SpireProfilerPanel>()) });
}

unsafe fn panel_draw(state: *mut c_void) {
    unsafe { (*state.cast::<SpireProfilerPanel>()).draw() };
}

unsafe fn panel_refresh(state: *mut c_void) {
    unsafe { (*state.cast::<SpireProfilerPanel>()).refresh() };
}

unsafe fn run_panel_create(object: Object) -> *mut c_void {
    let panel = Box::into_raw(Box::new(SpireProfilerRunPanel::new(object)));
    // Safety: the pointer names boxed run-panel state valid until
    // `run_panel_free`; child callbacks validate their owner token first.
    unsafe { (*panel).attach_children() };
    panel.cast()
}

unsafe fn run_panel_free(state: *mut c_void) {
    drop(unsafe { Box::from_raw(state.cast::<SpireProfilerRunPanel>()) });
}

unsafe fn run_panel_draw(state: *mut c_void) {
    unsafe { (*state.cast::<SpireProfilerRunPanel>()).draw() };
}

unsafe fn run_panel_refresh(state: *mut c_void) {
    unsafe { (*state.cast::<SpireProfilerRunPanel>()).refresh() };
}

/// A child instantiated outside its panel (the class name is public
/// ClassDB surface) stays valid but draws nothing — degrade, never a
/// failed engine create.
unsafe fn body_create(object: Object) -> *mut c_void {
    let target = panel_body::take_pending_child();
    if target.is_none() {
        crate::warn!("panel body instantiated outside its panel; drawing disabled");
    }
    Box::into_raw(Box::new(PanelBody::new(object, target))).cast()
}

unsafe fn body_free(state: *mut c_void) {
    drop(unsafe { Box::from_raw(state.cast::<PanelBody>()) });
}

unsafe fn body_draw(state: *mut c_void) {
    let body = unsafe { &*state.cast::<PanelBody>() };
    let Some(target) = body.live_target() else {
        return;
    };
    let object = body.object();
    match (target.owner_ref(), target.role()) {
        (panel_body::OwnerRef::Combat(panel), ChildRole::Rows) => unsafe {
            (*panel).draw_body(object)
        },
        (panel_body::OwnerRef::Combat(panel), ChildRole::Overlay) => unsafe {
            (*panel).draw_overlay(object)
        },
        (panel_body::OwnerRef::Run(panel), ChildRole::Rows) => unsafe {
            (*panel).draw_body(object)
        },
        (panel_body::OwnerRef::Run(panel), ChildRole::Overlay) => unsafe {
            (*panel).draw_overlay(object)
        },
    }
}
