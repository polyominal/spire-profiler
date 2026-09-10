//! The segmented contribution chart — a pure layout engine: turns the
//! chart payload into draw commands (rects + texts) plus hit-test tables,
//! split into a pinned HEADER (the floating tab strip above the plate,
//! then the title band, avatar row, and meta line) and a scrolling BODY
//! (sections, footer) — the game's top-bar idiom, where the header never
//! scrolls. The strip's band [`Layout::strip_h`] extends the Control
//! upward; the plate starts below it. No engine calls happen here.
//!
//! # Geometry
//!
//! * [`PANEL_WIDTH`] = 780px: fixed name/plays/gap/value columns plus a 197px bar track; builds
//!   reflow to any width, the bar absorbing the difference.
//! * 20px bars on 32px rows; per-section headers with a gold underline a few px below the title's
//!   descent (a baseline-level underline crosses the glyphs and reads as a strikethrough);
//!   dedicated red self-damage rows. Segments are NOT clamped when the per-mille sum exceeds 1000 —
//!   trailing segments overflow the track.
//! * Every element lays out against [`theme::content_box`], so bands span exactly the content width
//!   in both chrome modes.
//! * Right-side columns right-align to the content edge, so a longer string clips left, never past
//!   the edge under the scrollbar.
//!
//! Colors resolve from [`crate::ui::palette`], never defined here; draw
//! commands own their text and the lists respect the command cap.

use crate::engine::math::{Rect2, Vector2};
use crate::engine::object::TextAlign;
#[cfg(test)]
use crate::source_kind::SourceKind;
use crate::ui::palette::{
    COL_CREAM, COL_DIM, COL_GOLD, COL_HEADER_BG, COL_HOVER, COL_ROW_ALT, COL_SELF, COL_TRACK,
    Color, PREFIX_ADVANCE, kind_prefix, slot_color,
};
use crate::ui::theme::{self, TextRole};
use crate::ui::ui_model::{self, Section, Segment, UiMeta, UiRow, UiTab};

// Vertical constants derive from the shipped Kreon faces' metrics at
// 24/32px (ascent 997, descent −293, cap 708 per 1024 upm).

pub const PANEL_WIDTH: f32 = 780.0;
pub const HEADER_H: f32 = 40.0;
pub(crate) const TITLE_Y: f32 = 30.0;

/// The avatar row's square art and pitch, the game's 64px portrait idiom
/// (run_history_player_icon.tscn); the run panel's icon row aliases these.
pub(crate) const AVATAR_H: f32 = 64.0;
pub(crate) const AVATAR_GAP: f32 = 6.0;

/// The air between the floating tab strip and the plate's top edge (the
/// game's settings strip hangs 25px above its content; a tighter gap
/// reads as a panel handle).
pub(crate) const STRIP_GAP: f32 = 12.0;

pub(crate) const TAB_W: f32 = 256.0;
pub(crate) const TABS_H: f32 = 90.0;
const TAB_GAP: f32 = 8.0;
/// Optically centered 32px glyphs.
const TAB_LABEL_Y: f32 = 56.0;
// The tab box keeps the art frame's aspect within stretch tolerance.
const _: () =
    assert!((TAB_W / TABS_H - theme::TAB_ART_SIZE[0] / theme::TAB_ART_SIZE[1]).abs() < 0.01);

pub(crate) const META_H: f32 = 34.0;
pub(crate) const META_Y: f32 = 26.0;

// The slug is untruncated: the longest real id measures 572px against
// the 713px content width (a budget test pins the derivation). The width
// clip stays as insurance.

