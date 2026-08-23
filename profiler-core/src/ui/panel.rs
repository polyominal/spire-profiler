//! The combat chart panel: a GDExtension class rendering the segmented
//! contribution chart with immediate-mode drawing. All layout math lives
//! in [`chart_layout`]; this file is engine plumbing + interaction.
//!
//! Scroll input arrives from the C# shim through `spire_profiler_scroll_input`
//! (the extension must never inspect an engine input event); the left
//! button is polled for tabs and the scrollbar track. The parent is a plain
//! Control, not PanelContainer: the engine runs `_draw` BEFORE
//! NOTIFICATION_DRAW, so a PanelContainer's stylebox would paint over the
//! chart.

use std::cell::Cell;
use std::hash::{Hash, Hasher};

use crate::data::state::{CardStat, PlayerFilter, STATE};
use crate::engine::gdext::Object;
use crate::engine::math::{Rect2, Vector2};
use crate::ui::chart_layout::{self, Layout};
use crate::ui::panel_common::{self, InteractionState, PressZone};
use crate::ui::tooltip::RowDetail;
use crate::ui::ui_model::{self, UiMeta, UiRow, UiTab};
use crate::ui::{panel_replay, snapshot};

thread_local! {
    static VISIBLE: Cell<bool> = const { Cell::new(false) };
    /// Keeps the panel drawable after the synthetic run ends: the boot's
    /// draw gates run after `self_test` completed.
    static SELFTEST_FORCE: Cell<bool> = const { Cell::new(false) };
    /// Module scope because the flat C export cannot address the panel
    /// instance.
    static QUEUED_SCROLL: Cell<f32> = const { Cell::new(0.0) };
}

pub(crate) fn queue_scroll(delta: f32) {
    QUEUED_SCROLL.with(|c| c.set(c.get() + delta));
}

pub(crate) fn take_queued_scroll() -> f32 {
    QUEUED_SCROLL.with(|c| c.replace(0.0))
}

pub(crate) fn visible() -> bool {
    VISIBLE.with(|v| v.get())
}

pub(crate) fn run_active() -> bool {
    STATE.with(|s| s.borrow().run_ctx.active)
}

/// Outside a run the press is ignored; the stored state carries forward.
pub(crate) fn toggle() {
    if !run_active() {
        eprintln!("[SpireProfiler] panel toggle: ignored (outside a run)");
        return;
    }
    let next = VISIBLE.with(|v| !v.get());
    VISIBLE.set(next);
}

pub(crate) fn dismiss() {
    VISIBLE.with(|v| v.set(false));
}

/// A hidden Control never dispatches `_draw`.
pub(crate) fn enable_for_selftest() {
    SELFTEST_FORCE.set(true);
    if !visible() {
        toggle();
    }
}

/// The class name must stay `SpireProfilerPanel`.
pub struct SpireProfilerPanel {
    object: Object,
    rows: [UiRow; ui_model::MAX_UI_ROWS],
    row_count: usize,
    meta: UiMeta,
    footer: String,
    detail: RowDetail,
    layout: Layout,
    /// Rebuild only when the snapshot hash changes.
    sig: Option<u64>,
    /// The frame returns before building the snapshot when this repeats.
    cheap_sig: u64,
    active_tab: UiTab,
    hover_row: Option<usize>,
    interaction: InteractionState,
    scroll: f32,
    /// Input arrives as events, the offset moves per frame.
    pending_scroll: f32,
    box_size: Vector2,
    /// Parent-relative == viewport space (a scene-root child).
    plate_pos: Option<Vector2>,
    applied_frame: Option<Rect2>,
    origin_x: f32,
    legend: Option<Rect2>,
    legend_cmds: Vec<crate::ui::chart_layout::Cmd>,
    tip: Option<Rect2>,
    tip_lines: Vec<crate::ui::tooltip::TipLine>,
    viewport_seen: Option<Vector2>,
    mouse: Vector2,
    font: panel_replay::FontState,
    theme: crate::ui::theme::Theme,
    gutter: f32,
    /// Roster slots parallel to the layout's `portrait_paths` (only the
    /// combat tab carries a row), for the per-frame dim mask.
    avatar_slots: Vec<u8>,
    dimmed_scratch: Vec<bool>,
    logged_draw: bool,
    /// The `chart draw ok` marker proves clean engine calls, not just entry.
    draw_ok_logged: bool,
}

