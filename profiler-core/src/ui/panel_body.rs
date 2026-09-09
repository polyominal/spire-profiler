//! The panels' auxiliary children — the third GDExtension class
//! ([`CLASS_NAME`]). Godot draws children after their parent and in child
//! order, so the rows child is added first and clipped to the scroll band;
//! the overlay child is added second and draws scrollbar, legend, and
//! tooltip above the rows.
//!
//! Children hold Weak owner handles and engine object IDs. Draw dispatch
//! upgrades the owner and checks its mutable borrow; a freed owner disables
//! dispatch and a nested callback skips drawing.

use std::cell::RefCell;
use std::ffi::CStr;
use std::rc::Weak;

use crate::engine::gdext::{Object, ObjectId, object_from_id, object_id};
use crate::engine::math::{Rect2, Vector2};
use crate::ui::panel_common;

thread_local! {
    static PENDING_CHILD: RefCell<Option<ChildTarget>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
pub(crate) enum ChildRole {
    Rows,
    Overlay,
}

#[derive(Clone)]
pub(crate) enum OwnerRef {
    Combat(Weak<RefCell<crate::ui::panel::SpireProfilerPanel>>),
    Run(Weak<RefCell<crate::ui::run_panel::SpireProfilerRunPanel>>),
}

#[derive(Clone)]
pub(crate) struct ChildTarget {
    owner: OwnerRef,
    role: ChildRole,
}

impl ChildTarget {
    pub(crate) fn draw(self, object: &Object) {
        match self.owner {
            OwnerRef::Combat(owner) => {
                let Some(owner) = owner.upgrade() else { return };
                let Ok(mut panel) = owner.try_borrow_mut() else {
                    crate::fail!("nested combat child draw skipped");
                    return;
                };
                match self.role {
                    ChildRole::Rows => panel.draw_body(object),
                    ChildRole::Overlay => panel.draw_overlay(object),
                }
            }
            OwnerRef::Run(owner) => {
                let Some(owner) = owner.upgrade() else { return };
                let Ok(mut panel) = owner.try_borrow_mut() else {
                    crate::fail!("nested run child draw skipped");
                    return;
                };
                match self.role {
                    ChildRole::Rows => panel.draw_body(object),
                    ChildRole::Overlay => panel.draw_overlay(object),
                }
            }
        }
    }

    fn spawn(self, parent: Object) -> Option<Object> {
        let role = self.role;
        let pending = PendingChild::install(self)?;
        let child = crate::engine::gdext::instantiate_class(CLASS_NAME);
        drop(pending);
        let Some(child) = child else {
            let layer = match role {
                ChildRole::Rows => "rows",
                ChildRole::Overlay => "overlay",
            };
            crate::fail!("panel body class instantiation failed; {layer} disabled");
            return None;
        };
        if matches!(role, ChildRole::Rows) {
            child.set_clip_contents(true);
        }
        child.set_mouse_filter_ignore();
        parent.add_child(child);
        Some(child)
    }
}

// Engine creation consumes the handoff synchronously. The guard also clears
// it if instantiation fails or unwinds before running the create callback.
struct PendingChild;

impl PendingChild {
    fn install(target: ChildTarget) -> Option<Self> {
        PENDING_CHILD.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                crate::fail!("nested panel child creation before handoff consumed");
                return None;
            }
            *slot = Some(target);
            Some(Self)
        })
    }
}

impl Drop for PendingChild {
    fn drop(&mut self) {
        PENDING_CHILD.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

pub(crate) struct PanelBody {
    object: Object,
    target: Option<ChildTarget>,
}

impl PanelBody {
    pub(crate) fn new(object: Object, target: Option<ChildTarget>) -> Self {
        PanelBody { object, target }
    }

    pub(crate) fn target(&self) -> Option<ChildTarget> {
        self.target.clone()
    }

    pub(crate) fn object(&self) -> Object {
        self.object
    }
}

struct ChildControl {
    id: ObjectId,
    applied_frame: Option<Rect2>,
}

impl ChildControl {
    fn new(object: Object) -> Self {
        ChildControl {
            id: object_id(object),
            applied_frame: None,
        }
    }
    fn object(control: &mut Option<ChildControl>, layer: &'static str) -> Option<Object> {
        let child = control.as_mut()?;
        match object_from_id(child.id) {
            Some(object) => Some(object),
            None => {
                *control = None;
                crate::warn!("panel {layer} child freed; {layer} disabled");
                None
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct PanelChildren {
    rows: Option<ChildControl>,
    overlay: Option<ChildControl>,
}

impl PanelChildren {
    pub(crate) fn attach(parent: Object, owner: OwnerRef) -> Self {
        let rows = ChildTarget {
            owner: owner.clone(),
            role: ChildRole::Rows,
        };
        let overlay = ChildTarget {
            owner,
            role: ChildRole::Overlay,
        };
        PanelChildren {
            rows: rows.spawn(parent).map(ChildControl::new),
            overlay: overlay.spawn(parent).map(ChildControl::new),
        }
    }

    pub(crate) fn objects(&mut self) -> panel_common::ChildObjects {
        panel_common::ChildObjects {
            body: ChildControl::object(&mut self.rows, "rows"),
            overlay: ChildControl::object(&mut self.overlay, "overlay"),
        }
    }

    pub(crate) fn queue_redraw(&mut self) {
        self.objects().queue_redraw();
    }

    pub(crate) fn queue_overlay_redraw(&mut self) {
        if let Some(overlay) = ChildControl::object(&mut self.overlay, "overlay") {
            overlay.queue_redraw();
        }
    }

    pub(crate) fn update_frames(&mut self, parent_frame: Rect2, body_frame: Rect2) {
        Self::update_frame(&mut self.rows, "rows", body_frame);
        let overlay_frame = Rect2::new(Vector2::ZERO, parent_frame.size);
        Self::update_frame(&mut self.overlay, "overlay", overlay_frame);
    }

    fn update_frame(control: &mut Option<ChildControl>, layer: &'static str, frame: Rect2) {
        let Some(object) = ChildControl::object(control, layer) else {
            return;
        };
        let Some(child) = control.as_mut() else {
            return;
        };
        if panel_common::apply_control_frame(&object, frame, &mut child.applied_frame) {
            object.queue_redraw();
        }
    }
}

/// The single source of the registered class name: the class table is
/// built from it and [`ChildTarget::spawn`] resolves the class through it.
pub(crate) const CLASS_NAME: &CStr = c"SpireProfilerPanelBody";

pub(crate) fn take_pending_child() -> Option<ChildTarget> {
    PENDING_CHILD.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_child_creation_clears_pending_owner() {
        let result = std::panic::catch_unwind(|| {
            let _pending = PendingChild::install(ChildTarget {
                owner: OwnerRef::Combat(Weak::new()),
                role: ChildRole::Rows,
            })
            .expect("no child creation is active in this test");
            panic!("engine creation failed before consuming the owner");
        });
        assert!(result.is_err());
        assert!(
            take_pending_child().is_none(),
            "unrelated children cannot inherit failed handoffs"
        );
    }
}