const SECTION_HEADER_H: f32 = 38.0;
/// 24px caps clear the band top by ~3px.
const SECTION_TITLE_Y: f32 = 26.0;
/// Below the 24px title's descent (~7px): the rule must never cross the
/// glyphs (a baseline-level underline reads as a strikethrough).
const SECTION_UNDERLINE_Y: f32 = 34.0;
const SECTION_UNDERLINE_H: f32 = 2.0;
const _: () = assert!(SECTION_UNDERLINE_Y >= SECTION_TITLE_Y + 7.0);
const _: () = assert!(SECTION_UNDERLINE_Y + SECTION_UNDERLINE_H <= SECTION_HEADER_H);
const ROW_H: f32 = 32.0;
/// Ascent ~23.4, descent ~6.9: the descent ends a hair under the bottom edge.
const ROW_TEXT_Y: f32 = 25.0;
pub(crate) const BAR_H: f32 = 20.0;
const NONE_H: f32 = 30.0;
const SECTION_GAP: f32 = 12.0;
/// 24px lines; the baseline slack matches `ROW_H - ROW_TEXT_Y` so the last
/// footer's descenders clear the band's bottom edge.
const FOOTER_LINE_H: f32 = 32.0;
/// The "+ " marker is ASCII on purpose: Kreon ships no box-drawing glyphs,
/// and an uncovered glyph falls the whole panel back to the default font.
const SELF_INDENT: f32 = 18.0;

/// The common id + `"[R] "` prefix at the measured 24px advances (259px of
/// 264); longer ids truncate.
const NAME_W: f32 = 264.0;
/// Worst case overhangs ~15px into the plays column — accepted; a cut
/// name shows 15 chars + the marker, narrower than the replaced char.
const NAME_MAX_CHARS: usize = 16;
/// "x99" = 40px at 24px.
const PLAYS_W: f32 = 44.0;
/// "99999  (100.0%)" = 177px + air; a longer total clips left.
const VALUE_W: f32 = 200.0;
/// A tripped floor means a caller bug — a zero bar would invert the rect.
const MIN_BAR_W: f32 = 40.0;

/// Fixed name/plays columns and the value reserve at the right edge; the
/// bar stretches between them. The content box already carries the gutter.
struct Geom {
    content: theme::ContentBox,
    bar_x: f32,
    bar_w: f32,
    value_x: f32,
}

impl Geom {
    fn new(content: theme::ContentBox) -> Self {
        let bar_x = content.x + NAME_W + PLAYS_W;
        let bar_w = (content.w - NAME_W - PLAYS_W - 8.0 - VALUE_W).max(MIN_BAR_W);
        Geom {
            content,
            bar_x,
            bar_w,
            value_x: bar_x + bar_w + 8.0,
        }
    }
}

const SIZE_BODY: i32 = theme::SIZE_BODY;

/// One avatar in the combat header's roster row: the slot the press maps
/// to, and the load state resolved against the theme before the build.
/// Unloaded avatars are skipped, never placeheld.
pub(crate) struct AvatarFact {
    pub slot: u8,
    pub loaded: bool,
    pub path: String,
}

/// The avatar row's press box, in box-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AvatarHit {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub slot: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectCmd {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextCmd {
    pub x: f32,
    pub y: f32,
    pub size: i32,
    pub color: Color,
    /// The theme's title/body faces, falling back to the default font.
    pub role: TextRole,
    /// The game's body-text shadow; per-row shadows would double the dense
    /// rows' draw calls.
    pub shadow: bool,
    /// The 32px gold-header treatment: a #543F00 rim plus the (5,4) 12.5%
    /// header shadow. The two panels' titles only.
    pub outline: bool,
    /// Right/Center/LeftClipped replay over the box `[x, x + w]`; Left
    /// draws unconstrained.
    pub align: TextAlign,
    pub text: String,
}

/// The destination rect's aspect must match the icon's source region.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureCmd {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub icon: theme::IconId,
}

