//! The run-history panel's pure layout engine: the selected run's summary
//! view → draw commands + hit tests. No engine calls happen here.
//!
//! The panel renders a pinned HEADER (title band, the icon row cloning the
//! game's run-history TopSection, the meta line) over a scrolling BODY:
//! the same two-section chart the Run Summary tab renders, built
//! chrome-less and spliced in translated by y. The body ends with the
//! chart: the game's screen already renders per-room details, so there is
//! deliberately no per-combat list.
//!
//! Portraits are per-run DATA: a missing asset removes the icon (never a
//! placeholder); with no icon the header collapses to the plain
//! identity/seed lines.

use crate::data::run_history::RunSummaryView;
use crate::data::state::{CardStat, RunOutcome};
use crate::ui::chart_layout::{self, Cmd, truncate};
use crate::ui::palette;
use crate::ui::theme::{self, TextRole};
use crate::ui::ui_model::{UiMeta, UiRow, UiTab};

/// An unknown ascension (-1) is omitted rather than rendered as "A-1". The
/// character is deliberately absent: the avatar row carries the identity,
/// never a text name.
pub(crate) fn identity_line(view: &RunSummaryView) -> String {
    let mut line = String::new();
    if view.ascension >= 0 {
        line.push_str(&format!("A{}", view.ascension));
    }
    if !view.game_mode.is_empty() {
        if !line.is_empty() {
            line.push_str(" · ");
        }
        line.push_str(&view.game_mode);
    }
    let result = match view.outcome {
        Some(RunOutcome::Victory) => "Victory",
        Some(RunOutcome::Defeat) => "Defeat",
        Some(RunOutcome::Abandoned) => "Abandoned",
        None => "Unfinished",
    };
    if !line.is_empty() {
        line.push_str(" · ");
    }
    line.push_str(result);
    line
}

/// Aliased so the two panels cannot drift.
pub const WIDTH: f32 = chart_layout::PANEL_WIDTH;
const TITLE_H: f32 = 40.0;
const LINE_H: f32 = 28.0;
const SIZE_BODY: i32 = theme::SIZE_BODY;

/// The icon row's pitch aliases the chart's avatar row: one square art
/// size across both panels.
const ICON_ROW_H: f32 = chart_layout::AVATAR_H;
const ICON_LABEL_GAP: f32 = chart_layout::AVATAR_GAP;

const IDENTITY_BASELINE: f32 = 26.0;
const SEED_BASELINE: f32 = 54.0;

/// Unloaded portraits are skipped, never placeheld.
pub(crate) struct PortraitFact {
    /// The roster slot the press maps to; parallel to the drawn order.
    pub slot: u8,
    pub path: String,
    pub loaded: bool,
}

/// Resolved against the theme before the build so the layout engine stays
/// engine-free.
#[derive(Default)]
pub(crate) struct HeaderFacts {
    /// One entry per roster character, capped at
    /// [`crate::data::state::caps::MAX_PLAYERS`].
    pub portraits: Vec<PortraitFact>,
}

/// (slot, character) pairs in roster order, capped at the lobby max; the
/// comma-joined field on pre-roster records falls back with implicit
/// slot order.
pub(crate) fn roster_entries(view: &RunSummaryView) -> Vec<(u8, &str)> {
    const CAP: usize = crate::data::state::caps::MAX_PLAYERS;
    if !view.players.is_empty() {
        return view
            .players
            .iter()
            .take(CAP)
            .filter(|p| !p.character.is_empty())
            .map(|p| (p.slot, p.character.as_str()))
            .collect();
    }
    view.character
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .take(CAP)
        .enumerate()
        .map(|(slot, id)| (slot as u8, id))
        .collect()
}

/// Mirrored defensively: non-slug ids return None so a bogus path never
/// reaches ResourceLoader.
pub(crate) fn character_icon_path(id: &str) -> Option<String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    Some(format!(
        "res://images/ui/top_panel/character_icon_{}.png",
        id.to_lowercase()
    ))
}

#[derive(Default)]
pub struct RunLayout {
    pub cmds: Vec<Cmd>,
    pub header_cmds: Vec<Cmd>,
    pub row_hits: Vec<chart_layout::RowHit>,
    pub chart_rows: usize,
    pub has_chart: bool,
    pub header_bottom: f32,
    /// The icon row's press boxes; only loaded portraits get one.
    pub(crate) avatar_hits: Vec<chart_layout::AvatarHit>,
    /// All wanted portraits, loaded or not, so draw can re-resolve them.
    pub portrait_paths: Vec<String>,
    pub content: theme::ContentBox,
    pub width: f32,
    pub height: f32,
}

