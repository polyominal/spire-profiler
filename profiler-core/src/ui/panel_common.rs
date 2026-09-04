//! Engine plumbing shared by the two panels, refresh/input path. The
//! `_draw` path is [`super::panel_replay`].

use std::cell::Cell;
use std::time::Instant;

use crate::data::state::PlayerFilter;
use crate::engine::gdext::{MOUSE_BUTTON_LEFT, Object, mouse_button_pressed};
use crate::engine::math::{Rect2, Vector2};
use crate::ui::chart_layout::{self, Cmd, RectCmd};
use crate::ui::theme::Theme;
use crate::ui::tooltip::{self, RowDetail, TipLine};
use crate::ui::ui_model::UiTab;

pub(crate) fn over_panel(rect: Rect2, mouse: Vector2) -> bool {
    mouse.x >= rect.position.x
        && mouse.x < rect.position.x + rect.size.x
        && mouse.y >= rect.position.y
        && mouse.y < rect.position.y + rect.size.y
}

/// A held button drifting outside is not a dismissal (a scrollbar drag
/// must survive leaving the box).
pub(crate) fn dismiss_on_outside_press(press_edge: bool, over: bool) -> bool {
    press_edge && !over
}

/// Guarded to the visible box: input is polled globally, and the zones are
/// box-local.
pub(crate) fn guarded_zone(
    rect: Rect2,
    mouse: Vector2,
    zone: impl FnOnce(f32, f32) -> PressZone,
) -> PressZone {
    if !over_panel(rect, mouse) {
        return PressZone::Inert;
    }
    zone(mouse.x - rect.position.x, mouse.y - rect.position.y)
}

pub(crate) fn viewport_mouse(object: &Object, mouse: &mut Vector2) {
    if let Some(viewport) = object.get_viewport()
        && let Some(position) = viewport.get_mouse_position()
    {
        *mouse = position;
    }
}

pub(crate) fn hover_row(
    hits: &[chart_layout::RowHit],
    rect: Rect2,
    mouse: Vector2,
    scroll: f32,
    band: (f32, f32),
) -> Option<usize> {
    if !over_panel(rect, mouse) {
        return None;
    }
    let local_y = mouse.y - rect.position.y;
    if local_y < band.0 || local_y >= band.1 {
        return None;
    }
    chart_layout::row_at(hits, local_y + scroll)
}

/// Wheel deltas queue between frames (one cell per panel) and drain into
/// the panel's pending amount at every refresh.
pub(crate) fn queue_scroll(queue: &Cell<f32>, delta: f32) {
    queue.set(queue.get() + delta);
}