impl SpireProfilerPanel {
    pub(crate) fn new(object: Object) -> Self {
        let box_size = Vector2::new(chart_layout::PANEL_WIDTH, 200.0);
        object.set_size(box_size);
        object.set_visible(false);
        object.set_clip_contents(true);
        eprintln!("[SpireProfiler] panel instance created");
        Self {
            object,
            rows: [UiRow::default(); ui_model::MAX_UI_ROWS],
            row_count: 0,
            meta: UiMeta::default(),
            footer: String::new(),
            detail: RowDetail::default(),
            layout: Layout::default(),
            sig: None,
            cheap_sig: 0,
            active_tab: UiTab::Combat,
            hover_row: None,
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
            theme: crate::ui::theme::Theme::new(),
            gutter: 0.0,
            avatar_slots: Vec::new(),
            dimmed_scratch: Vec::new(),
            logged_draw: false,
            draw_ok_logged: false,
        }
    }

    /// Replays the body (translated, scissored), then the pinned header
    /// over it — the floating tab strip, title, avatar row, and meta line
    /// never move.
    pub(crate) fn draw(&mut self) {
        if self.sig.is_none() {
            return;
        }
        self.log_draw_start();

        // Engine calls never borrow `&mut self`.
        if self.theme.resolve() {
            // Newly loaded/failed assets change the chrome.
            self.sig = None;
        }
        if self.resolve_avatars() {
            // Newly loaded roster avatars change the header.
            self.sig = None;
        }
        // Hoisted ahead of `fonts` to avoid a second mutable borrow.
        let scrollbar_sprites = self.theme.scrollbar();
        let scrollbar_geom = self.scrollbar_geom(self.box_size);
        let fonts = panel_replay::Fonts::new(
            &self.object,
            &mut self.font,
            &self.theme,
            &self.layout.header_cmds,
            &self.layout.cmds,
            &self.detail,
            "[SpireProfiler] WARNING: theme default font unavailable; chart text disabled",
        );
        // Count failures so `chart draw ok` fires only after a clean draw.
        let mut call_errors = 0;
        let plate = self.theme.plate();
        call_errors += draw_pinned_chrome(
            &self.object,
            plate,
            self.origin_x,
            self.layout.width,
            self.box_size,
            self.layout.strip_h,
        );
        let icons = panel_replay::IconTextures {
            theme: &self.theme,
            portraits: &self.layout.portrait_paths,
            dimmed: &self.dimmed_scratch,
        };
        call_errors += panel_replay::replay_split(
            &self.object,
            &fonts,
            &self.layout.header_cmds,
            &self.layout.cmds,
            self.layout.header_bottom,
            self.layout.width,
            plate.is_some(),
            self.origin_x,
            self.scroll,
            self.box_size.y,
            &icons,
        );
        call_errors += panel_replay::draw_overlays(
            &self.object,
            &fonts,
            plate,
            scrollbar_sprites.zip(scrollbar_geom),
            self.origin_x,
            self.legend,
            &mut self.legend_cmds,
            self.tip,
            &self.tip_lines,
        );
        self.log_draw_ok(call_errors);
    }

    /// One-shot entry marker: the headless gate greps `chart _draw
    /// active`, and a repeated line would be noise.
    fn log_draw_start(&mut self) {
        if self.logged_draw {
            return;
        }
        self.logged_draw = true;
        eprintln!(
            "[SpireProfiler] chart _draw active: {} cmds, {} rows",
            self.layout.cmds.len() + self.layout.header_cmds.len(),
            self.layout.row_hits.len()
        );
    }

    /// The `chart draw ok` marker proves clean engine calls, not just
    /// entry.
    fn log_draw_ok(&mut self, call_errors: usize) {
        if !self.draw_ok_logged && call_errors == 0 {
            self.draw_ok_logged = true;
            eprintln!(
                "[SpireProfiler] chart draw ok: {} cmds, 0 call errors",
                self.layout.cmds.len() + self.layout.header_cmds.len()
            );
        }
    }