impl RunLayout {
    fn sink(&mut self) -> chart_layout::CmdSink<'_> {
        chart_layout::CmdSink::new(&mut self.cmds, "run panel")
    }

    fn splice_chart(&mut self, chart: &chart_layout::Layout, y_offset: f32) {
        for cmd in &chart.cmds {
            let cmd = match cmd {
                Cmd::Rect(r) => Cmd::Rect(chart_layout::RectCmd {
                    y: r.y + y_offset,
                    ..*r
                }),
                Cmd::Text(t) => Cmd::Text(chart_layout::TextCmd {
                    y: t.y + y_offset,
                    ..t.clone()
                }),
                Cmd::Texture(t) => Cmd::Texture(chart_layout::TextureCmd {
                    y: t.y + y_offset,
                    ..*t
                }),
            };
            chart_layout::push_cmd(&mut self.cmds, cmd, "run panel");
        }
        self.row_hits
            .extend(chart.row_hits.iter().map(|hit| chart_layout::RowHit {
                y0: hit.y0 + y_offset,
                y1: hit.y1 + y_offset,
                flat_index: hit.flat_index,
            }));
    }
}

/// Builds one frame; load state enters as data via `header`.
#[allow(clippy::too_many_arguments)] // one frame's full build context; bundling it further is artificial
pub(crate) fn build_run_layout(
    view: Option<&RunSummaryView>,
    header: &HeaderFacts,
    hover_row: Option<usize>,
    chart_rows: &mut [UiRow],
    width: f32,
    flat_chrome: bool,
    right_gutter: f32,
) -> RunLayout {
    let content = theme::content_box(width, !flat_chrome, right_gutter);
    let mut l = RunLayout {
        width,
        content,
        ..RunLayout::default()
    };
    let Some(view) = view else {
        return build_empty_state(l, flat_chrome);
    };
    l.has_chart = true;
    // The filter selects which roll-up the chart and detail render.
    let cards = crate::data::run_history::filtered_rollup(view);
    // The meta line and the chart share one computed meta.
    let turns: u32 = view.combats.iter().map(|c| c.turns).sum();
    let taken: i64 = view.combats.iter().map(|c| c.damage_taken).sum();
    let meta = crate::ui::snapshot::ui_snapshot_meta_from_run(
        cards,
        turns,
        view.combats.len() as u32,
        taken,
    );
    let mut y = content.top;
    y = emit_title(&mut l, &content, y);
    y = emit_header(&mut l, view, &content, header, y);
    // The meta line pins with the header; no redundant prefix.
    y = emit_meta(&mut l, &content, &meta, y);
    y += 6.0;
    // The header/body split point: everything emitted so far is the pinned
    // header. One Vec during emission keeps the emitters target-agnostic;
    // the drain at the end splits it into the two lists the replay draws.
    let header_len = l.cmds.len();
    l.header_bottom = y;
    // The body is the spliced chart, which ends the content: the game's
    // screen already renders per-room details.
    y = emit_chart(
        &mut l,
        chart_rows,
        cards,
        &meta,
        hover_row,
        width,
        right_gutter,
        flat_chrome,
        y,
    );
    // The chart's own height already ends with the bottom pad.
    l.height = y;
    l.header_cmds = l.cmds.drain(..header_len).collect();
    if flat_chrome {
        chart_layout::insert_borders(&mut l.header_cmds, "run panel", l.width, l.height);
    }
    l
}

fn build_empty_state(mut l: RunLayout, flat_chrome: bool) -> RunLayout {
    let content = l.content;
    let mut y = content.top;
    y = emit_title(&mut l, &content, y);
    y += 6.0;
    let header_len = l.cmds.len();
    l.header_bottom = y;
    l.sink().text(
        content.x + 8.0,
        y + 25.0,
        SIZE_BODY,
        palette::COL_DIM,
        "no profiling history for this run",
    );
    y += 30.0;
    l.sink().text(
        content.x + 8.0,
        y + 25.0,
        SIZE_BODY,
        palette::COL_DIM,
        "runs recorded by Spire Profiler appear here",
    );
    // The final line's baseline clears the band by its ~7px descent.
    y += 32.0;
    l.height = y + l.content.outer_bottom_pad;
    l.header_cmds = l.cmds.drain(..header_len).collect();
    if flat_chrome {
        chart_layout::insert_borders(&mut l.header_cmds, "run panel", l.width, l.height);
    }
    l
}