impl TextureCmd {
    pub(crate) fn scaled_rect(&self, scale: f32) -> Rect2 {
        let size = Vector2::new(self.w * scale, self.h * scale);
        let position = Vector2::new(
            self.x - (size.x - self.w) / 2.0,
            self.y - (size.y - self.h) / 2.0,
        );
        Rect2::new(position, size)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Rect(RectCmd),
    Text(TextCmd),
    Texture(TextureCmd),
}

#[derive(Clone, Copy)]
pub struct RowHit {
    pub y0: f32,
    pub y1: f32,
    pub flat_index: usize,
}

#[derive(Debug, PartialEq)]
pub struct TabHit {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub tab: UiTab,
}

/// Capped at the true worst case: 256 rows × 15 commands plus chrome.
pub(crate) const MAX_CMDS: usize = ui_model::MAX_UI_ROWS * 15 + 256;
const MAX_LINES: usize = 64;

pub struct Layout {
    /// The scrolling body: the clipped body child replays it, translated
    /// by the scroll offset.
    pub cmds: Vec<Cmd>,
    /// The pinned header, drawn untranslated on the panel itself. Empty
    /// when chrome is skipped.
    pub header_cmds: Vec<Cmd>,
    pub row_hits: Vec<RowHit>,
    pub tab_hits: Vec<TabHit>,
    /// The avatar row's press boxes; empty without a roster.
    pub(crate) avatar_hits: Vec<AvatarHit>,
    /// All wanted avatar paths (loaded or not), so draw can re-resolve
    /// them.
    pub(crate) portrait_paths: Vec<String>,
    /// Body start in box-local y. Overflow stays `height - box_height`:
    /// header and box both include the header.
    pub header_bottom: f32,
    /// The tab strip's band above the plate: the Control extends upward
    /// by this much and the plate starts below it. Zero when the chrome
    /// is skipped.
    pub(crate) strip_h: f32,
    pub content: theme::ContentBox,
    pub width: f32,
    pub height: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            cmds: Vec::new(),
            header_cmds: Vec::new(),
            row_hits: Vec::new(),
            tab_hits: Vec::new(),
            avatar_hits: Vec::new(),
            portrait_paths: Vec::new(),
            header_bottom: 0.0,
            strip_h: 0.0,
            content: theme::ContentBox::default(),
            width: PANEL_WIDTH,
            height: 0.0,
        }
    }
}

/// Refuses past the cap; the overflow report is mandatory because a silent
/// drop leaves row hits pointing at rows that never drew.
pub(crate) fn push_cmd(cmds: &mut Vec<Cmd>, cmd: Cmd, owner: &str) {
    if cmds.len() >= MAX_CMDS {
        crate::ui::panel_common::log_cmd_overflow_once(owner);
        return;
    }
    cmds.push(cmd);
}

/// The emitters' shared target: the command list under construction plus
/// the owner label the cap overflow reports under. Both layouts and the
/// legend emit through one, so the command shapes cannot drift.
pub(crate) struct CmdSink<'a> {
    cmds: &'a mut Vec<Cmd>,
    owner: &'static str,
}

impl<'a> CmdSink<'a> {
    pub(crate) fn new(cmds: &'a mut Vec<Cmd>, owner: &'static str) -> Self {
        CmdSink { cmds, owner }
    }