    /// Rebuilds only on change, pre-checked by a cheap state hash so an
    /// unchanged frame does no snapshot work.
    pub(crate) fn refresh(&mut self) {
        // Consume the shim-forwarded scroll queue before any early return
        // (including the hidden frame): queued input must never linger past
        // one frame, and the over-panel guard in `wheel_scroll` still
        // governs whether the accumulated pixels apply.
        self.pending_scroll += take_queued_scroll();
        // Plain F8 toggle, effective only inside a run; the stored state
        // carries into the next run. The self-test force is the exception.
        let shown = visible() && (run_active() || SELFTEST_FORCE.get());
        self.object.set_visible(shown);
        if !shown {
            self.sig = None;
            self.hover_row = None;
            self.legend = None;
            self.tip = None;
            self.tip_lines.clear();
            return;
        }

        let viewport = panel_common::viewport_size(&self.object);
        if viewport != self.viewport_seen {
            self.viewport_seen = viewport;
            self.apply_modal_geometry();
            self.reshape_tip();
        }
        // Presses, wheel input, and hover treat the strip as part of the
        // panel (a tab press must not dismiss, and every hit table is
        // Control-local). The guard doubles as the not-yet-placed check.
        let Some(control_rect) = self.control_rect() else {
            return;
        };
        let gutter = self.scrollbar_gutter();
        if gutter != self.gutter {
            self.gutter = gutter;
            self.sig = None;
        }
        let mouse = self.mouse_position();
        self.interaction(control_rect, mouse);
        self.apply_wheel_scroll(control_rect, mouse);

        // Hover maps through the row hit table, gated to the body band:
        // the pinned header has tab zones, not rows. The hits are
        // Control-local (the strip offset is baked in), so the mouse
        // converts through the Control rect, not the plate rect.
        let band = panel_replay::body_band(
            self.box_size.y,
            self.theme.plate().is_some(),
            self.layout.header_bottom,
        );
        let hover = panel_common::hover_row(
            &self.layout.row_hits,
            control_rect,
            mouse,
            self.scroll,
            band,
        );

        // The tooltip resolves before the dirty-check early-outs: a
        // scroll moves the hovered row's screen y without dirtying the
        // signature.
        self.update_frame(hover);

        // Cheap dirty check first: hash snapshot-relevant state plus the
        // hover target, so an unchanged hash ends the frame here.
        let cheap = cheap_frame_signature(self.active_tab, hover);
        if self.sig.is_some() && cheap == self.cheap_sig {
            return;
        }
        self.rebuild(hover, cheap);
    }

    fn rebuild(&mut self, hover: Option<usize>, cheap: u64) {
        let mut rows_scratch = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let n = snapshot::ui_snapshot_rows(self.active_tab, &mut rows_scratch);
        let meta = snapshot::ui_snapshot_meta(self.active_tab);
        let footer = snapshot::ui_footer_text(self.active_tab);
        let detail = hover.map_or_else(RowDetail::default, |row| {
            // The rows were just built above; passing them keeps a hover
            // change from rebuilding the snapshot a second time.
            snapshot::ui_row_detail_from_rows(self.active_tab, &rows_scratch[..n], row)
        });

        let sig = content_signature(
            &rows_scratch[..n],
            meta,
            &footer,
            &detail,
            self.active_tab,
            hover,
        );
        self.cheap_sig = cheap;
        if self.sig == Some(sig) {
            return;
        }
        self.sig = Some(sig);

        self.rows[..n].copy_from_slice(&rows_scratch[..n]);
        self.row_count = n;
        self.meta = meta;
        self.footer = footer;
        self.detail = detail;
        self.hover_row = hover;

        // The avatar row lists the combat's roster; paths resolve through
        // the theme's dynamic cache, and unloaded avatars collapse the
        // row (the filter then stays All — degraded, never broken).
        let avatars = self.avatar_facts();
        self.avatar_slots = avatars.iter().map(|a| a.slot).collect();

        let layout = chart_layout::build(chart_layout::BuildInput {
            tab: self.active_tab,
            rows: &self.rows[..self.row_count],
            meta: self.meta,
            footer: &self.footer,
            hover_row: self.hover_row,
            skip_chrome: false,
            avatars: &avatars,
            // The nine-patch plate is the chrome; the flat rects the fallback.
            flat_chrome: self.theme.plate().is_none(),
            tab_sprites: self.theme.tab_sprites(),
            width: chart_layout::PANEL_WIDTH,
            right_gutter: self.gutter,
        });
        self.layout = layout;
        // The tip's line cap comes from the box height: a tip capped to a
        // stale (larger) box could exceed the new y-band.
        self.apply_modal_geometry();
        self.reshape_tip();
        self.update_frame(hover);
        self.object.queue_redraw();
    }

