//! The run-history panel's pure layout engine: the selected run's summary
//! view → draw commands + hit tests. No engine calls happen here.
//!
//! The panel renders a pinned HEADER (title band, the icon row cloning the
//! game's run-history TopSection, the meta line) over a scrolling BODY —
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
#[cfg(test)]
use crate::engine::object::TextAlign;
use crate::ui::chart_layout::{self, Cmd, truncate};
use crate::ui::palette;
#[cfg(test)]
use crate::ui::panel_common;
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
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PortraitFact {
    /// The roster slot the press maps to; parallel to the drawn order.
    pub slot: u8,
    pub path: String,
    pub loaded: bool,
}

/// Resolved against the theme before the build so the layout engine stays
/// engine-free.
#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, PartialEq, Default)]
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
mod tests {
    use super::*;
    use crate::data::run_history::CombatView;
    use crate::data::state::{CardStat, CombatResult, RunOutcome};
    use crate::source_kind::SourceKind;
    use crate::test_util::cmd_texts as texts;
    use crate::ui::ui_model;

    /// IRONCLAD, A7, Standard, defeat — two combats and a three-row
    /// roll-up.
    fn view() -> RunSummaryView {
        let combats = |n: u32| {
            (0..n)
                .map(|i| CombatView {
                    seq: i + 1,
                    encounter: format!("ENC{i}"),
                    result: CombatResult::Completed,
                    damage_dealt: 30,
                    damage_taken: 10,
                    turns: 3,
                })
                .collect()
        };
        let card = |id: &str, kind: SourceKind, plays: u32, dmg: i64, blk: i64| CardStat {
            id: id.to_owned(),
            kind,
            plays,
            damage_dealt: dmg,
            dmg_direct: dmg,
            block_gained: blk,
            block_effective: blk,
            ..CardStat::default()
        };
        RunSummaryView {
            run_id: 2,
            character: "IRONCLAD".to_owned(),
            ascension: 7,
            game_mode: "Standard".to_owned(),
            outcome: Some(RunOutcome::Defeat),
            seed: "BETA".to_owned(),
            combats: combats(2),
            rollup: vec![
                card("STRIKE", SourceKind::Card, 4, 70, 0),
                card("DEMON_FORM", SourceKind::Power, 1, 35, 0),
                card("DEFEND", SourceKind::Card, 2, 0, 15),
            ],
            ..RunSummaryView::default()
        }
    }

    #[test]
    fn identity_line_omits_the_character_and_pins_the_rest() {
        let v = view();
        // The avatar row carries the character; the text line never names it.
        assert_eq!(identity_line(&v), "A7 · Standard · Defeat");
        let mut won = view();
        won.outcome = Some(RunOutcome::Victory);
        assert_eq!(identity_line(&won), "A7 · Standard · Victory");
        // An abandoned run reads "Abandoned", never a false "Defeat".
        let mut abandoned = view();
        abandoned.outcome = Some(RunOutcome::Abandoned);
        assert_eq!(identity_line(&abandoned), "A7 · Standard · Abandoned");
        // An unknown ascension is omitted; the fallback's "Unfinished"
        // still renders.
        let unfinished = RunSummaryView {
            ascension: -1,
            character: "SHROUD".to_owned(),
            ..RunSummaryView::default()
        };
        assert_eq!(identity_line(&unfinished), "Unfinished");
    }

    fn layout_of(view: Option<&RunSummaryView>) -> RunLayout {
        layout_of_header(view, &HeaderFacts::default())
    }

    fn layout_of_header(view: Option<&RunSummaryView>, header: &HeaderFacts) -> RunLayout {
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        build_run_layout(view, header, None, &mut rows, WIDTH, true, 0.0)
    }