    pub(crate) fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        push_cmd(
            self.cmds,
            Cmd::Rect(RectCmd { x, y, w, h, color }),
            self.owner,
        );
    }

    pub(crate) fn texture(&mut self, x: f32, y: f32, w: f32, h: f32, icon: theme::IconId) {
        push_cmd(
            self.cmds,
            Cmd::Texture(TextureCmd { x, y, w, h, icon }),
            self.owner,
        );
    }

    pub(crate) fn text(&mut self, x: f32, y: f32, size: i32, color: Color, s: impl Into<String>) {
        self.text_ex(x, y, size, color, TextRole::Body, false, s);
    }

    pub(crate) fn text_right(
        &mut self,
        x: f32,
        width: f32,
        y: f32,
        size: i32,
        color: Color,
        s: String,
    ) {
        if s.is_empty() {
            return;
        }
        push_cmd(
            self.cmds,
            Cmd::Text(TextCmd {
                x,
                y,
                size,
                color,
                role: TextRole::Body,
                shadow: false,
                outline: false,
                align: TextAlign::Right(width),
                text: s,
            }),
            self.owner,
        );
    }

    /// The meta block: the engine's width clip keeps content from bleeding
    /// past the right edge.
    #[allow(clippy::too_many_arguments)] // a draw command's full parameter list
    pub(crate) fn text_left_clipped(
        &mut self,
        x: f32,
        width: f32,
        y: f32,
        size: i32,
        color: Color,
        role: TextRole,
        s: String,
    ) {
        if s.is_empty() {
            return;
        }
        push_cmd(
            self.cmds,
            Cmd::Text(TextCmd {
                x,
                y,
                size,
                color,
                role,
                shadow: false,
                outline: false,
                align: TextAlign::LeftClipped(width),
                text: s,
            }),
            self.owner,
        );
    }

    #[allow(clippy::too_many_arguments)] // a draw command's full parameter list
    pub(crate) fn text_ex(
        &mut self,
        x: f32,
        y: f32,
        size: i32,
        color: Color,
        role: TextRole,
        shadow: bool,
        s: impl Into<String>,
    ) {
        let s = s.into();
        if s.is_empty() {
            return;
        }
        push_cmd(
            self.cmds,
            Cmd::Text(TextCmd {
                x,
                y,
                size,
                color,
                role,
                shadow,
                outline: false,
                align: TextAlign::Left,
                text: s,
            }),
            self.owner,
        );
    }

    pub(crate) fn title_text(&mut self, x: f32, y: f32, s: impl Into<String>) {
        let s = s.into();
        if s.is_empty() {
            return;
        }
        push_cmd(
            self.cmds,
            Cmd::Text(TextCmd {
                x,
                y,
                size: theme::SIZE_HEADER,
                color: COL_GOLD,
                role: TextRole::Title,
                shadow: false,
                outline: true,
                align: TextAlign::Left,
                text: s,
            }),
            self.owner,
        );
    }
}

impl Layout {
    fn sink(&mut self) -> CmdSink<'_> {
        CmdSink::new(&mut self.cmds, "chart")
    }
}

pub(crate) struct BuildInput<'a> {
    pub tab: UiTab,
    pub rows: &'a [UiRow],
    pub meta: UiMeta,
    pub footer: &'a str,
    pub hover_row: Option<usize>,
    /// Emit only the scrolling content from y 0: the run panel splices it
    /// and pins the meta line in its own header. Default false.
    pub skip_chrome: bool,
    /// The combat tab's roster avatars; empty renders no avatar row.
    pub avatars: &'a [AvatarFact],
    /// Flat insets + border rects; plate mode widens the insets to the
    /// nine-patch padding. Default true.
    pub flat_chrome: bool,
    /// Without sprites the strip falls back to text tabs. Default false.
    pub tab_sprites: bool,
    pub width: f32,
    /// Scrollbar reserve; fixed row heights mean the gutter can never
    /// change a panel's overflow verdict and oscillate.
    pub right_gutter: f32,
}

impl Default for BuildInput<'_> {
    fn default() -> Self {
        BuildInput {
            tab: UiTab::default(),
            rows: &[],
            meta: UiMeta::default(),
            footer: "",
            hover_row: None,
            skip_chrome: false,
            avatars: &[],
            flat_chrome: true,
            tab_sprites: false,
            width: PANEL_WIDTH,
            right_gutter: 0.0,
        }
    }
}

/// Per-mille widths, truncated; the tail overruns the bar past 1000.
#[derive(Clone, Copy)]
pub struct Seg {
    pub x: f32,
    pub w: f32,
}

pub(crate) fn segment_offsets(
    seg_milli: &[u16; ui_model::SEG_COUNT],
    width: f32,
) -> [Seg; ui_model::SEG_COUNT] {
    let mut out = [Seg { x: 0.0, w: 0.0 }; ui_model::SEG_COUNT];
    let mut offset: f32 = 0.0;
    for (segment, &milli) in Segment::ALL.iter().zip(seg_milli) {
        // Integer math first, then the float: per-mille × pixel width, with
        // the width truncated to whole pixels (by design).
        let w = (u32::from(milli) * (width as u32) / 1000) as f32;
        out[segment.index()] = Seg { x: offset, w };
        offset += w;
    }
    out
}