    /// Fixed row heights mean the gutter can never change the overflow
    /// verdict and oscillate.
    fn scrollbar_gutter(&self) -> f32 {
        if self.layout.height > self.box_size.y && self.theme.scrollbar().is_some() {
            crate::ui::scroll::GUTTER
        } else {
            0.0
        }
    }

    /// An unmapped character id yields no avatar, never a guess.
    fn avatar_facts(&self) -> Vec<chart_layout::AvatarFact> {
        STATE.with(|s| {
            let st = s.borrow();
            let Some(c) = &st.current else {
                return Vec::new();
            };
            c.players
                .iter()
                .filter_map(|p| {
                    let path = crate::ui::run_layout::character_icon_path(&p.character)?;
                    Some(chart_layout::AvatarFact {
                        slot: p.slot,
                        loaded: self.theme.dynamic(&path).is_some(),
                        path,
                    })
                })
                .collect()
        })
    }

    /// Resolves the roster avatars' textures and the per-frame dim mask
    /// (excluded players render at the game's deselect modulate). Returns
    /// whether a load state changed, so the header can rebuild.
    fn resolve_avatars(&mut self) -> bool {
        let mut changed = false;
        for path in &self.layout.portrait_paths {
            changed |= self.theme.resolve_dynamic(path);
        }
        self.dimmed_scratch.clear();
        let filter = STATE.with(|s| s.borrow().player_filter);
        if self.layout.portrait_paths.len() == self.avatar_slots.len() {
            for &slot in &self.avatar_slots {
                let dimmed = filter != PlayerFilter::All && filter != PlayerFilter::Player(slot);
                self.dimmed_scratch.push(dimmed);
            }
        }
        changed
    }

    /// None when the bar is hidden (content fits, or sprites failed).
    fn scrollbar_geom(&self, box_size: Vector2) -> Option<crate::ui::scroll::ScrollbarGeom> {
        self.theme.scrollbar()?;
        let plate = self.theme.plate().is_some();
        crate::ui::scroll::scrollbar_geom(
            box_size,
            plate,
            panel_replay::body_band(box_size.y, plate, self.layout.header_bottom),
            self.layout.height,
            self.scroll,
        )
    }

    /// A scroll moves pixels without touching the content signature, so
    /// the dirty checks must not be the ones to catch it.
    fn apply_wheel_scroll(&mut self, rect: Rect2, mouse: Vector2) {
        panel_common::wheel_scroll(
            &self.object,
            &mut self.scroll,
            &mut self.pending_scroll,
            rect,
            mouse,
            self.layout.height,
        );
    }

    /// The Control rect itself is applied later by the frame step, which
    /// widens it around the side plates.
    fn apply_modal_geometry(&mut self) {
        let viewport = panel_common::viewport_size(&self.object);
        let (size, pos) = panel_common::modal_box(
            viewport,
            self.layout.width,
            self.layout.height,
            &mut self.scroll,
        );
        // The tab strip extends the Control upward; the plate sits below
        // it. modal_box already centered the strip+plate assembly (the
        // size includes the strip via layout.height).
        self.box_size = size;
        self.plate_pos = pos.map(|p| p + Vector2::new(0.0, self.layout.strip_h));
    }

    /// The side plates are mouse-transparent in the game's idiom, so a
    /// press on one dismisses like any other outside press.
    fn plate_rect(&self) -> Option<Rect2> {
        let strip = Vector2::new(0.0, self.layout.strip_h);
        self.plate_pos
            .map(|pos| Rect2::new(pos, self.box_size - strip))
    }

    /// The plate plus the floating tab strip: the zone whose presses are
    /// "inside the panel", so a tab press never dismisses.
    fn control_rect(&self) -> Option<Rect2> {
        let strip = Vector2::new(0.0, self.layout.strip_h);
        self.plate_pos
            .map(|pos| Rect2::new(pos - strip, self.box_size))
    }

