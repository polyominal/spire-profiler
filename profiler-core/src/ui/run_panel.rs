//! Run-history panel — the second GDExtension class (`SpireProfilerRunPanel`),
//! rendering the selected run's contribution summary on the game's
//! run-history screen. The combat chart panel ([`crate::ui::panel`]) stays
//! untouched; this module owns its own state and tests. The pure layout
//! engine (the geometry, the emitters, the header icon row) is
//! [`crate::ui::run_layout`] — the same engine/class split as the combat
//! side's `chart_layout`/`panel`.
//!
//! ## Visibility + toggle semantics (decision)
//!
//! * A manual flag, off on entry: the panel shows only while the run-history screen is open AND the
//!   flag is set. The ABI's clear export (the screen's close and its open, before the first select)
//!   resets the flag, so every entry to the screen starts closed. The core tracks the open screen
//!   from the shim's select/clear events (`run_history::screen_open`), so no per-frame scene-tree
//!   probing is needed; the panel is a child of the `NRunHistory` node and inherits its hidden
//!   state anyway.
//! * Toggle: **core-side context routing on the existing export** — the shim's F8 handler and the
//!   run-history button both call `spire_profiler_panel_toggle` (no new export, keeping check-abi
//!   at 38 bindings); abi.rs routes it to this panel's flag while the screen is open and to the
//!   combat panel's otherwise. The panels keep separate flags, so hiding the combat panel mid-fight
//!   never hides the run panel.
//! * Empty state: the screen is open but the displayed run has no profiler record — the selection
//!   is empty while `screen_open` stays true, and the panel renders the empty-state notice instead
//!   of hiding.
//! * Click-away dismissal: a press outside the plate box clears the flag. The shim disables the
//!   toggle button while the panel is up, so a click on it cannot re-open on the same press: it
//!   dismisses like any other background press.
//!
//! ## Placement
//!
//! The panel is a centered modal: a content-sized plate (the designed
//! width, the content height capped at the viewport minus a margin)
//! centered in the viewport, re-derived whenever the viewport size
//! changes. `set_position` is parent-relative and the panel is reparented
//! under the run-history screen node — but in the v0.111.0 scenes every
//! ancestor of NRunHistory (the chain up through the run_history.tscn and
//! run.tscn roots; the main-menu and game-over hosts likewise) is a
//! full-rect anchored Control with zero offsets, so parent-relative ==
//! viewport position and the viewport-space centering lands exactly.
//! Nothing is persisted; the frame re-derives every session.
//!
//! ## Headless
//!
//! The class registers silently at extension load (the shared
//! [`crate::engine::gdext`] entry) and the shim instantiates it lazily on the
//! first `DisplayRun`, which never happens in a headless boot (main menu
//! only) — so no new boot markers and no behavior change there.

use std::cell::Cell;

use crate::data::run_history::RunSummaryView;
use crate::data::state::CardStat;
use crate::engine::gdext::Object;
use crate::engine::math::{Rect2, Vector2};
use crate::ui::panel_common::{self, AvatarScaleAnimation, InteractionState, PressZone};
use crate::ui::run_layout::{
    HeaderFacts, PortraitFact, RunLayout, WIDTH, build_run_layout, character_icon_path,
    roster_entries,
};
use crate::ui::tooltip::RowDetail;
use crate::ui::ui_model::{self, UiRow};
use crate::ui::{chart_layout, panel_body, panel_replay};

thread_local! {
    static RUN_MANUAL_VISIBLE: Cell<bool> = const { Cell::new(false) };
    /// Module scope because the flat C export cannot address the panel
    /// instance.
    static QUEUED_SCROLL: Cell<f32> = const { Cell::new(0.0) };
}

pub(crate) fn queue_scroll(delta: f32) {
    QUEUED_SCROLL.with(|q| panel_common::queue_scroll(q, delta));
}

pub(crate) fn take_queued_scroll() -> f32 {
    QUEUED_SCROLL.with(panel_common::take_queued_scroll)
}

/// Shown only while the run-history screen is open AND this flag is set.
pub(crate) fn run_manual_visible() -> bool {
    RUN_MANUAL_VISIBLE.with(|v| v.get())
}