pub(crate) fn take_queued_scroll(queue: &Cell<f32>) -> f32 {
    queue.replace(0.0)
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ChildObjects {
    pub(crate) body: Option<Object>,
    pub(crate) overlay: Option<Object>,
}

impl ChildObjects {
    pub(crate) fn queue_redraw(self) {
        if let Some(body) = self.body {
            body.queue_redraw();
        }
        if let Some(overlay) = self.overlay {
            overlay.queue_redraw();
        }
    }
}

/// The pending amount is read AND cleared every frame; the header rides
/// in both content and box heights, so it cancels out of the overflow.
/// Rows and scrollbar are separate items: a moved offset redraws both children.
pub(crate) fn wheel_scroll(
    children: ChildObjects,
    scroll: &mut f32,
    pending: &mut f32,
    rect: Rect2,
    mouse: Vector2,
    content_height: f32,
) {
    let old_scroll = *scroll;
    let delta = std::mem::take(pending);
    if over_panel(rect, mouse) && delta != 0.0 {
        *scroll = crate::ui::scroll::apply_scroll(*scroll, delta, rect.size.y, content_height);
    }
    if *scroll != old_scroll {
        children.queue_redraw();
    }
}

/// The body child's rect in Control-local space — the scroll viewport the
/// engine clips the child's drawing to.
pub(crate) fn body_frame(
    origin_x: f32,
    box_size: Vector2,
    plate: bool,
    header_bottom: f32,
) -> Rect2 {
    let band = crate::ui::panel_replay::body_band(box_size.y, plate, header_bottom);
    Rect2::new(
        Vector2::new(origin_x, band.0),
        Vector2::new(box_size.x, band.1 - band.0),
    )
}

/// Fixed row heights mean the gutter can never change the overflow
/// verdict and oscillate.
pub(crate) fn scrollbar_gutter(content_height: f32, box_height: f32, has_sprites: bool) -> f32 {
    if content_height > box_height && has_sprites {
        crate::ui::scroll::GUTTER
    } else {
        0.0
    }
}

/// None when the bar is hidden (content fits, or sprites failed).
pub(crate) fn scrollbar_geom(
    theme: &Theme,
    box_size: Vector2,
    header_bottom: f32,
    content_height: f32,
    scroll: f32,
) -> Option<crate::ui::scroll::ScrollbarGeom> {
    theme.scrollbar()?;
    let plate = theme.plate().is_some();
    crate::ui::scroll::scrollbar_geom(
        box_size,
        plate,
        crate::ui::panel_replay::body_band(box_size.y, plate, header_bottom),
        content_height,
        scroll,
    )
}

/// The scrollbar track is not a zone — it is hit-tested in screen space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PressZone {
    Tab(UiTab),
    Avatar(u8),
    Inert,
}

