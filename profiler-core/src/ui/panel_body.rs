//! The panels' auxiliary children — the third GDExtension class
//! ([`CLASS_NAME`]). Godot draws children after their parent and in child
//! order, so the rows child is added first and clipped to the scroll band;
//! the overlay child is added second and draws scrollbar, legend, and
//! tooltip above the rows.
//!
//! Child and owner state point across the engine boundary in both
//! directions. A child validates its owner token before every draw, while
//! the owner retains child object IDs (never raw object pointers), so a
//! child freed by the scene tree degrades that layer instead of crashing
//! the next dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;

use crate::engine::gdext::{Object, ObjectId, object_from_id, object_id};
use crate::engine::math::{Rect2, Vector2};
use crate::ui::panel_common;

thread_local! {
    /// Set by [`spawn`] and taken by the create callback running inside the
    /// instantiation call. Same thread, one call deep, so the slot never
    /// holds a stale child.
    static PENDING_CHILD: RefCell<Option<ChildTarget>> =
        const { RefCell::new(None) };
    static OWNERS: RefCell<OwnerRegistry> = RefCell::new(OwnerRegistry::new());
}

#[derive(Clone, Copy)]
pub(crate) enum ChildRole {
    Rows,
    Overlay,
}

#[derive(Clone, Copy)]
pub(crate) enum OwnerRef {
    Combat(*mut crate::ui::panel::SpireProfilerPanel),
    Run(*mut crate::ui::run_panel::SpireProfilerRunPanel),
}

impl OwnerRef {
    fn address(self) -> usize {
        match self {
            OwnerRef::Combat(panel) => panel.cast::<u8>().addr(),
            OwnerRef::Run(panel) => panel.cast::<u8>().addr(),
        }
    }
}

#[derive(Clone, Copy)]
struct OwnerId {
    owner: OwnerRef,
    generation: u64,
}