    fn reshape_tip(&mut self) {
        self.tip_lines = if self.detail.is_empty() {
            Vec::new()
        } else {
            crate::ui::tooltip::shape(
                &self.detail,
                crate::ui::tooltip::max_tip_lines(self.box_size.y),
            )
        };
    }

    /// Runs before the dirty-check early-outs: a scroll moves the hovered
    /// row's screen y without dirtying the signature.
    fn update_frame(&mut self, hover: Option<usize>) {
        let Some(plate_rect) = self.plate_rect() else {
            return;
        };
        let legend = self.viewport_seen.map(|viewport| {
            let lp = crate::ui::palette::legend_plate(self.theme.plate().is_some());
            crate::ui::tooltip::place_legend(viewport, plate_rect, lp.size)
        });
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
            // The hit y is Control-local (the strip offset is baked in),
            // so the row's screen y anchors on the Control's top.
            let row_y = plate_rect.position.y - self.layout.strip_h + hit.y0 - self.scroll;
            let size = Vector2::new(
                crate::ui::tooltip::TIP_WIDTH,
                crate::ui::tooltip::tip_height(self.tip_lines.len()),
            );
            Some(crate::ui::tooltip::place(
                viewport, plate_rect, row_y, size, legend,
            ))
        });
        let (frame, origin_x) = crate::ui::tooltip::frame(plate_rect, legend, tip);
        // The frame keeps the plate's y origin by design; the strip band
        // extends the Control upward on top of it.
        let strip = Vector2::new(0.0, self.layout.strip_h);
        let frame = Rect2::new(frame.position - strip, frame.size + strip);
        panel_common::apply_control_frame(&self.object, frame, &mut self.applied_frame);
        self.origin_x = origin_x;
        let legend = legend.map(|rect| Rect2::new(rect.position - frame.position, rect.size));
        if legend != self.legend {
            self.legend = legend;
            self.object.queue_redraw();
        }
        let tip = tip.map(|tip| Rect2::new(tip.position - frame.position, tip.size));
        if tip != self.tip {
            self.tip = tip;
            self.object.queue_redraw();
        }
    }

    /// Tab clicks switch tabs; a press outside the panel (plate + strip)
    /// dismisses.
    fn interaction(&mut self, rect: Rect2, mouse: Vector2) {
        // The tabs sit in the floating strip, so zones resolve box-local
        // against the whole Control.
        let zone = panel_common::guarded_zone(rect, mouse, |local_x, local_y| {
            press_zone(&self.layout, local_x, local_y)
        });
        let was_down = self.interaction.mouse_down;
        let scrollbar = panel_common::ScrollbarFrame {
            geom: self.scrollbar_geom(rect.size),
            content_height: self.layout.height,
        };
        let step = panel_common::interaction_step(
            &self.object,
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
            dismiss();
        }
        if step.pressed
            && !was_down
            && !step.on_track
            && let PressZone::Tab(tab) = zone
            && tab != self.active_tab
        {
            self.active_tab = tab;
            self.sig = None; // rebuild from the other dataset
        }
        if step.pressed
            && !was_down
            && !step.on_track
            && let PressZone::Avatar(slot) = zone
        {
            // The toggle itself is the state transition: pressing the
            // active avatar returns to All, any other selects it.
            crate::data::events::panel_filter_toggle(slot);
            self.sig = None; // rebuild with the filter applied
        }
    }

    fn mouse_position(&mut self) -> Vector2 {
        panel_common::viewport_mouse(&self.object, &mut self.mouse);
        self.mouse
    }
}

/// A free function, not a method: `draw` holds the font state's borrow.
/// `top_band` is the floating tab strip's height: the plate starts below
/// it, and the strip band above it shows the dimmer (plate mode) or the
/// pinned header fill (flat mode).
fn draw_pinned_chrome(
    object: &Object,
    plate: Option<&crate::ui::theme::Plate>,
    origin_x: f32,
    layout_width: f32,
    box_size: Vector2,
    top_band: f32,
) -> usize {
    if let Some(plate) = plate {
        return panel_replay::draw_plate(object, plate, origin_x, box_size, top_band);
    }
    panel_replay::draw_flat_background(object, origin_x, layout_width, box_size.y, top_band);
    0
}