pub(crate) fn toggle_run_manual() {
    RUN_MANUAL_VISIBLE.with(|v| v.set(!v.get()));
}

/// Clears the flag: the click-away dismissal, and the screen-close reset.
pub(crate) fn dismiss_run_manual() {
    RUN_MANUAL_VISIBLE.with(|v| v.set(false));
}

/// The placeholder `set_size` and the `box_size` field must agree.
const INITIAL_BOX_H: f32 = 300.0;

/// Instantiated lazily by the C# shim; the class name must stay exactly
/// `SpireProfilerRunPanel`.
pub struct SpireProfilerRunPanel {
    object: Object,
    children: panel_body::PanelChildren,
    view_fp: u64,
    /// False until the first build, so the empty notice renders initially.
    layout_valid: bool,
    layout: RunLayout,
    /// Kept so a hover change resolves the row's detail text without
    /// rebuilding the snapshot.
    rows: [UiRow; ui_model::MAX_UI_ROWS],
    hover_row: Option<usize>,
    detail: RowDetail,
    interaction: InteractionState,
    /// The rows child draws translated by this offset; its rect clips.
    scroll: f32,
    /// Input arrives as events while the offset only moves per frame.
    pending_scroll: f32,
    box_size: Vector2,
    /// Stored so the frame step can widen the Control without re-reading
    /// the engine. Parent-relative == viewport space here.
    plate_pos: Option<Vector2>,
    applied_frame: Option<Rect2>,
    origin_x: f32,
    legend: Option<Rect2>,
    legend_cmds: Vec<crate::ui::chart_layout::Cmd>,
    tip: Option<Rect2>,
    tip_lines: Vec<crate::ui::tooltip::TipLine>,
    /// Nothing signals a resize, so the panel polls.
    viewport_seen: Option<Vector2>,
    mouse: Vector2,
    font: panel_replay::FontState,
    font_plan: panel_replay::FontPlan,
    theme: crate::ui::theme::Theme,
    gutter: f32,
    /// Roster slots parallel to the layout's `portrait_paths`, for the
    /// per-frame dim mask.
    avatar_slots: Vec<u8>,
    avatar_animation: AvatarScaleAnimation,
    dimmed_scratch: Vec<bool>,
}

impl SpireProfilerRunPanel {
    /// Deliberately silent: the headless gate counts `[SpireProfiler]`
    /// boot lines.
    pub(crate) fn new(object: Object) -> Self {
        let box_size = Vector2::new(WIDTH, INITIAL_BOX_H);
        object.set_size(box_size);
        object.set_visible(false);
        object.set_clip_contents(true);
        Self {
            object,
            children: panel_body::PanelChildren::default(),
            view_fp: 0,
            layout_valid: false,
            layout: RunLayout::default(),
            rows: [UiRow::default(); ui_model::MAX_UI_ROWS],
            hover_row: None,
            detail: RowDetail::default(),
            interaction: InteractionState::default(),
            scroll: 0.0,
            pending_scroll: 0.0,
            box_size,
            plate_pos: None,
            applied_frame: None,
            origin_x: 0.0,
            legend: None,
            legend_cmds: Vec::new(),
            tip: None,
            tip_lines: Vec::new(),
            viewport_seen: None,
            mouse: Vector2::ZERO,
            font: panel_replay::FontState::Unfetched,
            font_plan: panel_replay::FontPlan::default(),
            theme: crate::ui::theme::Theme::new(),
            gutter: 0.0,
            avatar_slots: Vec::new(),
            avatar_animation: AvatarScaleAnimation::default(),
            dimmed_scratch: Vec::new(),
        }
    }

    /// Registration runs this once the boxed state address is stable.
    pub(crate) fn attach_children(&mut self) {
        self.children =
            panel_body::PanelChildren::attach(self.object, panel_body::OwnerRef::Run(self));
    }

    /// A content change redraws every panel-owned canvas item.
    fn queue_panel_redraw(&mut self) {
        self.object.queue_redraw();
        self.children.queue_redraw();
    }