    #[test]
    fn layout_renders_the_full_chart_and_the_empty_state_notice() {
        let l = layout_of(Some(&view()));
        let mut found_bar = false;
        for cmd in &l.cmds {
            if let Cmd::Rect(r) = cmd
                && r.w > 0.0
                && r.h == chart_layout::BAR_H
                && r.color == palette::COL_DMG_DIRECT
            {
                found_bar = true;
            }
        }
        assert!(found_bar, "a section-colored chart bar must render");
        // The meta line minus the redundant prefix the gold title already
        // carries.
        let header: Vec<&str> = texts(&l.header_cmds).collect();
        assert!(header.contains(&"Run Summary"));
        assert!(header.contains(&"A7 · Standard · Defeat"));
        assert!(header.contains(&"seed BETA"));
        assert!(header.contains(&"DPS 17.5 · 6 turns · 2 combats"));
        let t: Vec<&str> = texts(&l.cmds).collect();
        assert!(
            !t.iter().any(|s| s.starts_with("DPS ")),
            "the meta line must not scroll"
        );
        for section in ["Damage", "Defense"] {
            assert!(t.contains(&section), "section {section} must render");
        }
        assert!(!t.contains(&"Top sources"));
        // The body ends with the chart (no per-combat list).
        assert!(t.contains(&"STRIKE"));
        assert!(!t.contains(&"Combats"));
        assert!(l.height > 0.0);

        let mut unfinished = view();
        unfinished.character = "SHROUD".to_owned();
        unfinished.outcome = None;
        let l = layout_of(Some(&unfinished));
        let header: Vec<&str> = texts(&l.header_cmds).collect();
        assert!(header.contains(&"A7 · Standard · Unfinished"));
        assert!(header.contains(&"seed BETA"));

        let empty = layout_of(None);
        assert!(texts(&empty.cmds).any(|s| s.contains("no profiling history for this run")));
        assert!(empty.height > 0.0);
    }

    #[test]
    fn layout_renders_full_lists_without_caps() {
        let mut v = view();
        v.combats = (0..30)
            .map(|i| CombatView {
                seq: i + 1,
                encounter: format!("ENC{i}"),
                result: CombatResult::Completed,
                damage_dealt: 10,
                damage_taken: 2,
                turns: 1,
            })
            .collect();
        v.rollup = (0..20)
            .map(|i| CardStat {
                id: format!("CARD{i}"),
                damage_dealt: 100 - i as i64,
                dmg_direct: 100 - i as i64,
                ..CardStat::default()
            })
            .collect();
        let l = layout_of(Some(&v));
        assert!(texts(&l.cmds).any(|s| s == "CARD19"));
        assert!(texts(&l.header_cmds).any(|s| s.contains("30 combats")));
        assert!(!texts(&l.cmds).any(|s| s.contains("more sources")));
        assert!(l.height > 700.0);
    }

    #[test]
    fn layout_height_carries_the_bottom_pad_exactly_once() {
        let v = view();
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let run = build_run_layout(
            Some(&v),
            &HeaderFacts::default(),
            None,
            &mut rows,
            WIDTH,
            false,
            0.0,
        );
        let cards = crate::data::run_history::filtered_rollup(&v);
        let turns: u32 = v.combats.iter().map(|combat| combat.turns).sum();
        let taken: i64 = v.combats.iter().map(|combat| combat.damage_taken).sum();
        let meta = crate::ui::snapshot::ui_snapshot_meta_from_run(
            cards,
            turns,
            v.combats.len() as u32,
            taken,
        );
        let chart = chart_layout::build(chart_layout::BuildInput {
            tab: UiTab::Run,
            rows: &rows[..run.chart_rows],
            meta,
            footer: "",
            hover_row: None,
            skip_chrome: true,
            avatars: &[],
            flat_chrome: false,
            tab_sprites: false,
            width: WIDTH,
            right_gutter: 0.0,
        });
        assert_eq!(run.height, run.header_bottom + chart.height);
    }