/// The tab strip is pinned, so the coordinates need no scroll translation.
pub(crate) fn press_zone(layout: &Layout, local_x: f32, local_y: f32) -> PressZone {
    if let Some(tab) = chart_layout::tab_at(layout, local_x, local_y) {
        return PressZone::Tab(tab);
    }
    if let Some(slot) = chart_layout::avatar_at(&layout.avatar_hits, local_x, local_y) {
        return PressZone::Avatar(slot);
    }
    PressZone::Inert
}

/// Field-wise hashing: whole-struct hashing would flap on padding bytes.
fn content_signature(
    rows: &[UiRow],
    meta: UiMeta,
    footer: &str,
    detail: &RowDetail,
    tab: UiTab,
    hover: Option<usize>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for row in rows {
        row.section.hash(&mut hasher);
        row.kind.hash(&mut hasher);
        row.player.hash(&mut hasher);
        row.flags.hash(&mut hasher);
        row.plays.hash(&mut hasher);
        row.value.hash(&mut hasher);
        row.share_x10.hash(&mut hasher);
        row.seg_milli.hash(&mut hasher);
        row.name_str().hash(&mut hasher);
    }
    meta.turns.hash(&mut hasher);
    meta.plays.hash(&mut hasher);
    meta.combats.hash(&mut hasher);
    meta.total_damage.hash(&mut hasher);
    meta.damage_taken.hash(&mut hasher);
    meta.dps_x10.hash(&mut hasher);
    meta.encounter_str().hash(&mut hasher);
    footer.hash(&mut hasher);
    detail.hash(&mut hasher);
    (tab as u8).hash(&mut hasher);
    hover.map_or(-1i64, |h| h as i64).hash(&mut hasher);
    hasher.finish()
}

/// [`cheap_state_signature`] plus the hover target.
fn cheap_frame_signature(tab: UiTab, hover: Option<usize>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cheap_state_signature(tab).hash(&mut hasher);
    hover.map_or(-1i64, |h| h as i64).hash(&mut hasher);
    hasher.finish()
}

fn hash_card_stat(hasher: &mut std::collections::hash_map::DefaultHasher, card: &CardStat) {
    card.player.hash(hasher);
    card.id.hash(hasher);
    card.kind.hash(hasher);
    card.plays.hash(hasher);
    card.damage_dealt.hash(hasher);
    card.damage_blocked.hash(hasher);
    card.block_gained.hash(hasher);
    card.block_effective.hash(hasher);
    card.forge.hash(hasher);
    card.dmg_direct.hash(hasher);
    card.dmg_attributed.hash(hasher);
    card.dmg_modifier.hash(hasher);
    card.dmg_upgrade.hash(hasher);
    card.blk_modifier.hash(hasher);
    card.blk_upgrade.hash(hasher);
    card.mitigate_debuff.hash(hasher);
    card.mitigate_buff.hash(hasher);
    card.mitigate_str.hash(hasher);
    card.self_damage.hash(hasher);
}