    /// Call errors are dropped rather than counted: this class must stay
    /// silent (no boot markers).
    pub(crate) fn draw(&mut self) {
        if self.theme.resolve() {
            self.layout_valid = false;
        }
        let mut icons_changed = false;
        for path in &self.layout.portrait_paths {
            icons_changed |= self.theme.resolve_dynamic(path);
        }
        if icons_changed {
            self.layout_valid = false;
        }
        // The dim mask is parallel to the icon row: excluded players
        // render at the game's deselect modulate. Runs before `fonts`
        // borrows `self.font`.
        self.resolve_dim_mask();
        let fonts = panel_replay::Fonts::new(
            &self.object,
            &mut self.font,
            &self.theme,
            self.font_plan,
            "run panel font unavailable; text disabled",
        );
        let icons = panel_replay::IconTextures {
            theme: &self.theme,
            portraits: &self.layout.portrait_paths,
            dimmed: &self.dimmed_scratch,
            scales: self.avatar_animation.values(),
        };
        let plate = self.theme.plate();
        if let Some(plate) = plate {
            let _ =
                panel_replay::draw_plate(&self.object, plate, self.origin_x, self.box_size, 0.0);
        } else {
            panel_replay::draw_flat_background(
                &self.object,
                self.origin_x,
                self.layout.width,
                self.box_size.y,
                0.0,
            );
        }
        let _ = panel_replay::replay_cmds(
            &self.object,
            &fonts,
            &self.layout.header_cmds,
            Vector2::new(self.origin_x, 0.0),
            &icons,
        );
    }

    fn resolve_dim_mask(&mut self) {
        let filter = crate::data::run_history::run_filter();
        panel_common::fill_dim_mask(
            self.layout.portrait_paths.len(),
            &self.avatar_slots,
            filter,
            &mut self.dimmed_scratch,
        );
    }

    /// The rows child's `_draw`; commands are translated into child space.
    pub(crate) fn draw_body(&mut self, body: &Object) {
        let fonts = panel_replay::Fonts::new(
            &self.object,
            &mut self.font,
            &self.theme,
            self.font_plan,
            "run panel font unavailable; text disabled",
        );
        let icons = panel_replay::IconTextures {
            theme: &self.theme,
            portraits: &self.layout.portrait_paths,
            dimmed: &self.dimmed_scratch,
            scales: self.avatar_animation.values(),
        };
        panel_replay::replay_cmds(
            body,
            &fonts,
            &self.layout.cmds,
            Vector2::new(0.0, -(self.layout.header_bottom + self.scroll)),
            &icons,
        );
    }

    /// The overlay child's `_draw`.
    pub(crate) fn draw_overlay(&mut self, overlay: &Object) {
        let scrollbar_sprites = self.theme.scrollbar();
        let scrollbar_geom = self.scrollbar_geom(self.box_size);
        let fonts = panel_replay::Fonts::new(
            &self.object,
            &mut self.font,
            &self.theme,
            self.font_plan,
            "run panel font unavailable; text disabled",
        );
        let _ = panel_replay::draw_overlays(
            overlay,
            &fonts,
            self.theme.plate(),
            scrollbar_sprites.zip(scrollbar_geom),
            self.origin_x,
            self.legend,
            &mut self.legend_cmds,
            self.tip,
            &self.tip_lines,
        );
    }

    fn scrollbar_geom(&self, box_size: Vector2) -> Option<crate::ui::scroll::ScrollbarGeom> {
        panel_common::scrollbar_geom(
            &self.theme,
            box_size,
            self.layout.header_bottom,
            self.layout.height,
            self.scroll,
        )
    }

    fn apply_modal_geometry(&mut self) {
        let viewport = panel_common::viewport_size(&self.object);
        let (size, pos) = panel_common::modal_box(
            viewport,
            self.layout.width,
            self.layout.height,
            &mut self.scroll,
        );
        self.box_size = size;
        self.plate_pos = pos;
    }

    /// The side plates are mouse-transparent in the game's idiom, so a
    /// press on one dismisses like any other outside press.
    fn plate_rect(&self) -> Option<Rect2> {
        self.plate_pos.map(|pos| Rect2::new(pos, self.box_size))
    }