pub(crate) fn row_at(hits: &[RowHit], y: f32) -> Option<usize> {
    hits.iter()
        .find(|hit| y >= hit.y0 && y < hit.y1)
        .map(|hit| hit.flat_index)
}

/// The slot under the point, or None; shared by both panels' avatar rows.
pub(crate) fn avatar_at(hits: &[AvatarHit], x: f32, y: f32) -> Option<u8> {
    hits.iter()
        .find(|hit| x >= hit.x0 && x < hit.x1 && y >= hit.y0 && y < hit.y1)
        .map(|hit| hit.slot)
}

pub(crate) fn tab_at(l: &Layout, x: f32, y: f32) -> Option<UiTab> {
    l.tab_hits
        .iter()
        .find(|hit| x >= hit.x0 && x < hit.x1 && y >= hit.y0 && y < hit.y1)
        .map(|hit| hit.tab)
}

pub(crate) fn build(input: BuildInput<'_>) -> Layout {
    let mut content = theme::content_box(input.width, !input.flat_chrome, input.right_gutter);
    // Chrome-less starts at y 0 so the caller's splice translation is
    // exactly its own offset; the x insets stay to align the splice.
    if input.skip_chrome {
        content.top = 0.0;
    }
    // The tab strip floats ABOVE the plate (the game's settings idiom):
    // the band [0, strip_h) extends the Control, and the plate starts
    // below it. Bumping content.top moves every pinned/body emitter into
    // the plate; the strip itself is emitted at y 0. Both layout.height
    // and the Control height include the strip, so it cancels out of the
    // scroll overflow like any other pinned header.
    let strip_h = if input.skip_chrome {
        0.0
    } else {
        TABS_H + STRIP_GAP
    };
    content.top += strip_h;
    let g = Geom::new(content);
    let mut l = Layout {
        width: input.width,
        content,
        strip_h,
        ..Layout::default()
    };
    let mut y = content.top;
    if !input.skip_chrome {
        // The strip's hit boxes stay box-local, so the panel's press
        // zones resolve unchanged.
        emit_tabs(&mut l, &input, &g, 0.0);
        y = emit_title(&mut l, &g);
        // The avatar row pins with the chrome: the filter it drives must
        // never scroll away.
        y = emit_avatars(&mut l, &input, &g, y);
        // The meta line summarizes the body, so it pins with the chrome.
        y = emit_meta(&mut l, &input, &g, y);
    }
    // One Vec during emission keeps the emitters target-agnostic; the
    // drain splits it into the header/body lists the replay draws.
    let header_len = l.cmds.len();
    l.header_bottom = y;
    let mut y = emit_sections(&mut l, &input, &g, y);
    y += 4.0;
    y = emit_lines(&mut l, input.footer, content.x, y, FOOTER_LINE_H, COL_DIM);
    l.height = y + content.outer_bottom_pad;
    l.header_cmds = l.cmds.drain(..header_len).collect();
    if !input.skip_chrome && input.flat_chrome {
        insert_borders(&mut l.header_cmds, "chart", l.width, l.height);
    }
    l
}

/// No portrait: the 40px band cannot hold the game's 64px portrait.
fn emit_title(l: &mut Layout, g: &Geom) -> f32 {
    let y: f32 = g.content.top;
    l.sink()
        .title_text(g.content.x, y + TITLE_Y, "Contribution");
    y + HEADER_H
}

