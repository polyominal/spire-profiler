//! Scroll-state math shared by the two panels, unit-testable without an
//! engine: translation of shim-forwarded input into pixel deltas, the
//! clamped offset, and the scrollbar geometry.
//!
//! Scroll input arrives from the C# shim: natively inspecting engine input
//! events hangs the Godot fork, and polling wheel-button state misses macOS
//! trackpads, whose `InputEventPanGesture` deltas never set wheel-button
//! state.

use crate::engine::math::{Rect2, Vector2};

/// One wheel tick ≈ three rows.
const WHEEL_STEP: f32 = 60.0;

/// Godot `MouseButton` indexes (4 = up, 5 = down).
const MOUSE_BUTTON_WHEEL_UP: i64 = 4;
const MOUSE_BUTTON_WHEEL_DOWN: i64 = 5;

/// The flat C export cannot address the engine-owned panel, so the shim
/// names it.
pub(crate) const PANEL_COMBAT: i32 = 0;
pub(crate) const PANEL_RUN: i32 = 1;

/// A zero delta queues nothing; an unknown panel id degrades to no scroll.
pub(crate) fn queue_panel_scroll(panel: i32, button_index: i64, pressed: bool, pan_y: f32) {
    let delta = event_scroll_delta(button_index, pressed, pan_y);
    if delta == 0.0 {
        return;
    }
    match panel {
        PANEL_COMBAT => crate::ui::panel::queue_scroll(delta),
        PANEL_RUN => crate::ui::run_panel::queue_scroll(delta),
        _ => {}
    }
}

/// macOS negates the NSEvent deltas, so `delta.y` follows the system's
/// scroll-direction setting.
pub fn event_scroll_delta(button_index: i64, pressed: bool, pan_y: f32) -> f32 {
    match button_index {
        MOUSE_BUTTON_WHEEL_UP if pressed => -WHEEL_STEP,
        MOUSE_BUTTON_WHEEL_DOWN if pressed => WHEEL_STEP,
        _ => pan_y,
    }
}

pub fn apply_scroll(offset: f32, delta_px: f32, box_height: f32, content_height: f32) -> f32 {
    let max_scroll = (content_height - box_height).max(0.0);
    (offset + delta_px).clamp(0.0, max_scroll)
}

pub(crate) const GUTTER: f32 = 32.0;

/// Narrower than the game's 48px strip; the art stretches.
const TRACK_W: f32 = 20.0;
const TRACK_INSET_FLAT: f32 = 6.0;
/// The 60×36 cap art at track width, aspect kept.
const CAP_H: f32 = 12.0;
/// The game's 1.5× grabber:track ratio, aspect kept.
// TODO: the game's grabber spring-smooths and hover-scales; that juice
// needs an animation clock the poll-driven model lacks.
const GRABBER: f32 = TRACK_W * 1.5;