/// Every field that can reach the payloads feeds this hash, so the
/// early-out can never skip a redraw.
fn cheap_state_signature(tab: UiTab) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    STATE.with(|s| {
        let st = s.borrow();
        st.initialized.hash(&mut hasher);
        (tab as u8).hash(&mut hasher);
        match tab {
            UiTab::Combat => {
                let Some(c) = &st.current else {
                    return;
                };
                st.player_filter.hash(&mut hasher);
                c.seq.hash(&mut hasher);
                c.encounter_id.hash(&mut hasher);
                c.encounter_type.hash(&mut hasher);
                c.finished.hash(&mut hasher);
                c.result.hash(&mut hasher);
                c.cards.len().hash(&mut hasher);
                for card in &c.cards {
                    hash_card_stat(&mut hasher, card);
                }
                c.plays.hash(&mut hasher);
                c.turns.hash(&mut hasher);
                c.damage_received.hash(&mut hasher);
                c.block_total.hash(&mut hasher);
                c.potions_used.hash(&mut hasher);
            }
            UiTab::Run => {
                st.player_filter.hash(&mut hasher);
                st.run_turns.hash(&mut hasher);
                st.run_combats.hash(&mut hasher);
                st.run_cards.len().hash(&mut hasher);
                for card in &st.run_cards {
                    hash_card_stat(&mut hasher, card);
                }
            }
        }
    });
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::state::{Combat, PlayerFilter};
    use crate::test_util::test_row;
    use crate::ui::ui_model::Section;

    fn test_layout() -> Layout {
        let rows = [test_row(Section::Damage, 0, 0, "STRIKE", 0, 20, 0, [0; 8])];
        chart_layout::build(chart_layout::BuildInput {
            tab: UiTab::Combat,
            rows: &rows,
            meta: UiMeta::default(),
            footer: "",
            hover_row: None,
            skip_chrome: false,
            avatars: &[],
            flat_chrome: true,
            tab_sprites: false,
            width: chart_layout::PANEL_WIDTH,
            right_gutter: 0.0,
        })
    }

    #[test]
    fn panel_visible_toggles_on_and_off() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.initialized = true;
            st.run_ctx.active = true;
        });
        VISIBLE.with(|v| v.set(false));
        assert!(!visible());
        toggle();
        assert!(visible());
        toggle();
        assert!(!visible());
        toggle();
        assert!(visible());
    }

    #[test]
    fn toggle_is_ignored_outside_a_run() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.initialized = true;
            st.run_ctx.active = false;
        });
        VISIBLE.with(|v| v.set(false));
        toggle();
        assert!(!visible(), "F8 outside a run must not turn the panel on");
        STATE.with(|s| s.borrow_mut().run_ctx.active = true);
        toggle();
        assert!(visible(), "F8 inside a run must turn the panel on");
    }

    #[test]
    fn dismiss_turns_the_panel_off_like_f8() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.initialized = true;
            st.run_ctx.active = true;
        });
        VISIBLE.with(|v| v.set(true));
        dismiss();
        assert!(!visible());
        toggle();
        assert!(visible(), "F8 after a click-away shows the panel again");
    }

    #[test]
    fn press_zone_distinguishes_tab_and_inert() {
        let l = test_layout();
        let tab = chart_layout::tab_at(&l, l.tab_hits[0].x0 + 1.0, l.tab_hits[0].y0 + 1.0)
            .expect("combat tab hit exists");
        assert_eq!(tab, UiTab::Combat);
        assert_eq!(
            press_zone(&l, l.tab_hits[0].x0 + 1.0, l.tab_hits[0].y0 + 1.0),
            PressZone::Tab(UiTab::Combat)
        );
        assert_eq!(
            press_zone(&l, l.tab_hits[1].x0 + 1.0, l.tab_hits[1].y0 + 1.0),
            PressZone::Tab(UiTab::Run)
        );
        assert_eq!(press_zone(&l, 5.0, 100.0), PressZone::Inert);
        assert_eq!(press_zone(&l, 5.0, 5.0), PressZone::Inert);
    }

    #[test]
    fn press_zone_distinguishes_avatar_tab_and_inert() {
        let rows = [test_row(Section::Damage, 0, 0, "STRIKE", 0, 20, 0, [0; 8])];
        let l = chart_layout::build(chart_layout::BuildInput {
            tab: UiTab::Combat,
            rows: &rows,
            meta: UiMeta::default(),
            footer: "",
            hover_row: None,
            skip_chrome: false,
            avatars: &[chart_layout::AvatarFact {
                slot: 1,
                loaded: true,
                path: "res://images/ui/top_panel/character_icon_silent.png".to_owned(),
            }],
            flat_chrome: true,
            tab_sprites: false,
            width: chart_layout::PANEL_WIDTH,
            right_gutter: 0.0,
        });
        let avatar = &l.avatar_hits[0];
        assert_eq!(
            press_zone(&l, avatar.x0 + 1.0, avatar.y0 + 1.0),
            PressZone::Avatar(1)
        );
        assert_eq!(
            press_zone(&l, avatar.x1, avatar.y0 + 1.0),
            PressZone::Inert,
            "outside the avatar box is inert, not the slot"
        );
        let tab = chart_layout::tab_at(&l, l.tab_hits[0].x0 + 1.0, l.tab_hits[0].y0 + 1.0)
            .expect("combat tab hit exists");
        assert_eq!(
            press_zone(&l, l.tab_hits[0].x0 + 1.0, l.tab_hits[0].y0 + 1.0),
            PressZone::Tab(tab)
        );
        assert_eq!(press_zone(&l, 5.0, 100.0), PressZone::Inert);
    }

    #[test]
    fn clicks_outside_the_panel_box_are_inert() {
        let l = test_layout();
        let rect = Rect2::new(Vector2::new(10.0, 180.0), Vector2::new(l.width, 200.0));
        let on_tab = Vector2::new(
            rect.position.x + l.tab_hits[0].x0 + 1.0,
            rect.position.y + l.tab_hits[0].y0 + 1.0,
        );
        assert!(panel_common::over_panel(rect, on_tab));
        let zone = panel_common::guarded_zone(rect, on_tab, |lx, ly| press_zone(&l, lx, ly));
        assert_eq!(zone, PressZone::Tab(UiTab::Combat));
        for y in [rect.position.y - 8.0, rect.position.y + rect.size.y + 8.0] {
            let outside = Vector2::new(on_tab.x, y);
            assert!(!panel_common::over_panel(rect, outside));
            assert_eq!(
                panel_common::guarded_zone(rect, outside, |lx, ly| press_zone(&l, lx, ly)),
                PressZone::Inert
            );
        }
    }

    #[test]
    fn content_signature_is_stable_and_sensitive() {
        let rows = [test_row(Section::Damage, 0, 0, "STRIKE", 0, 20, 0, [0; 8])];
        let meta = UiMeta::default();
        let none = RowDetail::default();
        let base = content_signature(&rows, meta, "footer", &none, UiTab::Combat, None);
        assert_eq!(
            content_signature(&rows, meta, "footer", &none, UiTab::Combat, None),
            base
        );
        let mut changed = rows;
        changed[0].value = 21;
        assert_ne!(
            content_signature(&changed, meta, "footer", &none, UiTab::Combat, None),
            base
        );
        assert_ne!(
            content_signature(&rows, meta, "other", &none, UiTab::Combat, None),
            base
        );
        assert_ne!(
            content_signature(&rows, meta, "footer", &none, UiTab::Run, None),
            base
        );
        assert_ne!(
            content_signature(&rows, meta, "footer", &none, UiTab::Combat, Some(0)),
            base
        );
        let detail = RowDetail {
            title: "detail".to_owned(),
            ..RowDetail::default()
        };
        assert_ne!(
            content_signature(&rows, meta, "footer", &detail, UiTab::Combat, None),
            base
        );
        let mut replayer = rows;
        replayer[0].player = 1;
        assert_ne!(
            content_signature(&replayer, meta, "footer", &none, UiTab::Combat, None),
            base
        );
    }

    #[test]
    fn cheap_state_signature_tracks_snapshot_relevant_changes() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.initialized = true;
            st.current = Some(Combat {
                seq: 1,
                encounter_id: "BYGONE_EFFIGY".to_owned(),
                plays: 2,
                turns: 1,
                cards: vec![CardStat {
                    id: "STRIKE".to_owned(),
                    damage_dealt: 9,
                    ..CardStat::default()
                }],
                ..Combat::default()
            });
        });
        let base = cheap_state_signature(UiTab::Combat);
        assert_eq!(cheap_state_signature(UiTab::Combat), base);
        STATE.with(|s| {
            s.borrow_mut()
                .current
                .as_mut()
                .expect("combat seeded")
                .plays += 1
        });
        assert_ne!(cheap_state_signature(UiTab::Combat), base);
        STATE.with(|s| {
            s.borrow_mut()
                .current
                .as_mut()
                .expect("combat seeded")
                .cards[0]
                .damage_dealt += 1
        });
        assert_ne!(cheap_state_signature(UiTab::Combat), base);
        assert_ne!(cheap_state_signature(UiTab::Run), base);
        // A filter toggle dirties both tabs: the avatar row filters the
        // run accumulator too.
        let run_base = cheap_state_signature(UiTab::Run);
        STATE.with(|s| s.borrow_mut().player_filter = PlayerFilter::Player(1));
        assert_ne!(cheap_state_signature(UiTab::Combat), base);
        assert_ne!(
            cheap_state_signature(UiTab::Run),
            run_base,
            "the run tab re-renders on a filter change"
        );
    }
}
