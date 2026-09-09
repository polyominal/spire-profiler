//! Composition root for engine registration and concrete panel dispatch.
//! Instance pointers own boxed handles. Callbacks clone the handle before
//! calling the engine: reentrant free can release the box while the active
//! callback retains its state. Checked borrows reject nested mutation.

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use crate::engine::gdext::EngineClass;
use crate::engine::object::Object;
use crate::ui::panel::SpireProfilerPanel;
use crate::ui::panel_body::{self, PanelBody};
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

struct PanelHandle<T>(Rc<RefCell<T>>);

impl<T> PanelHandle<T> {
    /// # Safety
    /// `state` is this type's live boxed handle on its creation thread. Free
    /// may reenter during `call`, but no callback uses the freed handle again.
    unsafe fn dispatch(state: *mut c_void, call: impl FnOnce(&mut T)) {
        // SAFETY: clone while the handle is live; no reference into its box
        // survives `call`, which can trigger the engine's free callback.
        let panel = unsafe { Rc::clone(&(*state.cast::<Self>()).0) };
        let Ok(mut panel) = panel.try_borrow_mut() else {
            crate::fail!("nested panel callback skipped");
            return;
        };
        call(&mut panel);
    }

    /// # Safety
    /// `state` is this type's boxed handle, freed once on its creation thread.
    unsafe fn free(state: *mut c_void) {
        // SAFETY: consumes the matching Box exactly once. Active dispatches
        // own separate Rc handles and need no borrow of the boxed handle.
        drop(unsafe { Box::from_raw(state.cast::<Self>()) });
    }
}

fn panel_create(object: Object) -> *mut c_void {
    let panel = Rc::new(RefCell::new(SpireProfilerPanel::new(object)));
    panel.borrow_mut().attach_children(Rc::downgrade(&panel));
    Box::into_raw(Box::new(PanelHandle(panel))).cast()
}

/// # Safety
/// `state` is the live combat-panel handle, freed once on its creation thread.
unsafe fn panel_free(state: *mut c_void) {
    // SAFETY: the class table pairs this free with panel_create.
    unsafe { PanelHandle::<SpireProfilerPanel>::free(state) };
}

/// # Safety
/// `state` is the live combat-panel handle on its creation thread.
unsafe fn panel_draw(state: *mut c_void) {
    // SAFETY: the class table pairs this dispatch with panel_create.
    unsafe { PanelHandle::dispatch(state, SpireProfilerPanel::draw) };
}

/// # Safety
/// `state` is the live combat-panel handle on its creation thread.
unsafe fn panel_refresh(state: *mut c_void) {
    // SAFETY: the class table pairs this dispatch with panel_create.
    unsafe { PanelHandle::dispatch(state, SpireProfilerPanel::refresh) };
}

fn run_panel_create(object: Object) -> *mut c_void {
    let panel = Rc::new(RefCell::new(SpireProfilerRunPanel::new(object)));
    panel.borrow_mut().attach_children(Rc::downgrade(&panel));
    Box::into_raw(Box::new(PanelHandle(panel))).cast()
}

/// # Safety
/// `state` is the live run-panel handle, freed once on its creation thread.
unsafe fn run_panel_free(state: *mut c_void) {
    // SAFETY: the class table pairs this free with run_panel_create.
    unsafe { PanelHandle::<SpireProfilerRunPanel>::free(state) };
}

/// # Safety
/// `state` is the live run-panel handle on its creation thread.
unsafe fn run_panel_draw(state: *mut c_void) {
    // SAFETY: the class table pairs this dispatch with run_panel_create.
    unsafe { PanelHandle::dispatch(state, SpireProfilerRunPanel::draw) };
}

/// # Safety
/// `state` is the live run-panel handle on its creation thread.
unsafe fn run_panel_refresh(state: *mut c_void) {
    // SAFETY: the class table pairs this dispatch with run_panel_create.
    unsafe { PanelHandle::dispatch(state, SpireProfilerRunPanel::refresh) };
}

fn body_create(object: Object) -> *mut c_void {
    let target = panel_body::take_pending_child();
    if target.is_none() {
        crate::warn!("panel body instantiated outside its panel; drawing disabled");
    }
    Box::into_raw(Box::new(PanelBody::new(object, target))).cast()
}

/// # Safety
/// `state` is the live body-create pointer, freed once on its creation thread.
unsafe fn body_free(state: *mut c_void) {
    // SAFETY: state is the create-owned pointer reconstructed exactly once.
    drop(unsafe { Box::from_raw(state.cast::<PanelBody>()) });
}

/// # Safety
/// `state` is the live body-create pointer on its creation thread.
unsafe fn body_draw(state: *mut c_void) {
    // SAFETY: copy identity and clone the Weak target before engine calls can
    // reenter body_free. No borrow into the body's allocation survives draw.
    let (object, target) = unsafe {
        let body = &*state.cast::<PanelBody>();
        (body.object(), body.target())
    };
    if let Some(target) = target {
        target.draw(&object);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn nested_dispatch_is_skipped_and_free_waits_for_active_dispatch() {
        struct Probe(Rc<Cell<bool>>, usize);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let dropped = Rc::new(Cell::new(false));
        let panel = Rc::new(RefCell::new(Probe(Rc::clone(&dropped), 0)));
        let weak = Rc::downgrade(&panel);
        let handle = Box::into_raw(Box::new(PanelHandle(panel))).cast();
        // SAFETY: handle is the matching live Box on this thread; nested
        // dispatch precedes its single free, and no use follows the outer call.
        unsafe {
            PanelHandle::dispatch(handle, |probe: &mut Probe| {
                probe.1 += 1;
                PanelHandle::dispatch(handle, |_: &mut Probe| panic!("nested mutation"));
                PanelHandle::<Probe>::free(handle);
                assert!(
                    !dropped.get(),
                    "active callback keeps state alive after free"
                );
                assert!(weak.upgrade().is_some());
                probe.1 += 1;
                assert_eq!(probe.1, 2);
            });
        }
        assert!(
            dropped.get(),
            "state is released when the active callback returns"
        );
        assert!(
            weak.upgrade().is_none(),
            "child owner expires after teardown"
        );
    }
}