/// A track press must never switch a tab.
pub(crate) fn interaction_step(
    children: ChildObjects,
    rect: Rect2,
    mouse: Vector2,
    state: &mut InteractionState,
    scrollbar: ScrollbarFrame,
    scroll: &mut f32,
) -> InteractionStep {
    let pressed = mouse_button_pressed(MOUSE_BUTTON_LEFT);
    let on_track = scrollbar_step(
        children,
        rect.size,
        mouse - rect.position,
        pressed,
        state,
        scrollbar,
        scroll,
    );
    state.mouse_down = pressed;
    InteractionStep { pressed, on_track }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct InteractionState {
    pub scrollbar: bool,
    pub mouse_down: bool,
}

pub(crate) struct InteractionStep {
    pub pressed: bool,
    pub on_track: bool,
}

// The v0.111.0 run-history icons select at 1.1, deselect at 0.95, and take
// 0.05 seconds to move between those states.
const AVATAR_SCALE_DURATION_SECS: f32 = 0.05;
const AVATAR_ALL_SCALE: f32 = 1.0;
const AVATAR_SELECTED_SCALE: f32 = 1.1;
const AVATAR_EXCLUDED_SCALE: f32 = 0.95;

#[derive(Clone, Copy, Debug)]
struct AvatarScaleTransition {
    current: f32,
    from: f32,
    target: f32,
    elapsed: f32,
}

impl AvatarScaleTransition {
    fn at(target: f32) -> Self {
        Self {
            current: target,
            from: target,
            target,
            elapsed: 0.0,
        }
    }

    fn retarget(&mut self, target: f32) {
        if self.target == target {
            return;
        }
        self.from = self.current;
        self.target = target;
        self.elapsed = 0.0;
    }

    fn advance(&mut self, delta: f32) -> bool {
        if self.current == self.target {
            return false;
        }
        self.elapsed += delta;
        if self.elapsed >= AVATAR_SCALE_DURATION_SECS {
            self.current = self.target;
            return false;
        }
        let progress = (self.elapsed / AVATAR_SCALE_DURATION_SECS).clamp(0.0, 1.0);
        self.current = self.from + (self.target - self.from) * progress;
        true
    }
}

#[derive(Clone, Copy, Debug)]
struct AvatarScaleEntry {
    slot: u8,
    transition: AvatarScaleTransition,
}

#[derive(Default)]
pub(crate) struct AvatarScaleAnimation {
    entries: Vec<AvatarScaleEntry>,
    /// Flat projection of the entries' current scales; replay needs a
    /// contiguous `&[f32]`.
    scales: Vec<f32>,
    last_tick: Option<Instant>,
}

impl AvatarScaleAnimation {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.scales.clear();
        self.last_tick = None;
    }

    pub(crate) fn set_targets(&mut self, filter: PlayerFilter, slots: &[u8]) {
        let shared = slots.len().min(self.entries.len());
        for (index, &slot) in slots.iter().enumerate().take(shared) {
            // A changed slot identity is a new roster entry, not a filter
            // transition.
            let target = Self::target(filter, slot);
            let entry = &mut self.entries[index];
            if entry.slot != slot {
                entry.slot = slot;
                entry.transition = AvatarScaleTransition::at(target);
            } else {
                entry.transition.retarget(target);
            }
            self.scales[index] = entry.transition.current;
        }
        self.entries.truncate(shared);
        self.scales.truncate(shared);
        for &slot in &slots[shared..] {
            let transition = AvatarScaleTransition::at(Self::target(filter, slot));
            self.entries.push(AvatarScaleEntry { slot, transition });
            self.scales.push(transition.current);
        }
    }

    pub(crate) fn advance_frame(&mut self, object: &Object, filter: PlayerFilter, slots: &[u8]) {
        self.set_targets(filter, slots);
        if !self
            .entries
            .iter()
            .any(|entry| entry.transition.current != entry.transition.target)
        {
            self.last_tick = None;
            return;
        }
        let now = Instant::now();
        let delta = self
            .last_tick
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        let needs_redraw = self.advance(delta);
        if needs_redraw {
            object.queue_redraw();
        }
    }

    fn advance(&mut self, delta: f32) -> bool {
        let mut needs_redraw = false;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            let before = entry.transition.current;
            // Reaching the target still needs a draw; the preceding draw
            // showed the prior animation frame.
            let active = entry.transition.advance(delta);
            needs_redraw |= active || entry.transition.current != before;
            self.scales[index] = entry.transition.current;
        }
        needs_redraw
    }

    pub(crate) fn values(&self) -> &[f32] {
        &self.scales
    }

    fn target(filter: PlayerFilter, slot: u8) -> f32 {
        match filter {
            PlayerFilter::All => AVATAR_ALL_SCALE,
            PlayerFilter::Player(selected) if selected == slot => AVATAR_SELECTED_SCALE,
            PlayerFilter::Player(_) => AVATAR_EXCLUDED_SCALE,
        }
    }
}

pub(crate) struct ScrollbarFrame {
    pub geom: Option<crate::ui::scroll::ScrollbarGeom>,
    pub content_height: f32,
}

/// An empty detail reshapes to no lines; the line cap derives from the
/// box height, so a stale larger box can never leave a tip exceeding the
/// new y-band.
pub(crate) fn reshape_tip(detail: &RowDetail, box_height: f32) -> Vec<TipLine> {
    if detail.is_empty() {
        Vec::new()
    } else {
        tooltip::shape(detail, tooltip::max_tip_lines(box_height))
    }
}

/// The dim mask parallels the icon row: excluded players render at the
/// game's deselect modulate. A roster/slot mismatch leaves the mask empty
/// (release draws avatars undimmed) instead of panicking in the contained
/// draw path.
pub(crate) fn fill_dim_mask(
    portrait_count: usize,
    slots: &[u8],
    filter: PlayerFilter,
    dimmed: &mut Vec<bool>,
) {
    dimmed.clear();
    debug_assert_eq!(
        portrait_count,
        slots.len(),
        "the rebuild fills portrait paths and roster slots together"
    );
    if portrait_count == slots.len() {
        for &slot in slots {
            dimmed.push(filter != PlayerFilter::All && filter != PlayerFilter::Player(slot));
        }
    }
}