fn emit_title(l: &mut RunLayout, content: &theme::ContentBox, y_in: f32) -> f32 {
    l.sink().title_text(content.x, y_in + 30.0, "Run Summary");
    y_in + TITLE_H
}

/// Icons render only when loaded; with none the row collapses to the plain
/// identity/seed lines.
fn emit_header(
    l: &mut RunLayout,
    view: &RunSummaryView,
    content: &theme::ContentBox,
    header: &HeaderFacts,
    y_in: f32,
) -> f32 {
    l.portrait_paths = header.portraits.iter().map(|p| p.path.clone()).collect();
    let mut x = content.x;
    let mut drew = false;
    for (i, portrait) in header.portraits.iter().enumerate() {
        if !portrait.loaded {
            continue;
        }
        // The portrait art is square, so the destination rect is too.
        l.sink().texture(
            x,
            y_in,
            ICON_ROW_H,
            ICON_ROW_H,
            theme::IconId::Character(i as u8),
        );
        l.avatar_hits.push(chart_layout::AvatarHit {
            x0: x,
            y0: y_in,
            x1: x + ICON_ROW_H,
            y1: y_in + ICON_ROW_H,
            slot: portrait.slot,
        });
        x += ICON_ROW_H + ICON_LABEL_GAP;
        drew = true;
    }
    if !drew {
        let left = content.x + 8.0;
        let width = (l.content.right() - left).max(0.0);
        l.sink().text_left_clipped(
            left,
            width,
            y_in + 25.0,
            SIZE_BODY,
            palette::COL_CREAM,
            TextRole::Body,
            identity_line(view),
        );
        let y = y_in + LINE_H;
        l.sink().text_left_clipped(
            left,
            width,
            y + 25.0,
            SIZE_BODY,
            palette::COL_DIM,
            TextRole::Body,
            format!("seed {}", truncate(&view.seed, 72)),
        );
        return y + LINE_H;
    }
    // The alignment box starts past the icon groups, so an overlong line
    // clips there instead of drawing over the icons.
    let block_x = x + 8.0;
    let block_w = (content.right() - block_x).max(0.0);
    l.sink().text_right(
        block_x,
        block_w,
        y_in + IDENTITY_BASELINE,
        SIZE_BODY,
        palette::COL_CREAM,
        identity_line(view),
    );
    l.sink().text_right(
        block_x,
        block_w,
        y_in + SEED_BASELINE,
        SIZE_BODY,
        palette::COL_DIM,
        format!("seed {}", truncate(&view.seed, 72)),
    );
    y_in + ICON_ROW_H
}

fn emit_meta(l: &mut RunLayout, content: &theme::ContentBox, meta: &UiMeta, y_in: f32) -> f32 {
    l.sink().text_left_clipped(
        content.x,
        content.w,
        y_in + chart_layout::META_Y,
        SIZE_BODY,
        palette::COL_CREAM,
        TextRole::Title,
        chart_layout::meta_line(UiTab::Run, meta),
    );
    y_in + chart_layout::META_H
}

/// Chrome-less, so the splice translation is exactly this panel's offset.
#[allow(clippy::too_many_arguments)] // one section's full build context; bundling it further is artificial
fn emit_chart(
    l: &mut RunLayout,
    rows: &mut [UiRow],
    cards: &[CardStat],
    meta: &UiMeta,
    hover_row: Option<usize>,
    width: f32,
    right_gutter: f32,
    flat_chrome: bool,
    y_in: f32,
) -> f32 {
    let n = crate::ui::snapshot::ui_snapshot_rows_from(cards, rows);
    l.chart_rows = n;
    let chart = chart_layout::build(chart_layout::BuildInput {
        tab: UiTab::Run,
        rows: &rows[..n],
        meta: *meta,
        footer: "",
        hover_row,
        skip_chrome: true,
        avatars: &[],
        flat_chrome,
        // The chrome-less chart emits no tab strip; the flag is inert.
        tab_sprites: false,
        width,
        right_gutter,
    });
    l.splice_chart(&chart, y_in);
    y_in + chart.height
}

#[cfg(test)]
mod tests;