    #[test]
    fn spliced_row_hits_resolve_hover_and_detail() {
        let v = view();
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let l = build_run_layout(
            Some(&v),
            &HeaderFacts::default(),
            None,
            &mut rows,
            WIDTH,
            true,
            0.0,
        );
        assert!(l.chart_rows > 0, "the fixture's roll-up builds chart rows");
        assert_eq!(l.row_hits.len(), l.chart_rows);
        // The splice pushes hits below the header lines, so none start at y 0.
        let first = l.row_hits[0];
        assert!(first.y0 > 0.0, "the splice translated the hits down");
        for hit in &l.row_hits {
            let mid = f32::midpoint(hit.y0, hit.y1);
            assert_eq!(chart_layout::row_at(&l.row_hits, mid), Some(hit.flat_index));
        }
        let detail = crate::ui::snapshot::ui_row_detail_from_cards(
            &rows[..l.chart_rows],
            first.flat_index,
            &v.rollup,
        );
        assert!(!detail.is_empty(), "a chart row resolves detail text");
        let empty = layout_of(None);
        assert!(empty.row_hits.is_empty());
        assert_eq!(empty.chart_rows, 0);
        let mut rows2 = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let hovered = build_run_layout(
            Some(&v),
            &HeaderFacts::default(),
            Some(first.flat_index),
            &mut rows2,
            WIDTH,
            true,
            0.0,
        );
        assert!(
            hovered
                .cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Rect(r) if r.color == palette::COL_HOVER)),
            "the hovered row bakes its highlight rect"
        );
    }

    #[test]
    fn every_command_lies_inside_the_content_box() {
        for flat_chrome in [true, false] {
            for header in [HeaderFacts::default(), loaded_header()] {
                let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
                let l = build_run_layout(
                    Some(&view()),
                    &header,
                    None,
                    &mut rows,
                    WIDTH,
                    flat_chrome,
                    0.0,
                );
                let chrome: Vec<Cmd> = if flat_chrome {
                    panel_common::border_rects(l.width, l.height).to_vec()
                } else {
                    Vec::new()
                };
                crate::test_util::assert_layout_bounds(
                    &l.header_cmds,
                    &l.cmds,
                    l.content,
                    l.header_bottom,
                    l.height,
                    0.0,
                    &chrome,
                );
            }
        }
    }

    #[test]
    fn header_carries_the_title_icon_row_and_meta_body_starts_at_the_chart() {
        let l = layout_of_header(Some(&view()), &loaded_header());
        let header: Vec<&str> = texts(&l.header_cmds).collect();
        assert!(header.contains(&"Run Summary"));
        assert!(header.contains(&"A7 · Standard · Defeat"));
        assert!(header.contains(&"seed BETA"));
        assert!(
            header.contains(&"DPS 17.5 · 6 turns · 2 combats"),
            "the meta line pins under the identity/seed block: {header:?}"
        );
        // The pinned meta line is hard-clipped at the content's right
        // edge, like the chart's own.
        let meta_cmd = l
            .header_cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == "DPS 17.5 · 6 turns · 2 combats" => Some(t),
                _ => None,
            })
            .expect("the pinned meta line renders");
        assert_eq!(meta_cmd.x, l.content.x);
        assert_eq!(meta_cmd.align, TextAlign::LeftClipped(l.content.w));
        assert!(
            l.header_cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Texture(_))),
            "the header's icons are pinned"
        );
        let body: Vec<&str> = texts(&l.cmds).collect();
        assert!(
            !body.iter().any(|s| s.starts_with("DPS ")),
            "the meta line must not scroll: {body:?}"
        );
        assert!(
            !body.contains(&"seed BETA"),
            "the seed line must not scroll"
        );
        let first = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) => Some(t),
                _ => None,
            })
            .expect("a section title renders");
        assert_eq!(first.text, "Damage", "the body opens with the chart");
        assert_eq!(first.y, l.header_bottom + 26.0, "at the band edge");

        let l = layout_of(Some(&view()));
        let header: Vec<&str> = texts(&l.header_cmds).collect();
        assert!(header.contains(&"seed BETA"));
        assert!(header.contains(&"DPS 17.5 · 6 turns · 2 combats"));
        assert!(
            l.header_bottom < layout_of_header(Some(&view()), &loaded_header()).header_bottom,
            "the collapsed header is shorter than the icon row"
        );

        let empty = layout_of(None);
        let empty_header: Vec<&str> = texts(&empty.header_cmds).collect();
        assert!(empty_header.contains(&"Run Summary"));
        assert!(
            !empty_header.iter().any(|s| s.starts_with("DPS ")),
            "no meta line without a chart"
        );
        assert!(
            texts(&empty.cmds).any(|t| t.contains("no profiling history")),
            "the notice scrolls with the body"
        );
    }

    fn loaded_header() -> HeaderFacts {
        HeaderFacts {
            portraits: vec![PortraitFact {
                slot: 0,
                path: character_icon_path("IRONCLAD").expect("a slug-shaped id maps"),
                loaded: true,
            }],
        }
    }

    #[test]
    fn roster_entries_carry_slots_and_character_icon_paths_mirror_the_game() {
        let v = view();
        assert_eq!(roster_entries(&v), vec![(0, "IRONCLAD")]);
        assert_eq!(
            character_icon_path("IRONCLAD").as_deref(),
            Some("res://images/ui/top_panel/character_icon_ironclad.png")
        );
        assert_eq!(
            character_icon_path("NECROBINDER").as_deref(),
            Some("res://images/ui/top_panel/character_icon_necrobinder.png")
        );
        // A well-formed unknown id still maps: the load fails and skips.
        assert_eq!(character_icon_path("?"), None);
        assert_eq!(character_icon_path(""), None);
        assert_eq!(character_icon_path("ironclad"), None);

        let mut mp = view();
        mp.character = "IRONCLAD, SILENT, DEFECT, REGENT, NECROBINDER".to_owned();
        assert_eq!(
            roster_entries(&mp),
            vec![(0, "IRONCLAD"), (1, "SILENT"), (2, "DEFECT"), (3, "REGENT")]
        );
        mp.players = vec![crate::data::records::PlayerRec {
            slot: 0,
            character: "SILENT".to_owned(),
        }];
        assert_eq!(roster_entries(&mp), vec![(0, "SILENT")]);
        mp.players = vec![
            crate::data::records::PlayerRec {
                slot: 2,
                character: "SHROUD".to_owned(),
            },
            crate::data::records::PlayerRec {
                slot: 0,
                character: "".to_owned(),
            },
        ];
        assert_eq!(
            roster_entries(&mp),
            vec![(2, "SHROUD")],
            "record slots are kept verbatim; empty characters are skipped"
        );
    }

    #[test]
    fn header_row_emits_loaded_icons_and_the_right_aligned_block() {
        let v = view();
        let l = layout_of_header(Some(&v), &loaded_header());
        let icons: Vec<&chart_layout::TextureCmd> = l
            .header_cmds
            .iter()
            .filter_map(|cmd| match cmd {
                Cmd::Texture(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(icons.len(), 1, "portrait only");
        assert_eq!(icons[0].icon, theme::IconId::Character(0));
        assert_eq!(icons[0].w, icons[0].h, "the portrait art is square");
        assert_eq!(
            l.portrait_paths,
            vec!["res://images/ui/top_panel/character_icon_ironclad.png".to_owned()]
        );
        // The loaded portrait is pressable: one hit, keyed to its slot,
        // in the icon row below the title band.
        assert_eq!(l.avatar_hits.len(), 1);
        assert_eq!(
            l.avatar_hits[0],
            chart_layout::AvatarHit {
                x0: l.content.x,
                y0: l.content.top + TITLE_H,
                x1: l.content.x + ICON_ROW_H,
                y1: l.content.top + TITLE_H + ICON_ROW_H,
                slot: 0,
            }
        );
        assert_eq!(
            chart_layout::avatar_at(
                &l.avatar_hits,
                l.content.x + 1.0,
                l.content.top + TITLE_H + 1.0
            ),
            Some(0)
        );
        let identity = l
            .header_cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == "A7 · Standard · Defeat" => Some(t),
                _ => None,
            })
            .expect("the identity line renders");
        let TextAlign::Right(w) = identity.align else {
            panic!("the identity right-aligns")
        };
        assert_eq!(identity.x + w, l.content.right());
        assert!(identity.x > icons[0].x, "the block starts past the icons");
    }

    #[test]
    fn header_row_falls_back_without_loaded_icons() {
        let v = view();
        let partial = HeaderFacts {
            portraits: vec![PortraitFact {
                slot: 0,
                path: character_icon_path("IRONCLAD").expect("maps"),
                loaded: false,
            }],
        };
        let l = layout_of_header(Some(&v), &partial);
        assert_eq!(l.portrait_paths.len(), 1, "the wanted path is recorded");
        assert!(
            !l.header_cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Texture(t) if t.icon == theme::IconId::Character(0))),
            "an unloaded portrait leaves no placeholder"
        );

        let l = layout_of(Some(&v));
        assert!(
            !l.header_cmds
                .iter()
                .chain(l.cmds.iter())
                .any(|cmd| matches!(cmd, Cmd::Texture(_)))
        );
        assert!(l.portrait_paths.is_empty());
        let identity = l
            .header_cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == "A7 · Standard · Defeat" => Some(t),
                _ => None,
            })
            .expect("the identity line renders");
        // A multiplayer roster identity can exceed the width; the clip
        // keeps it from bleeding.
        assert_eq!(
            identity.align,
            TextAlign::LeftClipped(l.content.right() - identity.x),
            "the collapsed line left-aligns with the width clip"
        );
        assert_eq!(identity.x, l.content.x + 8.0);
    }

    /// The dump splits header from body, so a wrong-zone command reviews.
    #[test]
    fn golden_run_panel_commands_flat_chrome() {
        let l = layout_of(Some(&view()));
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }

    #[test]
    fn golden_run_panel_commands_plate_chrome() {
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let l = build_run_layout(
            Some(&view()),
            &HeaderFacts::default(),
            None,
            &mut rows,
            WIDTH,
            false,
            0.0,
        );
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }

    #[test]
    fn golden_run_panel_commands_flat_chrome_icons() {
        let l = layout_of_header(Some(&view()), &loaded_header());
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }

    #[test]
    fn golden_run_panel_commands_plate_chrome_icons() {
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let l = build_run_layout(
            Some(&view()),
            &loaded_header(),
            None,
            &mut rows,
            WIDTH,
            false,
            0.0,
        );
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }
}