/// While active, maps the cursor's y to the scroll offset (the game's
/// click-jumps mapping).
#[allow(clippy::too_many_arguments)] // one frame's input context; bundling it further is artificial
fn scrollbar_step(
    children: ChildObjects,
    box_size: Vector2,
    local: Vector2,
    pressed: bool,
    state: &mut InteractionState,
    frame: ScrollbarFrame,
    scroll: &mut f32,
) -> bool {
    let on_track = frame.geom.is_some_and(|geom| {
        local.x >= geom.track.position.x
            && local.x < geom.track.position.x + geom.track.size.x
            && local.y >= geom.track.position.y
            && local.y < geom.track.position.y + geom.track.size.y
    });
    let next = crate::ui::scroll::scrollbar_state_next(
        state.scrollbar,
        pressed,
        state.mouse_down,
        on_track,
    );
    if next && let Some(geom) = frame.geom {
        let max_scroll = (frame.content_height - box_size.y).max(0.0);
        let next_scroll = crate::ui::scroll::track_scroll(geom.track, local.y, max_scroll);
        if next_scroll != *scroll {
            *scroll = next_scroll;
            children.queue_redraw();
        }
    }
    state.scrollbar = next;
    on_track
}

/// The game's data screens never run content to the screen edge, and the
/// plate's drop shadow stays fully on-screen.
pub(crate) const MODAL_MARGIN: f32 = 48.0;

/// A degenerate viewport never collapses the box to nothing.
const MIN_MODAL_H: f32 = 96.0;

const MODAL_CAP_FALLBACK: f32 = 600.0;

pub(crate) fn modal_height_cap(viewport_height: Option<f32>) -> f32 {
    viewport_height.map_or(MODAL_CAP_FALLBACK, |h| {
        (h - 2.0 * MODAL_MARGIN).max(MIN_MODAL_H)
    })
}

/// Dead center, floored at the origin — a box wider than the viewport pins
/// top-left rather than centering its overflow off-screen on both sides.
pub(crate) fn centered_position(viewport: Vector2, box_size: Vector2) -> Vector2 {
    Vector2::new(
        ((viewport.x - box_size.x) / 2.0).max(0.0),
        ((viewport.y - box_size.y) / 2.0).max(0.0),
    )
}

/// Without a viewport (init's placeholder frame) the position is None and
/// resolves on the first in-tree refresh.
pub(crate) fn modal_box(
    viewport: Option<Vector2>,
    width: f32,
    content_height: f32,
    scroll: &mut f32,
) -> (Vector2, Option<Vector2>) {
    let size = Vector2::new(
        width,
        modal_height_cap(viewport.map(|v| v.y)).min(content_height),
    );
    *scroll = (*scroll).min((content_height - size.y).max(0.0));
    let pos = viewport.map(|v| centered_position(v, size));
    (size, pos)
}

/// `set_position`/`set_size` only on change: every engine call is a
/// `variant_call` round-trip, so a steady frame must not re-issue them.
pub(crate) fn apply_control_frame(
    object: &Object,
    frame: Rect2,
    applied: &mut Option<Rect2>,
) -> bool {
    if *applied == Some(frame) {
        return false;
    }
    object.set_position(frame.position);
    object.set_size(frame.size);
    *applied = Some(frame);
    true
}

/// Stores the frame-local legend/tip rect and, on change only, redraws
/// the panel and the overlay child — a steady frame must not re-issue
/// engine calls.
pub(crate) fn apply_overlay_rect(
    slot: &mut Option<Rect2>,
    rect: Option<Rect2>,
    object: &Object,
    children: &mut crate::ui::panel_body::PanelChildren,
) {
    if *slot == rect {
        return;
    }
    *slot = rect;
    object.queue_redraw();
    children.queue_overlay_redraw();
}

