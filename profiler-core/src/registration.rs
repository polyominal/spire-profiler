//! Composition root for the engine registration: the one module that knows
//! both the FFI's `EngineClass` shape and the concrete panel types, so the
//! engine layer stays free of profiler-specific types. All the per-panel
//! unsafe casts concentrate here.
//!
//! The callbacks dispatch on the instance-state pointer `create` returned
//! (`Box::into_raw` — never null), and the engine-side null guards live at
//! the FFI boundary in `gdext`'s callbacks, so no defensive null checks
//! are needed here.

use std::ffi::c_void;

use crate::engine::gdext::EngineClass;
use crate::engine::object::Object;
use crate::ui::panel::SpireProfilerPanel;
use crate::ui::run_panel::SpireProfilerRunPanel;

pub(crate) fn engine_classes() -> [EngineClass; 2] {
    [
        EngineClass::new(
            "SpireProfilerPanel",
            c"SpireProfilerPanel",
            panel_create,
            panel_free,
            panel_draw,
            panel_refresh,
        ),
        EngineClass::new(
            "SpireProfilerRunPanel",
            c"SpireProfilerRunPanel",
            run_panel_create,
            run_panel_free,
            run_panel_draw,
            run_panel_refresh,
        ),
    ]
}

unsafe fn panel_create(object: Object) -> *mut c_void {
    Box::into_raw(Box::new(SpireProfilerPanel::new(object))).cast()
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
    Box::into_raw(Box::new(SpireProfilerRunPanel::new(object))).cast()
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