/// The avatar row toggles the player filter on both tabs. Unloaded
/// avatars are skipped, never placeheld; the wanted paths are still
/// recorded so draw can re-resolve them and rebuild the row when a load
/// lands.
fn emit_avatars(l: &mut Layout, input: &BuildInput<'_>, g: &Geom, y_in: f32) -> f32 {
    if input.avatars.is_empty() {
        return y_in;
    }
    let mut x = g.content.x;
    let mut drew = false;
    for (i, avatar) in input.avatars.iter().enumerate() {
        l.portrait_paths.push(avatar.path.clone());
        if !avatar.loaded {
            continue;
        }
        // The portrait art is square, so the destination rect is too.
        l.sink().texture(
            x,
            y_in,
            AVATAR_H,
            AVATAR_H,
            theme::IconId::Character(i as u8),
        );
        l.avatar_hits.push(AvatarHit {
            x0: x,
            y0: y_in,
            x1: x + AVATAR_H,
            y1: y_in + AVATAR_H,
            slot: avatar.slot,
        });
        x += AVATAR_H + AVATAR_GAP;
        drew = true;
    }
    if drew { y_in + AVATAR_H } else { y_in }
}

fn emit_tabs(l: &mut Layout, input: &BuildInput<'_>, g: &Geom, y_in: f32) {
    let y = y_in;
    let strip_w = TAB_W * UiTab::ALL.len() as f32 + TAB_GAP;
    let x0 = g.content.x + ((g.content.w - strip_w) / 2.0).max(0.0);
    for (i, tab) in UiTab::ALL.into_iter().enumerate() {
        let x = x0 + i as f32 * (TAB_W + TAB_GAP);
        let active = input.tab == tab;
        if input.tab_sprites {
            // The plate and stroke share one 515×181 draw frame, so both
            // draw into the tab box.
            l.sink()
                .texture(x, y, TAB_W, TABS_H, theme::IconId::TabPlate);
            if active {
                l.sink()
                    .texture(x, y, TAB_W, TABS_H, theme::IconId::TabStroke);
            }
        }
        push_cmd(
            &mut l.cmds,
            Cmd::Text(TextCmd {
                x,
                y: y + TAB_LABEL_Y,
                size: theme::SIZE_HEADER,
                color: if active { COL_CREAM } else { COL_DIM },
                role: TextRole::Title,
                shadow: true,
                outline: false,
                align: TextAlign::Center(TAB_W),
                text: tab.label().to_owned(),
            }),
            "chart",
        );
        if active && !input.tab_sprites {
            l.sink()
                .rect(x + 24.0, y + TABS_H - 8.0, TAB_W - 48.0, 2.0, COL_GOLD);
        }
        l.tab_hits.push(TabHit {
            x0: x,
            y0: y,
            x1: x + TAB_W,
            y1: y + TABS_H,
            tab,
        });
    }
}

fn emit_meta(l: &mut Layout, input: &BuildInput<'_>, g: &Geom, y_in: f32) -> f32 {
    let mut y = y_in;
    if input.tab == UiTab::Combat {
        let enc = input.meta.encounter_str();
        if !enc.is_empty() {
            l.sink().text_left_clipped(
                g.content.x,
                g.content.w,
                y + META_Y,
                SIZE_BODY,
                COL_CREAM,
                TextRole::Body,
                format!("Vs. {enc}"),
            );
            y += META_H;
        }
    }
    l.sink().text_left_clipped(
        g.content.x,
        g.content.w,
        y + META_Y,
        SIZE_BODY,
        COL_CREAM,
        TextRole::Title,
        meta_line(input.tab, &input.meta),
    );
    y + META_H
}

fn emit_sections(l: &mut Layout, input: &BuildInput<'_>, g: &Geom, y_in: f32) -> f32 {
    let mut y = y_in;
    for section in Section::ALL {
        y = emit_section(l, input, g, section, y);
        y += SECTION_GAP;
    }
    y
}
/// A PanelContainer stylebox would paint over the _draw output, so the
/// border is drawn as commands, pinned at the front of the header. The
/// splice bypasses `push_cmd`, so the cap is re-checked here.
pub(crate) fn insert_borders(header_cmds: &mut Vec<Cmd>, owner: &str, width: f32, height: f32) {
    let borders = crate::ui::panel_common::border_rects(width, height);
    if header_cmds.len() + borders.len() <= MAX_CMDS {
        header_cmds.splice(0..0, borders);
    } else {
        crate::ui::panel_common::log_cmd_overflow_once(owner);
    }
}