impl OwnerId {
    fn is_live(self) -> bool {
        OWNERS.with(|owners| owners.borrow().contains(self))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ChildTarget {
    owner: OwnerId,
    role: ChildRole,
}

impl ChildTarget {
    fn is_live(self) -> bool {
        self.owner.is_live()
    }

    pub(crate) fn owner_ref(self) -> OwnerRef {
        self.owner.owner
    }

    pub(crate) fn role(self) -> ChildRole {
        self.role
    }
}

struct OwnerToken {
    id: OwnerId,
}

struct OwnerRegistry {
    next_generation: u64,
    live: HashMap<usize, u64>,
}

impl OwnerRegistry {
    fn new() -> Self {
        OwnerRegistry {
            next_generation: 0,
            live: HashMap::new(),
        }
    }

    fn register(&mut self, owner: OwnerRef) -> u64 {
        let generation = self.next_generation.wrapping_add(1);
        self.next_generation = generation;
        self.live.insert(owner.address(), generation);
        generation
    }

    fn unregister(&mut self, id: OwnerId) {
        if self.live.get(&id.owner.address()).copied() == Some(id.generation) {
            self.live.remove(&id.owner.address());
        }
    }

    fn contains(&self, id: OwnerId) -> bool {
        self.live
            .get(&id.owner.address())
            .is_some_and(|generation| *generation == id.generation)
    }
}

fn register_owner(owner: OwnerRef) -> OwnerToken {
    let generation = OWNERS.with(|owners| owners.borrow_mut().register(owner));
    OwnerToken {
        id: OwnerId { owner, generation },
    }
}

impl Drop for OwnerToken {
    fn drop(&mut self) {
        OWNERS.with(|owners| owners.borrow_mut().unregister(self.id));
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

    pub(crate) fn live_target(&self) -> Option<ChildTarget> {
        self.target.filter(|target| target.is_live())
    }

    pub(crate) fn object(&self) -> &Object {
        &self.object
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
}

struct AttachedChildren {
    // Held for Drop: it removes the owner from the live-owner registry.
    #[allow(dead_code)]
    owner: OwnerToken,
    rows: Option<ChildControl>,
    overlay: Option<ChildControl>,
}

#[derive(Default)]
pub(crate) struct PanelChildren {
    attached: Option<AttachedChildren>,
}

impl PanelChildren {
    pub(crate) fn attach(parent: Object, owner: OwnerRef) -> Self {
        let token = register_owner(owner);
        let rows = ChildTarget {
            owner: token.id,
            role: ChildRole::Rows,
        };
        let overlay = ChildTarget {
            owner: token.id,
            role: ChildRole::Overlay,
        };
        PanelChildren {
            attached: Some(AttachedChildren {
                owner: token,
                rows: spawn(parent, rows).map(ChildControl::new),
                overlay: spawn(parent, overlay).map(ChildControl::new),
            }),
        }
    }

    pub(crate) fn objects(&mut self) -> panel_common::ChildObjects {
        let Some(attached) = self.attached.as_mut() else {
            return panel_common::ChildObjects::default();
        };
        panel_common::ChildObjects {
            body: object(&mut attached.rows, "rows"),
            overlay: object(&mut attached.overlay, "overlay"),
        }
    }

    pub(crate) fn queue_redraw(&mut self) {
        self.objects().queue_redraw();
    }

    pub(crate) fn queue_overlay_redraw(&mut self) {
        if let Some(overlay) = self.overlay_object() {
            overlay.queue_redraw();
        }
    }

    pub(crate) fn update_frames(&mut self, parent_frame: Rect2, body_frame: Rect2) {
        let Some(attached) = self.attached.as_mut() else {
            return;
        };
        Self::update_frame(&mut attached.rows, "rows", body_frame);
        let overlay_frame = Rect2::new(Vector2::ZERO, parent_frame.size);
        Self::update_frame(&mut attached.overlay, "overlay", overlay_frame);
    }

    fn overlay_object(&mut self) -> Option<Object> {
        let attached = self.attached.as_mut()?;
        object(&mut attached.overlay, "overlay")
    }

    fn update_frame(control: &mut Option<ChildControl>, layer: &'static str, frame: Rect2) {
        let Some(object) = object(control, layer) else {
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

/// The single source of the registered class name: the class table is
/// built from it and [`spawn`] resolves the class through it.
pub(crate) const CLASS_NAME: &CStr = c"SpireProfilerPanelBody";

pub(crate) fn take_pending_child() -> Option<ChildTarget> {
    PENDING_CHILD.with(|slot| slot.borrow_mut().take())
}

/// Creates a child, makes it mouse-transparent, and parents it. A None —
/// loudly logged — leaves that layer unavailable while the panel continues.
fn spawn(parent: Object, target: ChildTarget) -> Option<Object> {
    PENDING_CHILD.with(|slot| *slot.borrow_mut() = Some(target));
    let child = crate::engine::gdext::instantiate_class(CLASS_NAME);
    // A failed instantiation may not have run the create callback; don't
    // poison the next spawn's handoff.
    PENDING_CHILD.with(|slot| {
        *slot.borrow_mut() = None;
    });
    let Some(child) = child else {
        let layer = match target.role {
            ChildRole::Rows => "rows",
            ChildRole::Overlay => "overlay",
        };
        crate::fail!("panel body class instantiation failed; {layer} disabled");
        return None;
    };
    if matches!(target.role, ChildRole::Rows) {
        child.set_clip_contents(true);
    }
    child.set_mouse_filter_ignore();
    parent.add_child(child);
    Some(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_generations_distinguish_reused_addresses() {
        let mut owner = 0_u8;
        let owner_ref = OwnerRef::Combat(std::ptr::from_mut(&mut owner).cast());
        let first = register_owner(owner_ref);
        let first_id = first.id;
        drop(first);
        let second = register_owner(owner_ref);
        let second_id = second.id;

        let target = |id| ChildTarget {
            owner: id,
            role: ChildRole::Rows,
        };
        assert!(!target(first_id).is_live());
        assert!(target(second_id).is_live());
        drop(second);
        assert!(!target(second_id).is_live());
    }
}