    /// Runs before the dirty-check early-outs: a scroll moves the hovered
    /// row's screen y without dirtying the fingerprint.
    fn update_frame(&mut self, hover: Option<usize>) {
        let Some(plate_rect) = self.plate_rect() else {
            return;
        };
        let legend = if self.layout.has_chart {
            self.viewport_seen.map(|viewport| {
                let lp = crate::ui::palette::legend_plate(self.theme.plate().is_some());
                crate::ui::tooltip::place_legend(viewport, plate_rect, lp.size)
            })
        } else {
            None
        };
        let tip = hover.and_then(|row| {
            let viewport = self.viewport_seen?;
            if self.tip_lines.is_empty() {
                return None;
            }
            let hit = self
                .layout
                .row_hits
                .iter()
                .find(|hit| hit.flat_index == row)?;
            let row_y = plate_rect.position.y + hit.y0 - self.scroll;
            let size = Vector2::new(
                crate::ui::tooltip::TIP_WIDTH,
                crate::ui::tooltip::tip_height(self.tip_lines.len()),
            );
            Some(crate::ui::tooltip::place(
                viewport, plate_rect, row_y, size, legend,
            ))
        });
        let (frame, origin_x) = crate::ui::tooltip::frame(plate_rect, legend, tip);
        panel_common::apply_control_frame(&self.object, frame, &mut self.applied_frame);
        self.origin_x = origin_x;
        let body_frame = panel_common::body_frame(
            self.origin_x,
            self.box_size,
            self.theme.plate().is_some(),
            self.layout.header_bottom,
        );
        self.children.update_frames(frame, body_frame);
        let legend = legend.map(|rect| Rect2::new(rect.position - frame.position, rect.size));
        if legend != self.legend {
            self.legend = legend;
            self.object.queue_redraw();
            self.children.queue_overlay_redraw();
        }
        let tip = tip.map(|tip| Rect2::new(tip.position - frame.position, tip.size));
        if tip != self.tip {
            self.tip = tip;
            self.object.queue_redraw();
            self.children.queue_overlay_redraw();
        }
    }

    /// An unmapped id yields no portrait, never a guess.
    fn header_facts(&self, view: &RunSummaryView) -> HeaderFacts {
        let portraits = roster_entries(view)
            .into_iter()
            .filter_map(|(slot, id)| {
                let path = character_icon_path(id)?;
                Some(PortraitFact {
                    slot,
                    loaded: self.theme.dynamic(&path).is_some(),
                    path,
                })
            })
            .collect();
        HeaderFacts { portraits }
    }

    /// A `&RunSummaryView` cannot escape the thread-local RefCell, so the
    /// fingerprint token comes out of the borrow instead.
    pub(crate) fn refresh(&mut self) {
        // Queued input must never linger past one frame.
        self.pending_scroll += take_queued_scroll();
        let open = crate::data::run_history::screen_open();
        let visible = open && run_manual_visible();
        self.object.set_visible(visible);
        if !visible {
            // A stale baked-in highlight must never survive a hide cycle.
            self.hover_row = None;
            self.detail = RowDetail::default();
            self.legend = None;
            self.tip = None;
            self.tip_lines.clear();
            self.layout_valid = false;
            self.avatar_animation.clear();
            return;
        }
        let viewport = panel_common::viewport_size(&self.object);
        if viewport != self.viewport_seen {
            self.viewport_seen = viewport;
            self.apply_modal_geometry();
            self.tip_lines = panel_common::reshape_tip(&self.detail, self.box_size.y);
        }
        let Some(plate_rect) = self.plate_rect() else {
            return;
        };
        let gutter = panel_common::scrollbar_gutter(
            self.layout.height,
            self.box_size.y,
            self.theme.scrollbar().is_some(),
        );
        if gutter != self.gutter {
            self.gutter = gutter;
            self.layout_valid = false;
        }
        panel_common::viewport_mouse(&self.object, &mut self.mouse);
        let mouse = self.mouse;
        self.interaction(plate_rect, mouse);

        panel_common::wheel_scroll(
            self.children.objects(),
            &mut self.scroll,
            &mut self.pending_scroll,
            plate_rect,
            mouse,
            self.layout.height,
        );

        // Gated to the body band — the pinned header has no rows.
        let band = panel_replay::body_band(
            self.box_size.y,
            self.theme.plate().is_some(),
            self.layout.header_bottom,
        );
        let hover =
            panel_common::hover_row(&self.layout.row_hits, plate_rect, mouse, self.scroll, band);

        // The tip must track a scroll without a fingerprint change.
        self.update_frame(hover);
        let filter = crate::data::run_history::run_filter();
        self.avatar_animation
            .advance_frame(&self.object, filter, &self.avatar_slots);

        let fp = crate::data::run_history::selected_view_fingerprint().unwrap_or(0)
            ^ crate::data::run_history::run_filter_fingerprint();
        if self.layout_valid && fp == self.view_fp && hover == self.hover_row {
            return;
        }
        self.view_fp = fp;
        self.layout_valid = true;
        self.hover_row = hover;
        self.rebuild(hover);
    }