/// One shape for every section, so two headers can't drift into
/// underline-crosses-glyphs.
fn emit_section_header(sink: &mut CmdSink, g: &Geom, y: f32, name: &str) {
    let rows_w = g.content.w;
    sink.rect(g.content.x, y, rows_w, SECTION_HEADER_H, COL_HEADER_BG);
    sink.rect(
        g.content.x,
        y + SECTION_UNDERLINE_Y,
        rows_w,
        SECTION_UNDERLINE_H,
        COL_GOLD,
    );
    sink.text_ex(
        g.content.x + 8.0,
        y + SECTION_TITLE_Y,
        SIZE_BODY,
        COL_GOLD,
        TextRole::Title,
        true,
        name,
    );
}

fn emit_section(
    l: &mut Layout,
    input: &BuildInput<'_>,
    g: &Geom,
    section: Section,
    y_in: f32,
) -> f32 {
    let mut y = y_in;
    emit_section_header(&mut l.sink(), g, y, section.name());
    y += SECTION_HEADER_H;

    let mut any = false;
    for (flat, row) in input.rows.iter().enumerate() {
        if row.section != section {
            continue;
        }
        any = true;
        emit_row(l, g, row, flat, input.hover_row == Some(flat), y);
        y += ROW_H;
    }
    if !any {
        l.sink().text(
            g.content.x + 8.0,
            y + ROW_TEXT_Y,
            SIZE_BODY,
            COL_DIM,
            "(none)",
        );
        y += NONE_H;
    }
    y
}

fn emit_row(l: &mut Layout, g: &Geom, row: &UiRow, flat: usize, hovered: bool, y: f32) {
    let is_self = row.flags & ui_model::ROW_FLAG_SELF != 0;
    let solo = row.flags & ui_model::ROW_FLAG_SELF_SOLO != 0;
    let hanging = is_self && !solo;
    let base_y = y + ROW_TEXT_Y;
    emit_row_background(l, g, flat, hovered, y);
    emit_name(l, g, row, hanging, is_self, base_y);
    emit_plays(l, g, row, hanging, base_y);
    emit_segments(l, g, row, y);
    emit_value(l, g, row, is_self, base_y);
    l.row_hits.push(RowHit {
        y0: y,
        y1: y + ROW_H,
        flat_index: flat,
    });
}

fn emit_row_background(l: &mut Layout, g: &Geom, flat: usize, hovered: bool, y: f32) {
    let rows_w = g.content.w;
    if flat.is_multiple_of(2) {
        l.sink().rect(g.content.x, y, rows_w, ROW_H, COL_ROW_ALT);
    }
    if hovered {
        l.sink().rect(g.content.x, y, rows_w, ROW_H, COL_HOVER);
    }
}

/// Hanging self rows show "+ self damage" indented; `is_self` keeps the
/// red for solo rows too, so a solo self row still reads as a cost.
fn emit_name(l: &mut Layout, g: &Geom, row: &UiRow, hanging: bool, is_self: bool, base_y: f32) {
    let name_x = g.content.x + 4.0 + if hanging { SELF_INDENT } else { 0.0 };
    let name_color = if is_self { COL_SELF } else { COL_CREAM };
    if hanging {
        l.sink()
            .text(name_x, base_y, SIZE_BODY, name_color, "+ self damage");
    } else {
        // The kind marker is its own color run; the name follows at the
        // fixed advance.
        let name_run_x = match kind_prefix(row.kind) {
            Some(prefix) => {
                l.sink()
                    .text(name_x, base_y, SIZE_BODY, prefix.color, prefix.text);
                name_x + PREFIX_ADVANCE
            }
            None => name_x,
        };
        l.sink().text(
            name_run_x,
            base_y,
            SIZE_BODY,
            name_color,
            truncate_marked(row.name_str(), NAME_MAX_CHARS),
        );
    }
}

