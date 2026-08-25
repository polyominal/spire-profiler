//! Engine plumbing shared by the two panels, refresh/input path. The
//! `_draw` path is [`super::panel_replay`].

use std::cell::Cell;

use crate::engine::gdext::Object;
use crate::engine::math::{Rect2, Vector2};
use crate::fail;
use crate::ui::chart_layout::{self, Cmd, RectCmd};
use crate::ui::ui_model::UiTab;

pub(crate) fn over_panel(rect: Rect2, mouse: Vector2) -> bool {
    mouse.x >= rect.position.x
        && mouse.x < rect.position.x + rect.size.x
        && mouse.y >= rect.position.y
        && mouse.y < rect.position.y + rect.size.y
}

/// A held button drifting outside is not a dismissal (a scrollbar drag
/// must survive leaving the box).
pub(crate) fn dismiss_on_outside_press(shown: bool, press_edge: bool, over: bool) -> bool {
    shown && press_edge && !over
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

/// The pending amount is read AND cleared every frame; the header rides
/// in both content and box heights, so it cancels out of the overflow.
pub(crate) fn wheel_scroll(
    object: &Object,
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
        object.queue_redraw();
    }
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
    object: &Object,
    rect: Rect2,
    mouse: Vector2,
    state: &mut InteractionState,
    scrollbar: ScrollbarFrame,
    scroll: &mut f32,
) -> InteractionStep {
    let pressed = crate::engine::gdext::mouse_button_left();
    let on_track = scrollbar_step(
        object,
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

pub(crate) struct ScrollbarFrame {
    pub geom: Option<crate::ui::scroll::ScrollbarGeom>,
    pub content_height: f32,
}

/// While active, maps the cursor's y to the scroll offset (the game's
/// click-jumps mapping).
fn scrollbar_step(
    object: &Object,
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
            object.queue_redraw();
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
pub(crate) fn apply_control_frame(object: &Object, frame: Rect2, applied: &mut Option<Rect2>) {
    if *applied == Some(frame) {
        return;
    }
    object.set_position(frame.position);
    object.set_size(frame.size);
    *applied = Some(frame);
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
    CMD_OVERFLOW_LOGGED.with(|logged| {
        if !logged.get() {
            logged.set(true);
            fail!("{owner}: layout command cap exceeded; tail commands dropped");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chart_layout::RowHit;
    use crate::ui::panel_replay::body_band;

    #[test]
    fn outside_press_dismisses_only_on_the_edge_while_shown() {
        assert!(dismiss_on_outside_press(true, true, false));
        for (shown, edge, over) in [
            (false, true, false),
            (true, false, false),
            (true, true, true),
            (false, false, true),
        ] {
            assert!(!dismiss_on_outside_press(shown, edge, over));
        }
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