    fn rebuild(&mut self, hover: Option<usize>) {
        // The icon row only renders slots present in the displayed run;
        // a stale filter heals to All before the layout bakes the state.
        // The heal dirties the next frame's fingerprint, so at most one
        // extra rebuild follows.
        crate::data::run_history::heal_run_filter();
        let view = crate::data::run_history::selected_view();
        let header = view
            .as_ref()
            .map_or_else(HeaderFacts::default, |v| self.header_facts(v));
        self.avatar_slots = header.portraits.iter().map(|p| p.slot).collect();
        let filter = crate::data::run_history::run_filter();
        self.avatar_animation
            .set_targets(filter, &self.avatar_slots);
        self.layout = build_run_layout(
            view.as_ref(),
            &header,
            hover,
            &mut self.rows,
            WIDTH,
            self.theme.plate().is_none(),
            self.gutter,
        );
        // Never the live state: the screen can show a run from any session.
        let detail_cards: &[CardStat] = view
            .as_ref()
            .map_or(&[][..], |v| crate::data::run_history::filtered_rollup(v));
        self.detail = hover.map_or_else(RowDetail::default, |row| {
            crate::ui::snapshot::ui_row_detail_from_cards(
                &self.rows[..self.layout.chart_rows],
                row,
                detail_cards,
            )
        });
        self.font_plan =
            panel_replay::FontPlan::scan(&self.layout.header_cmds, &self.layout.cmds, &self.detail);
        self.apply_modal_geometry();
        self.tip_lines = panel_common::reshape_tip(&self.detail, self.box_size.y);
        self.update_frame(hover);
        self.queue_panel_redraw();
    }

    fn interaction(&mut self, rect: Rect2, mouse: Vector2) {
        let scrollbar = panel_common::ScrollbarFrame {
            geom: self.scrollbar_geom(rect.size),
            content_height: self.layout.height,
        };
        let was_down = self.interaction.mouse_down;
        // The icon row sits in the pinned header, so zones resolve
        // box-local like the combat panel's tabs.
        let zone = panel_common::guarded_zone(rect, mouse, |local_x, local_y| {
            chart_layout::avatar_at(&self.layout.avatar_hits, local_x, local_y)
                .map_or(PressZone::Inert, PressZone::Avatar)
        });
        let step = panel_common::interaction_step(
            self.children.objects(),
            rect,
            mouse,
            &mut self.interaction,
            scrollbar,
            &mut self.scroll,
        );
        if panel_common::dismiss_on_outside_press(
            true,
            step.pressed && !was_down,
            panel_common::over_panel(rect, mouse),
        ) {
            dismiss_run_manual();
        }
        if step.pressed
            && !was_down
            && !step.on_track
            && let PressZone::Avatar(slot) = zone
        {
            // The toggle itself is the state transition: pressing the
            // active avatar returns to All, any other selects it.
            crate::data::run_history::toggle_run_filter(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_manual_visible_cycles() {
        RUN_MANUAL_VISIBLE.with(|v| v.set(false));
        assert!(!run_manual_visible());
        toggle_run_manual();
        assert!(run_manual_visible());
        toggle_run_manual();
        assert!(!run_manual_visible());
    }

    #[test]
    fn dismiss_run_manual_lands_on_hidden() {
        RUN_MANUAL_VISIBLE.with(|v| v.set(true));
        dismiss_run_manual();
        assert!(!run_manual_visible());
        toggle_run_manual();
        assert!(run_manual_visible());
        dismiss_run_manual();
        assert!(!run_manual_visible());
    }
}