/// Omitted on hanging self rows (the positive row showed it).
fn emit_plays(l: &mut Layout, g: &Geom, row: &UiRow, hanging: bool, base_y: f32) {
    if row.plays > 0 && !hanging {
        l.sink().text(
            g.content.x + NAME_W,
            base_y,
            SIZE_BODY,
            COL_DIM,
            format!("x{}", row.plays),
        );
    }
}

fn emit_segments(l: &mut Layout, g: &Geom, row: &UiRow, y: f32) {
    let bar_y = y + (ROW_H - BAR_H) / 2.0;
    l.sink().rect(g.bar_x, bar_y, g.bar_w, BAR_H, COL_TRACK);
    let segs = segment_offsets(&row.seg_milli, g.bar_w);
    for (segment, seg) in Segment::ALL.iter().zip(segs.iter()) {
        if seg.w <= 0.0 {
            continue;
        }
        l.sink().rect(
            g.bar_x + seg.x,
            bar_y,
            seg.w,
            BAR_H,
            slot_color(*segment, row.section, row.kind),
        );
    }
}

/// Self rows show the raw HP cost without a percentage — the percentage
/// denominators exclude self damage by design.
fn emit_value(l: &mut Layout, g: &Geom, row: &UiRow, is_self: bool, base_y: f32) {
    let value_color = if is_self { COL_SELF } else { COL_CREAM };
    let width = g.content.right() - g.value_x;
    if is_self {
        l.sink().text_right(
            g.value_x,
            width,
            base_y,
            SIZE_BODY,
            value_color,
            format!("{}", row.value),
        );
    } else {
        // Truncating division: whole.frac percent.
        l.sink().text_right(
            g.value_x,
            width,
            base_y,
            SIZE_BODY,
            value_color,
            format!(
                "{}  ({}.{}%)",
                row.value,
                row.share_x10 / 10,
                row.share_x10 % 10
            ),
        );
    }
}

/// Totals joined with the game's "·" middot idiom, no name prefix.
pub(crate) fn meta_line(tab: UiTab, meta: &UiMeta) -> String {
    let dps_whole = meta.dps_x10 / 10;
    let dps_frac = meta.dps_x10 % 10;
    if tab == UiTab::Run {
        if meta.turns == 0 {
            return format!("DPS — · {} combats", meta.combats);
        }
        return format!(
            "DPS {}.{} · {} turns · {} combats",
            dps_whole, dps_frac, meta.turns, meta.combats
        );
    }
    if meta.turns == 0 {
        return format!("DPS — · {} plays", meta.plays);
    }
    format!(
        "DPS {}.{} · {} turns · {} plays · took {}",
        dps_whole, dps_frac, meta.turns, meta.plays, meta.damage_taken
    )
}

fn emit_lines(l: &mut Layout, text: &str, x: f32, y_in: f32, line_h: f32, color: Color) -> f32 {
    let mut y = y_in;
    // take() before filter(): empty lines count against the cap but emit
    // nothing and do not advance y.
    for line in text.lines().take(MAX_LINES).filter(|line| !line.is_empty()) {
        l.sink().text(x, y + ROW_TEXT_Y, SIZE_BODY, color, line);
        y += line_h;
    }
    y
}

/// Byte truncation that never splits a UTF-8 codepoint.
// TODO: truncate by rendered width, not bytes — a long localized name can
// still run into the plays column. That needs font metrics, and measuring
// dispatches ON the engine-created Font object, a call shape the fork
// discipline forbids (docs/gdextension.md); a safe route would be width
// tables measured once through the shim's managed fonts.
pub(crate) fn truncate(s: &str, max: usize) -> &str {
    &s[..s.floor_char_boundary(max.min(s.len()))]
}

/// The game's exact "⋯" (U+22EF) is not in Kreon's cmap, so the standard
/// "…" is used (coverage-verified against the shipped TTFs).
pub(crate) const TRUNCATION_MARK: &str = "…";

pub(crate) fn truncate_marked(s: &str, max: usize) -> String {
    debug_assert!(max >= 1, "the budget holds at least the marker");
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push_str(TRUNCATION_MARK);
    out
}

#[cfg(test)]
mod tests;