/// The overhang ends flush with the plate body's right edge, never into
/// the shadow strip.
fn track_inset(plate: bool) -> f32 {
    if plate {
        crate::ui::theme::PLATE_SHADOW_OFFSET + (GRABBER - TRACK_W) / 2.0
    } else {
        TRACK_INSET_FLAT
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollbarGeom {
    pub track: Rect2,
    pub body: Rect2,
    pub cap_top: Rect2,
    pub cap_bottom: Rect2,
    pub grabber: Rect2,
}

/// The grabber's size never changes; its y follows the scroll percent.
pub(crate) fn scrollbar_geom(
    box_size: Vector2,
    plate: bool,
    band: (f32, f32),
    content_height: f32,
    scroll: f32,
) -> Option<ScrollbarGeom> {
    let max_scroll = content_height - box_size.y;
    let track_h = band.1 - band.0;
    if max_scroll <= 0.0 || track_h <= 0.0 {
        return None;
    }
    let track = Rect2::new(
        Vector2::new(box_size.x - track_inset(plate) - TRACK_W, band.0),
        Vector2::new(TRACK_W, track_h),
    );
    let body = Rect2::new(
        track.position + Vector2::new(0.0, CAP_H),
        Vector2::new(TRACK_W, (track_h - 2.0 * CAP_H).max(0.0)),
    );
    let cap_top = Rect2::new(track.position, Vector2::new(TRACK_W, CAP_H));
    let cap_bottom = Rect2::new(
        track.position + Vector2::new(0.0, track_h - CAP_H),
        Vector2::new(TRACK_W, CAP_H),
    );
    let percent = scroll / max_scroll;
    let grabber_y = band.0 + percent * (track_h - GRABBER).max(0.0);
    let grabber = Rect2::new(
        Vector2::new(track.position.x + TRACK_W / 2.0 - GRABBER / 2.0, grabber_y),
        Vector2::new(GRABBER, GRABBER),
    );
    Some(ScrollbarGeom {
        track,
        body,
        cap_top,
        cap_bottom,
        grabber,
    })
}

pub(crate) fn track_scroll(track: Rect2, mouse_y: f32, max_scroll: f32) -> f32 {
    ((mouse_y - track.position.y) / track.size.y).clamp(0.0, 1.0) * max_scroll
}

pub(crate) fn scrollbar_state_next(
    active: bool,
    pressed: bool,
    mouse_down_prev: bool,
    on_track: bool,
) -> bool {
    pressed && (active || (!mouse_down_prev && on_track))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_scroll_steps_and_clamps() {
        assert_eq!(apply_scroll(0.0, -60.0, 400.0, 300.0), 0.0);
        assert_eq!(apply_scroll(0.0, 60.0, 400.0, 300.0), 0.0);

        assert_eq!(apply_scroll(0.0, -60.0, 200.0, 400.0), 0.0);
        assert_eq!(apply_scroll(0.0, 60.0, 200.0, 400.0), 60.0);
        assert_eq!(apply_scroll(60.0, 60.0, 200.0, 400.0), 120.0);
        assert_eq!(apply_scroll(120.0, -60.0, 200.0, 400.0), 60.0);
        assert_eq!(apply_scroll(180.0, 60.0, 200.0, 400.0), 200.0);
        assert_eq!(apply_scroll(200.0, 60.0, 200.0, 400.0), 200.0);
        assert_eq!(apply_scroll(200.0, -60.0, 200.0, 400.0), 140.0);

        assert_eq!(apply_scroll(100.0, 0.0, 200.0, 400.0), 100.0);
    }

    #[test]
    fn apply_scroll_accumulates_deltas_additively() {
        assert_eq!(apply_scroll(0.0, 12.5 + 7.5, 200.0, 400.0), 20.0);
        assert_eq!(apply_scroll(0.0, 60.0 + 15.0, 200.0, 400.0), 75.0);
        assert_eq!(apply_scroll(100.0, 12.5 - 7.5, 200.0, 400.0), 105.0);
    }

    #[test]
    fn event_scroll_delta_maps_wheel_edges_and_pan_deltas() {
        assert_eq!(event_scroll_delta(4, true, 0.0), -60.0);
        assert_eq!(event_scroll_delta(5, true, 0.0), 60.0);
        assert_eq!(event_scroll_delta(4, false, 0.0), 0.0);
        assert_eq!(event_scroll_delta(5, false, 0.0), 0.0);
        assert_eq!(event_scroll_delta(1, true, 0.0), 0.0);
        assert_eq!(event_scroll_delta(1, false, 0.0), 0.0);

        assert_eq!(event_scroll_delta(0, false, 24.0), 24.0);
        assert_eq!(event_scroll_delta(0, false, -10.0), -10.0);
        assert_eq!(event_scroll_delta(0, false, 0.0), 0.0);
    }

    #[test]
    fn scrollbar_geom_auto_hides_and_positions_the_grabber() {
        let box_size = Vector2::new(600.0, 400.0);
        let band = (100.0, 380.0);
        for plate in [false, true] {
            assert!(scrollbar_geom(box_size, plate, band, 300.0, 0.0).is_none());
            assert!(scrollbar_geom(box_size, plate, band, 400.0, 0.0).is_none());
        }

        let geom = scrollbar_geom(box_size, false, band, 600.0, 0.0).expect("overflowing content");
        assert_eq!(geom.track.position.y, band.0);
        assert_eq!(geom.track.size.y, band.1 - band.0);
        assert!(geom.track.position.x + geom.track.size.x <= box_size.x);
        assert_eq!(geom.body.size.y, geom.track.size.y - 2.0 * CAP_H);
        assert_eq!(geom.grabber.size, Vector2::new(30.0, 30.0));
        assert_eq!(geom.grabber.position.y, band.0);
        let track_center = geom.track.position.x + geom.track.size.x / 2.0;
        assert_eq!(
            geom.grabber.position.x + geom.grabber.size.x / 2.0,
            track_center
        );

        let travel = geom.track.size.y - geom.grabber.size.y;
        let bottom = scrollbar_geom(box_size, false, band, 600.0, 200.0).expect("overflowing");
        assert_eq!(bottom.grabber.position.y, band.0 + travel);
        let mid = scrollbar_geom(box_size, false, band, 600.0, 100.0).expect("overflowing");
        assert_eq!(mid.grabber.position.y, band.0 + travel / 2.0);
    }

    #[test]
    fn scrollbar_spans_the_body_band_inside_the_plate() {
        let box_size = Vector2::new(600.0, 400.0);
        for plate in [false, true] {
            let band = crate::ui::panel_replay::body_band(box_size.y, plate, 120.0);
            let geom = scrollbar_geom(box_size, plate, band, 512.0, 0.0).expect("overflowing");
            assert_eq!(geom.track.position.y, band.0, "plate={plate}");
            let pad = if plate {
                crate::ui::theme::PLATE_OUTER_PAD_BOTTOM
            } else {
                crate::ui::theme::FLAT_PAD
            };
            let track_bottom = geom.track.position.y + geom.track.size.y;
            assert_eq!(track_bottom, box_size.y - pad, "plate={plate}");
            let full = scrollbar_geom(box_size, plate, band, 512.0, 512.0 - box_size.y)
                .expect("overflowing");
            assert!(full.grabber.position.y + full.grabber.size.y <= box_size.y);
            let grabber_right = geom.grabber.position.x + geom.grabber.size.x;
            if plate {
                assert_eq!(
                    grabber_right,
                    box_size.x - crate::ui::theme::PLATE_SHADOW_OFFSET
                );
            } else {
                assert!(grabber_right <= box_size.x);
            }
        }
    }

    #[test]
    fn scrollbar_geom_at_minimum_box_sizes() {
        for box_size in [Vector2::new(360.0, 240.0), Vector2::new(520.0, 280.0)] {
            for plate in [false, true] {
                let band = crate::ui::panel_replay::body_band(box_size.y, plate, 100.0);
                let content = box_size.y * 3.0;
                let geom = scrollbar_geom(box_size, plate, band, content, 0.0).expect("overflows");
                assert!(geom.track.size.y >= 2.0 * CAP_H + geom.grabber.size.y);
                let track_bottom = geom.track.position.y + geom.track.size.y;
                for scroll in [0.0, box_size.y, 2.0 * box_size.y] {
                    let g =
                        scrollbar_geom(box_size, plate, band, content, scroll).expect("overflows");
                    assert!(g.grabber.position.y >= geom.track.position.y);
                    assert!(g.grabber.position.y + g.grabber.size.y <= track_bottom);
                }
            }
        }
    }

    #[test]
    fn track_scroll_maps_mouse_y_to_scroll_percent() {
        let box_size = Vector2::new(600.0, 400.0);
        let band = (100.0, 380.0);
        let geom = scrollbar_geom(box_size, false, band, 600.0, 0.0).expect("overflowing");
        assert_eq!(track_scroll(geom.track, geom.track.position.y, 200.0), 0.0);
        assert_eq!(
            track_scroll(
                geom.track,
                geom.track.position.y + geom.track.size.y / 2.0,
                200.0
            ),
            100.0
        );
        assert_eq!(
            track_scroll(geom.track, geom.track.position.y + geom.track.size.y, 200.0),
            200.0
        );
        assert_eq!(
            track_scroll(geom.track, geom.track.position.y - 50.0, 200.0),
            0.0
        );
        assert_eq!(
            track_scroll(
                geom.track,
                geom.track.position.y + geom.track.size.y + 50.0,
                200.0
            ),
            200.0
        );
    }

    #[test]
    fn scrollbar_drag_captures_on_track_press_and_ends_on_release() {
        assert!(scrollbar_state_next(false, true, false, true));
        assert!(!scrollbar_state_next(false, true, false, false));
        assert!(scrollbar_state_next(true, true, true, false));
        assert!(!scrollbar_state_next(true, false, true, false));
        assert!(!scrollbar_state_next(false, true, true, true));
    }

    #[test]
    fn queue_panel_scroll_routes_translated_pixels() {
        let _ = crate::ui::panel::take_queued_scroll();
        let _ = crate::ui::run_panel::take_queued_scroll();

        queue_panel_scroll(PANEL_COMBAT, 5, true, 0.0);
        queue_panel_scroll(PANEL_RUN, 4, true, 0.0);
        queue_panel_scroll(PANEL_COMBAT, 0, false, 12.5);
        queue_panel_scroll(PANEL_RUN, 4, false, 0.0);
        queue_panel_scroll(99, 5, true, 0.0);

        assert_eq!(crate::ui::panel::take_queued_scroll(), 72.5);
        assert_eq!(crate::ui::run_panel::take_queued_scroll(), -60.0);
        assert_eq!(crate::ui::panel::take_queued_scroll(), 0.0);
        assert_eq!(crate::ui::run_panel::take_queued_scroll(), 0.0);
    }
}