/// The viewport's visible size, or None when the panel is not in the tree
/// (init runs before `AddChild`).
pub(crate) fn viewport_size(object: &Object) -> Option<Vector2> {
    object
        .get_viewport()
        .and_then(|viewport| viewport.get_visible_rect())
        .map(|rect| rect.size)
}

/// The parent is a plain Control, so the border is ordinary rects rather
/// than a PanelContainer's theme stylebox.
pub(crate) fn border_rects(width: f32, height: f32) -> [Cmd; 4] {
    [
        Cmd::Rect(RectCmd {
            x: 0.0,
            y: 0.0,
            w: width,
            h: 1.0,
            color: crate::ui::palette::COL_PANEL_BORDER,
        }),
        Cmd::Rect(RectCmd {
            x: 0.0,
            y: height - 1.0,
            w: width,
            h: 1.0,
            color: crate::ui::palette::COL_PANEL_BORDER,
        }),
        Cmd::Rect(RectCmd {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: height,
            color: crate::ui::palette::COL_PANEL_BORDER,
        }),
        Cmd::Rect(RectCmd {
            x: width - 1.0,
            y: 0.0,
            w: 1.0,
            h: height,
            color: crate::ui::palette::COL_PANEL_BORDER,
        }),
    ]
}

thread_local! {
    /// A cap trip means the layout outgrew the worst case the caps were
    /// sized for; one ERROR line per process is loud enough.
    static CMD_OVERFLOW_LOGGED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn log_cmd_overflow_once(owner: &str) {
    crate::fail_once(
        &CMD_OVERFLOW_LOGGED,
        format_args!("{owner}: layout command cap exceeded; tail commands dropped"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chart_layout::RowHit;
    use crate::ui::panel_replay::body_band;

    fn assert_scale(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "scale {actual} should be {expected}"
        );
    }

    #[test]
    fn avatar_scales_interpolate_linearly_and_clamp() {
        let mut animation = AvatarScaleAnimation::default();
        animation.set_targets(PlayerFilter::All, &[0]);
        assert_eq!(animation.values(), &[1.0]);

        animation.set_targets(PlayerFilter::Player(0), &[0]);
        assert_eq!(animation.values(), &[1.0]);
        assert!(animation.advance(0.0));

        animation.advance(0.025);
        assert_scale(animation.values()[0], 1.05);

        assert!(animation.advance(0.05));
        assert_eq!(animation.values(), &[1.1]);
        assert!(!animation.advance(0.0));

        animation.set_targets(PlayerFilter::Player(1), &[0]);
        assert!(animation.advance(0.05));
        assert_eq!(animation.values(), &[0.95]);
        assert!(!animation.advance(0.0));
    }

    #[test]
    fn avatar_scale_retargeting_starts_from_the_current_scale() {
        let mut animation = AvatarScaleAnimation::default();
        animation.set_targets(PlayerFilter::All, &[0]);
        animation.set_targets(PlayerFilter::Player(0), &[0]);
        animation.advance(0.025);
        assert_scale(animation.values()[0], 1.05);

        animation.set_targets(PlayerFilter::All, &[0]);
        assert_scale(animation.values()[0], 1.05);
        animation.advance(0.025);
        assert_scale(animation.values()[0], 1.025);
    }

    #[test]
    fn avatar_roster_changes_initialize_new_slots_at_target() {
        let mut animation = AvatarScaleAnimation::default();
        animation.set_targets(PlayerFilter::All, &[0]);
        animation.set_targets(PlayerFilter::Player(0), &[0]);
        animation.advance(0.025);
        animation.set_targets(PlayerFilter::Player(0), &[0, 1]);
        assert_scale(animation.values()[0], 1.05);
        assert_eq!(animation.values()[1], 0.95);

        animation.set_targets(PlayerFilter::Player(2), &[2, 1]);
        assert_eq!(animation.values(), &[1.1, 0.95]);
        animation.clear();
        assert!(animation.values().is_empty());
    }

    #[test]
    fn outside_press_dismisses_only_on_the_edge() {
        assert!(dismiss_on_outside_press(true, false));
        assert!(!dismiss_on_outside_press(false, false));
        assert!(!dismiss_on_outside_press(true, true));
    }

    fn hits() -> Vec<RowHit> {
        vec![
            RowHit {
                y0: 110.0,
                y1: 120.0,
                flat_index: 0,
            },
            RowHit {
                y0: 120.0,
                y1: 130.0,
                flat_index: 1,
            },
            RowHit {
                y0: 400.0,
                y1: 420.0,
                flat_index: 2,
            },
        ]
    }

    fn rect() -> Rect2 {
        Rect2::new(Vector2::new(100.0, 100.0), Vector2::new(660.0, 300.0))
    }

    fn band() -> (f32, f32) {
        body_band(300.0, true, 100.0)
    }

    #[test]
    fn hover_row_maps_content_coordinates_through_scroll() {
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 215.0), 0.0, band()),
            Some(0)
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 215.0), 300.0, band()),
            Some(2)
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 235.0), 0.0, band()),
            None
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(50.0, 215.0), 0.0, band()),
            None
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(650.0, 215.0), 0.0, band()),
            Some(0)
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(599.0, 215.0), 0.0, band()),
            Some(0)
        );
    }

    #[test]
    fn hover_row_resolves_only_inside_the_body_band() {
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 130.0), 90.0, band()),
            None
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 390.0), 120.0, band()),
            None
        );
        assert_eq!(
            hover_row(&hits(), rect(), Vector2::new(200.0, 210.0), 0.0, band()),
            Some(0)
        );
    }

    #[test]
    fn body_frame_is_the_band_translated_to_origin_x() {
        let plate = body_frame(40.0, Vector2::new(660.0, 300.0), true, 100.0);
        assert_eq!(
            plate,
            Rect2::new(Vector2::new(40.0, 100.0), Vector2::new(660.0, 172.0))
        );
        assert_eq!(
            body_frame(40.0, Vector2::new(660.0, 300.0), false, 100.0),
            Rect2::new(Vector2::new(40.0, 100.0), Vector2::new(660.0, 188.0))
        );
    }

    #[test]
    fn modal_height_cap_subtracts_the_margin_and_floors() {
        assert_eq!(modal_height_cap(Some(1080.0)), 1080.0 - 96.0);
        assert_eq!(modal_height_cap(Some(150.0)), MIN_MODAL_H);
        assert_eq!(modal_height_cap(Some(2.0 * MODAL_MARGIN)), MIN_MODAL_H);
        assert_eq!(modal_height_cap(None), MODAL_CAP_FALLBACK);
    }

    #[test]
    fn border_rects_are_the_four_edges_only() {
        let borders = border_rects(100.0, 100.0);
        assert_eq!(borders.len(), 4);
        assert!(
            borders.iter().all(
                |cmd| matches!(cmd, Cmd::Rect(r) if r.color == crate::ui::palette::COL_PANEL_BORDER)
            ),
            "only border edges, no background rect"
        );
    }

    #[test]
    fn centered_position_centers_and_floors_at_the_origin() {
        let viewport = Vector2::new(1920.0, 1080.0);
        let plate = Vector2::new(600.0, 400.0);
        assert_eq!(
            centered_position(viewport, plate),
            Vector2::new(660.0, 340.0)
        );
        assert_eq!(
            centered_position(viewport, Vector2::new(601.0, 401.0)),
            Vector2::new(659.5, 339.5)
        );
        assert_eq!(
            centered_position(Vector2::new(500.0, 1080.0), plate),
            Vector2::new(0.0, 340.0)
        );
        assert_eq!(
            centered_position(Vector2::new(1920.0, 300.0), plate),
            Vector2::new(660.0, 0.0)
        );
    }
}
