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

/// The common id + "[R] " prefix at the measured 24px advances (259px of
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowHit {
    pub y0: f32,
    pub y1: f32,
    pub flat_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug)]
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
        if meta.dps_x10 < 0 {
            return format!("DPS — · {} combats", meta.combats);
        }
        return format!(
            "DPS {}.{} · {} turns · {} combats",
            dps_whole, dps_frac, meta.turns, meta.combats
        );
    }
    if meta.dps_x10 < 0 {
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
mod tests {
    use super::*;
    use crate::test_util::{cmd_texts as texts, test_row};

    /// The tab/rows/meta core of a BuildInput with default chrome; a
    /// test overlays its tweaks via `..build_input(..)`.
    fn build_input(tab: UiTab, rows: &[UiRow], meta: UiMeta) -> BuildInput<'_> {
        BuildInput {
            tab,
            rows,
            meta,
            ..BuildInput::default()
        }
    }

    #[test]
    fn scaled_texture_rects_keep_their_original_center() {
        let tex = TextureCmd {
            x: 100.0,
            y: 40.0,
            w: 64.0,
            h: 64.0,
            icon: theme::IconId::Character(0),
        };

        assert_eq!(
            tex.scaled_rect(1.0),
            Rect2::new(Vector2::new(100.0, 40.0), Vector2::new(64.0, 64.0))
        );
        assert_eq!(
            tex.scaled_rect(1.1),
            Rect2::new(Vector2::new(96.8, 36.8), Vector2::new(70.4, 70.4))
        );
        assert_eq!(
            tex.scaled_rect(0.95),
            Rect2::new(Vector2::new(101.6, 41.6), Vector2::new(60.8, 60.8))
        );
    }

    #[test]
    fn segment_offsets_per_mille_to_pixels_uncapped_tail() {
        let segs = [500, 250, 0, 0, 0, 0, 0];
        let px = segment_offsets(&segs, 300.0);
        assert_eq!(px[0].x, 0.0);
        assert_eq!(px[0].w, 150.0);
        assert_eq!(px[1].x, 150.0);
        assert_eq!(px[1].w, 75.0);
        assert_eq!(px[2].w, 0.0);

        // Sum > 1000 overflows the bar instead of clamping (by design).
        let over = [800, 400, 0, 0, 0, 0, 0];
        let px_over = segment_offsets(&over, 300.0);
        assert_eq!(px_over[0].w, 240.0);
        assert_eq!(px_over[1].x, 240.0);
        assert_eq!(px_over[1].w, 120.0);

        // Truncation, not rounding: 1/3 of 300 = 99.9 -> 99.
        let third = [333, 0, 0, 0, 0, 0, 0];
        assert_eq!(segment_offsets(&third, 300.0)[0].w, 99.0);
    }

    fn with_encounter(mut meta: UiMeta, enc: &str) -> UiMeta {
        meta.encounter[..enc.len()].copy_from_slice(enc.as_bytes());
        meta.encounter_len = enc.len() as u8;
        meta
    }

    fn combat_tab_layout() -> Layout {
        combat_tab_layout_mode(true, false, 0.0)
    }

    fn combat_tab_layout_mode(flat_chrome: bool, tab_sprites: bool, right_gutter: f32) -> Layout {
        let rows = [
            test_row(
                Section::Damage,
                SourceKind::Card,
                0,
                "STRIKE",
                2,
                20,
                487,
                [769, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Defense,
                SourceKind::Card,
                0,
                "CRIMSON_MANTLE",
                1,
                10,
                1000,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Defense,
                SourceKind::Card,
                ui_model::ROW_FLAG_SELF,
                "CRIMSON_MANTLE",
                1,
                -3,
                0,
                [0, 0, 0, 0, 0, 0, 428],
            ),
        ];
        let meta = with_encounter(
            UiMeta {
                turns: 2,
                dps_x10: 100,
                ..UiMeta::default()
            },
            "BYGONE_EFFIGY",
        );
        build(BuildInput {
            flat_chrome,
            tab_sprites,
            right_gutter,
            ..build_input(UiTab::Combat, &rows, meta)
        })
    }

    #[test]
    fn layout_sections_row_order_self_rendering_hit_tests() {
        let l = combat_tab_layout();
        assert!(!l.cmds.is_empty());
        assert_eq!(l.row_hits.len(), 3);

        let y0 = l.row_hits[0].y0;
        assert_eq!(row_at(&l.row_hits, y0 + 1.0), Some(0));
        assert_eq!(row_at(&l.row_hits, l.row_hits[1].y0 + 1.0), Some(1));
        assert_eq!(row_at(&l.row_hits, l.height - 1.0), None);

        let mut found_self_bar = false;
        let mut found_self_label = false;
        for cmd in &l.cmds {
            match cmd {
                Cmd::Rect(r) => {
                    if r.color[0] == COL_SELF[0] && r.color[1] == COL_SELF[1] && r.w > 0.0 {
                        found_self_bar = true;
                    }
                }
                Cmd::Text(t) => {
                    if t.text.contains("self damage") {
                        found_self_label = true;
                    }
                }
                Cmd::Texture(_) => {}
            }
        }
        assert!(found_self_bar);
        assert!(found_self_label);

        assert_eq!(l.tab_hits.len(), 2);
        assert_eq!(
            tab_at(&l, l.tab_hits[1].x0 + 1.0, l.tab_hits[1].y0 + 1.0),
            Some(UiTab::Run)
        );
        assert_eq!(tab_at(&l, 2.0, 2.0), None);
    }

    #[test]
    fn tab_strip_sprites_and_text_fallback_modes() {
        let sprite = combat_tab_layout_mode(true, true, 0.0);
        let text = combat_tab_layout_mode(true, false, 0.0);
        let textures: Vec<&TextureCmd> = sprite
            .header_cmds
            .iter()
            .filter_map(|cmd| match cmd {
                Cmd::Texture(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(textures.len(), 3, "two plates + the active stroke");
        assert_eq!(textures[0].icon, theme::IconId::TabPlate);
        assert_eq!(textures[1].icon, theme::IconId::TabStroke);
        assert_eq!(textures[2].icon, theme::IconId::TabPlate);
        assert_eq!(textures[0].w, TAB_W);
        assert_eq!(textures[0].h, TABS_H);
        assert_eq!(textures[0].x, textures[1].x);
        assert_eq!(textures[0].y, textures[1].y);
        assert!(
            !text
                .header_cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Texture(_))),
            "the text fallback draws no sprites"
        );
        assert!(
            text.header_cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Rect(r) if r.color == COL_GOLD && r.w < TAB_W)),
            "the text fallback marks the active tab with the underline"
        );
        assert!(
            !sprite
                .header_cmds
                .iter()
                .any(|cmd| matches!(cmd, Cmd::Rect(r) if r.color == COL_GOLD && r.w < TAB_W)),
            "the sprite mode's active marker is the stroke, not the underline"
        );
        for (a, b) in ["This Combat", "Run Summary"].map(|label| {
            let find = |l: &Layout| {
                l.header_cmds
                    .iter()
                    .find_map(|cmd| match cmd {
                        Cmd::Text(t) if t.text == label => Some(t.clone()),
                        _ => None,
                    })
                    .expect("the tab label renders")
            };
            (find(&sprite), find(&text))
        }) {
            assert_eq!(a, b, "the label command is mode-independent");
            assert_eq!(a.align, TextAlign::Center(TAB_W));
            assert_eq!(a.size, theme::SIZE_HEADER);
        }
        assert_eq!(sprite.tab_hits, text.tab_hits);
    }

    #[test]
    fn layout_value_share_formatting_and_self_row_semantics() {
        let rows = [
            test_row(
                Section::Damage,
                SourceKind::Card,
                0,
                "STRIKE",
                2,
                20,
                487,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Defense,
                SourceKind::Card,
                ui_model::ROW_FLAG_SELF,
                "OFFERING",
                1,
                -6,
                0,
                [0, 0, 0, 0, 0, 0, 500],
            ),
        ];
        let meta = UiMeta::default();
        let l = build(build_input(UiTab::Combat, &rows, meta));

        let mut found_share = false;
        let mut found_self_value = false;
        for t in texts(&l.cmds) {
            if t == "20  (48.7%)" {
                found_share = true;
            }
            if t == "-6" {
                found_self_value = true;
            }
            assert!(!t.contains("-6  ("), "self row got a share suffix: {t:?}");
        }
        assert!(found_share);
        assert!(found_self_value);
    }

    #[test]
    fn layout_solo_self_row_shows_the_card_name_and_plays() {
        let rows = [test_row(
            Section::Defense,
            SourceKind::Card,
            ui_model::ROW_FLAG_SELF | ui_model::ROW_FLAG_SELF_SOLO,
            "BLOODLETTING",
            3,
            -9,
            0,
            [0, 0, 0, 0, 0, 0, 1000],
        )];
        let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));
        let all: Vec<&str> = texts(&l.cmds).collect();
        assert!(
            all.iter().any(|t| t.contains("BLOODLETTING")),
            "solo self row must show the card name: {all:?}"
        );
        assert!(
            !all.iter().any(|t| t.contains("self damage")),
            "solo self row must not use the hanging label: {all:?}"
        );
        assert!(all.contains(&"x3"), "plays shown: {all:?}");
        assert!(
            all.contains(&"-9"),
            "red raw cost, no share suffix: {all:?}"
        );
        assert!(!all.iter().any(|t| t.contains("-9  (")));
    }

    #[test]
    fn layout_run_tab_renders_the_dps_dash_without_turns() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            5,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        let meta = UiMeta {
            combats: 3,
            dps_x10: -1,
            ..UiMeta::default()
        };
        let l = build(build_input(UiTab::Run, &rows, meta));

        let dps_dash = texts(&l.header_cmds).any(|t| t.contains("DPS —"));
        assert!(dps_dash);
        assert!(texts(&l.header_cmds).any(|t| t == "DPS — · 3 combats"));
    }

    #[test]
    fn meta_block_formats_the_stats_line_per_tab() {
        let meta = with_encounter(
            UiMeta {
                turns: 4,
                plays: 14,
                dps_x10: 405,
                damage_taken: 27,
                combats: 2,
                ..UiMeta::default()
            },
            "DEVOTED_SCULPTOR_WEAK",
        );
        assert_eq!(
            meta_line(UiTab::Combat, &meta),
            "DPS 40.5 · 4 turns · 14 plays · took 27"
        );
        assert_eq!(
            meta_line(UiTab::Run, &meta),
            "DPS 40.5 · 4 turns · 2 combats"
        );
        let no_dps = UiMeta {
            dps_x10: -1,
            ..meta
        };
        assert_eq!(meta_line(UiTab::Combat, &no_dps), "DPS — · 14 plays");
        assert_eq!(meta_line(UiTab::Run, &no_dps), "DPS — · 2 combats");
    }

    #[test]
    fn encounter_line_emits_untruncated_and_clipped() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        let build_with = |enc: &str, flat_chrome: bool| {
            let meta = with_encounter(
                UiMeta {
                    turns: 2,
                    dps_x10: 100,
                    ..UiMeta::default()
                },
                enc,
            );
            build(BuildInput {
                flat_chrome,
                ..build_input(UiTab::Combat, &rows, meta)
            })
        };
        // A free fn, not a closure: the return borrows the layout alone,
        // which a closure's lifetime elision cannot express.
        fn find_line<'a>(l: &'a Layout, prefix: &str) -> &'a TextCmd {
            l.header_cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Text(t) if t.text.starts_with(prefix) => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the {prefix} line renders"))
        }
        for flat_chrome in [true, false] {
            let l = build_with("BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER", flat_chrome);
            let enc_line = find_line(&l, "Vs. ");
            assert_eq!(enc_line.text, "Vs. BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER");
            assert_eq!(enc_line.role, TextRole::Body, "a name, not a number");
            let stats = find_line(&l, "DPS ");
            assert_eq!(stats.role, TextRole::Title, "the number idiom");
            assert_eq!(stats.y, enc_line.y + META_H, "stats under the encounter");
            for line in [enc_line, stats] {
                assert_eq!(line.x, l.content.x);
                assert_eq!(line.align, TextAlign::LeftClipped(l.content.w));
            }
            let l = build_with("", flat_chrome);
            assert!(
                !texts(&l.header_cmds).any(|t| t.starts_with("Vs. ")),
                "no bare Vs. line"
            );
            assert!(texts(&l.header_cmds).any(|t| t.starts_with("DPS ")));
        }
    }

    #[test]
    fn encounter_line_fits_the_longest_real_slug() {
        // fontTools-measured Kreon Regular hmtx advances at 24px plus the
        // 1px/char tracking.
        const HEAD: f32 = 38.8; // "Vs. "
        const SLUG_CHAR: f32 = 15.24; // the uppercase-slug mean + tracking
        const CONTENT_W: f32 = 713.0; // the plate chrome's content width
        const LONGEST_REAL_SLUG: f32 = 35.0; // BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER
        let line = HEAD + LONGEST_REAL_SLUG * SLUG_CHAR;
        assert!(line <= CONTENT_W, "the longest real slug fits: {line}");
    }

    #[test]
    fn truncate_marked_swaps_the_last_char_for_the_marker() {
        assert_eq!(truncate_marked("STRIKE", 16), "STRIKE");
        assert_eq!(truncate_marked("ABCDEFGHIJKLMNOP", 16), "ABCDEFGHIJKLMNOP");
        assert_eq!(truncate_marked("ABCDEFGHIJKLMNOPQ", 16), "ABCDEFGHIJKLMNO…");
        let wide = format!("{}É", "A".repeat(17));
        assert_eq!(truncate_marked(&wide, 16), "AAAAAAAAAAAAAAA…");
        assert!(truncate_marked(&wide, 16).chars().count() == 16);
    }

    #[test]
    fn title_band_never_renders_a_player_filter_label() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        for tab in [UiTab::Combat, UiTab::Run] {
            let l = build(build_input(tab, &rows, UiMeta::default()));
            assert!(
                !texts(&l.header_cmds).any(|t| t == "All" || t == "P1" || t == "P2"),
                "the avatar row carries the filter state, never a text label"
            );
        }
    }

    fn avatar_facts() -> Vec<AvatarFact> {
        vec![
            AvatarFact {
                slot: 0,
                loaded: true,
                path: "res://images/ui/top_panel/character_icon_ironclad.png".to_owned(),
            },
            AvatarFact {
                slot: 1,
                loaded: true,
                path: "res://images/ui/top_panel/character_icon_silent.png".to_owned(),
            },
        ]
    }

    fn avatar_fact(slot: u8, loaded: bool) -> AvatarFact {
        AvatarFact {
            slot,
            loaded,
            path: format!("res://images/ui/top_panel/character_icon_{slot}.png"),
        }
    }

    fn combat_build(avatars: &[AvatarFact]) -> Layout {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        build(BuildInput {
            avatars,
            ..build_input(UiTab::Combat, &rows, UiMeta::default())
        })
    }

    fn icons_of(l: &Layout) -> Vec<&TextureCmd> {
        l.header_cmds
            .iter()
            .filter_map(|cmd| match cmd {
                Cmd::Texture(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Without the avatar row the header flow is title + meta (the tabs
    /// live in the strip band, already inside `content.top`).
    fn bare_header_bottom(content: theme::ContentBox) -> f32 {
        content.top + HEADER_H + META_H
    }

    #[test]
    fn avatar_row_emits_textures_hits_and_paths_on_the_combat_tab() {
        let l = combat_build(&avatar_facts());
        let icons = icons_of(&l);
        assert_eq!(icons.len(), 2, "two loaded avatars draw");
        assert_eq!(icons[0].icon, theme::IconId::Character(0));
        assert_eq!(icons[0].w, icons[0].h, "the portrait art is square");
        assert_eq!(icons[0].w, AVATAR_H);
        assert_eq!(icons[1].x, icons[0].x + AVATAR_H + AVATAR_GAP);
        assert_eq!(
            l.portrait_paths,
            avatar_facts()
                .iter()
                .map(|a| a.path.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(l.avatar_hits.len(), 2);
        assert_eq!(
            l.avatar_hits[0],
            AvatarHit {
                x0: l.content.x,
                y0: l.content.top + HEADER_H,
                x1: l.content.x + AVATAR_H,
                y1: l.content.top + HEADER_H + AVATAR_H,
                slot: 0,
            }
        );
        assert_eq!(l.avatar_hits[1].slot, 1);
        // The row pins with the chrome: body starts past the avatar row.
        assert_eq!(l.header_bottom, bare_header_bottom(l.content) + AVATAR_H);
    }

    #[test]
    fn avatar_at_maps_presses_inside_the_boxes_only() {
        let l = combat_build(&avatar_facts());
        let hit = l.avatar_hits[1];
        assert_eq!(
            avatar_at(&l.avatar_hits, hit.x0 + 1.0, hit.y0 + 1.0),
            Some(1)
        );
        assert_eq!(avatar_at(&l.avatar_hits, hit.x1, hit.y0 + 1.0), None);
        assert_eq!(
            avatar_at(
                &l.avatar_hits,
                l.avatar_hits[0].x0 + 1.0,
                l.avatar_hits[0].y1
            ),
            None
        );
        assert_eq!(avatar_at(&[], 5.0, 5.0), None);
    }

    #[test]
    fn avatar_row_renders_on_the_run_tab_too() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        let l = build(BuildInput {
            avatars: &avatar_facts(),
            ..build_input(UiTab::Run, &rows, UiMeta::default())
        });
        assert_eq!(icons_of(&l).len(), 2, "the run tab carries the row");
        assert_eq!(l.avatar_hits.len(), 2);
        assert_eq!(
            l.portrait_paths,
            avatar_facts()
                .iter()
                .map(|a| a.path.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(l.avatar_hits[0].slot, 0);
        assert_eq!(l.avatar_hits[1].slot, 1);
        assert_eq!(l.header_bottom, bare_header_bottom(l.content) + AVATAR_H);
    }

    #[test]
    fn avatar_row_skips_unloaded_avatars_but_records_their_paths() {
        let facts = vec![avatar_fact(0, false), avatar_fact(1, true)];
        let l = combat_build(&facts);
        assert_eq!(
            l.portrait_paths.len(),
            2,
            "paths record wanted, loaded or not"
        );
        let icons = icons_of(&l);
        assert_eq!(icons.len(), 1, "only the loaded avatar draws");
        assert_eq!(
            icons[0].icon,
            theme::IconId::Character(1),
            "index stays roster-relative"
        );
        assert_eq!(l.avatar_hits.len(), 1);
        assert_eq!(l.avatar_hits[0].slot, 1);
    }

    #[test]
    fn avatar_row_collapses_when_nothing_draws() {
        let l = combat_build(&[avatar_fact(0, false)]);
        assert!(l.avatar_hits.is_empty());
        assert_eq!(l.portrait_paths.len(), 1, "the wanted path still records");
        assert_eq!(l.header_bottom, bare_header_bottom(l.content));

        let l = combat_build(&[]);
        assert!(l.avatar_hits.is_empty() && l.portrait_paths.is_empty());
    }

    #[test]
    fn chrome_less_build_emits_only_the_sections() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        let meta = UiMeta {
            turns: 3,
            combats: 1,
            dps_x10: 30,
            ..UiMeta::default()
        };
        let l = build(BuildInput {
            skip_chrome: true,
            ..build_input(UiTab::Run, &rows, meta)
        });
        assert!(l.tab_hits.is_empty(), "no tab strip without chrome");
        assert!(
            l.header_cmds.is_empty() && l.header_bottom == 0.0,
            "the chrome-less build is all body"
        );
        let t: Vec<&str> = texts(&l.cmds).collect();
        assert!(!t.contains(&"Contribution"), "no title band without chrome");
        assert!(!t.contains(&"This Combat"), "no tab labels without chrome");
        assert!(
            !t.iter().any(|s| s.starts_with("DPS ")),
            "no meta line: the splicing panel pins it"
        );
        assert!(t.contains(&"Damage"));
        assert!(t.contains(&"STRIKE"));
        // Content starts at y 0, so a caller splicing at its own offset
        // translates by exactly that offset.
        let first_y = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) => Some(r.y),
                _ => None,
            })
            .expect("the section band renders first");
        assert_eq!(first_y, 0.0);
        assert!(l.height > 0.0);
    }

    #[test]
    fn layout_emits_every_row_without_height_cap() {
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        for (i, row) in rows.iter_mut().enumerate() {
            let section = if i % 2 == 0 {
                Section::Damage
            } else {
                Section::Defense
            };
            let name = format!("CARD{i}");
            *row = test_row(
                section,
                SourceKind::Card,
                0,
                &name,
                1,
                10,
                10,
                [500, 0, 0, 0, 0, 0, 0],
            );
        }
        let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));

        assert_eq!(l.row_hits.len(), ui_model::MAX_UI_ROWS);
        for hit in &l.row_hits {
            assert_eq!(row_at(&l.row_hits, hit.y0 + 1.0), Some(hit.flat_index));
        }

        assert!(!texts(&l.cmds).any(|t| t.contains("truncated")));

        // The height is asserted region by region (not the sum) so a
        // geometry tweak names the part it regressed; the fixture's meta
        // carries no encounter, so the block is one stats line. The strip
        // band sits above the plate but counts into the Control height.
        let sections = 2.0 * SECTION_HEADER_H + 2.0 * 128.0 * ROW_H + 2.0 * SECTION_GAP;
        assert_eq!(sections, 8_292.0);
        let chrome_and_headers =
            theme::FLAT_PAD + TABS_H + STRIP_GAP + HEADER_H + META_H + 4.0 + theme::FLAT_PAD;
        assert_eq!(chrome_and_headers, 204.0);
        assert_eq!(l.height, sections + chrome_and_headers);
    }

    #[test]
    fn layout_reflows_to_the_build_width() {
        let rows = [test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            9,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        )];
        let track_w = |width: f32| {
            let l = build(BuildInput {
                width,
                ..build_input(UiTab::Combat, &rows, UiMeta::default())
            });
            assert_eq!(l.width, width, "the layout reports the build width");
            l.cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Rect(r) if r.color == COL_TRACK => Some(r.w),
                    _ => None,
                })
                .expect("every row emits a bar track")
        };
        // Designed 240px bar in flat chrome; plate insets leave 197px.
        assert_eq!(track_w(PANEL_WIDTH), 240.0);
        assert_eq!(track_w(PANEL_WIDTH + 200.0), 440.0);
        assert_eq!(track_w(PANEL_WIDTH - 150.0), 90.0);
        assert_eq!(track_w(100.0), MIN_BAR_W);
    }

    #[test]
    fn worst_case_command_count_fits_the_cap() {
        // Full complement per row (relic kind, so the prefix run emits too).
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        for (i, row) in rows.iter_mut().enumerate() {
            let section = if i % 2 == 0 {
                Section::Damage
            } else {
                Section::Defense
            };
            *row = test_row(
                section,
                SourceKind::Relic,
                0,
                &format!("CARD{i}"),
                1,
                10,
                10,
                [1000; 7],
            );
        }
        let l = build(BuildInput {
            tab: UiTab::Combat,
            rows: &rows,
            meta: UiMeta::default(),
            footer: &"footer\n".repeat(64),
            hover_row: Some(0),
            skip_chrome: false,
            avatars: &[],
            flat_chrome: true,
            // The sprite tabs add three texture commands over the text
            // tabs; the cap must see them.
            tab_sprites: true,
            width: PANEL_WIDTH,
            right_gutter: 0.0,
        });
        assert!(
            l.cmds.len() < MAX_CMDS,
            "the cap must fit the true worst case ({} cmds)",
            l.cmds.len()
        );
        assert_eq!(l.row_hits.len(), ui_model::MAX_UI_ROWS);
    }

    #[test]
    fn insert_borders_splices_only_under_the_cap() {
        let border = || {
            Cmd::Rect(RectCmd {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
                color: COL_TRACK,
            })
        };
        let mut full: Vec<Cmd> = (0..MAX_CMDS).map(|_| border()).collect();
        insert_borders(&mut full, "chart", 100.0, 100.0);
        assert_eq!(full.len(), MAX_CMDS, "at the cap the borders are dropped");

        let mut empty = Vec::new();
        insert_borders(&mut empty, "chart", 100.0, 100.0);
        assert_eq!(empty.len(), 4, "under the cap the four edges splice in");
    }

    #[test]
    fn section_band_and_underline_span_exactly_the_content_width() {
        for (flat_chrome, tab_sprites, gutter) in
            [(true, false, 0.0), (false, false, 0.0), (false, true, 32.0)]
        {
            let l = combat_tab_layout_mode(flat_chrome, tab_sprites, gutter);
            let rows_w = l.content.w;
            // The gold rect spanning the rows region (text tabs' underline
            // is one tab wide).
            let underline = l
                .cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Rect(r) if r.color == COL_GOLD && r.w == rows_w => Some(r),
                    _ => None,
                })
                .expect("the damage section underline renders");
            assert_eq!(underline.x, l.content.x, "flat_chrome={flat_chrome}");
            let band = l
                .cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Rect(r) if r.color == COL_HEADER_BG => Some(r),
                    _ => None,
                })
                .expect("the section header band renders");
            assert_eq!(band.x, l.content.x, "flat_chrome={flat_chrome}");
            assert_eq!(band.w, rows_w, "flat_chrome={flat_chrome}");
        }
    }

    #[test]
    fn section_header_underline_sits_below_the_title_inside_the_band() {
        // The 24px title's descent reaches ~7px under the baseline; the
        // emitted commands must agree with the compile-time pins.
        let l = combat_tab_layout();
        let rows_w = l.content.w;
        let underline = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) if r.color == COL_GOLD && r.w == rows_w => Some(r),
                _ => None,
            })
            .expect("the damage underline renders");
        let title = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == "Damage" => Some(t),
                _ => None,
            })
            .expect("the damage title renders");
        assert!(underline.y >= title.y + 7.0, "underline crosses the glyphs");
        let band = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) if r.color == COL_HEADER_BG => Some(r),
                _ => None,
            })
            .expect("the header band renders");
        assert!(title.y >= band.y && title.y <= band.y + band.h);
        assert!(underline.y + underline.h <= band.y + band.h);
    }

    #[test]
    fn the_legend_is_gone_from_the_command_lists() {
        let l = combat_tab_layout();
        for label in ["direct", "str down", "self dmg"] {
            assert!(
                !texts(&l.cmds)
                    .chain(texts(&l.header_cmds))
                    .any(|t| t == label),
                "the key renders on its own plate, not in the panel: {label}"
            );
        }
        assert_eq!(
            l.height,
            12.0 + TABS_H + STRIP_GAP + HEADER_H + 2.0 * META_H + 196.0 + 4.0 + 12.0
        );
    }

    #[test]
    fn kind_prefix_renders_as_a_separate_colored_run() {
        let rows = [
            test_row(
                Section::Damage,
                SourceKind::Relic,
                0,
                "CRIMSON_MANTLE",
                1,
                10,
                1000,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Damage,
                SourceKind::Card,
                0,
                "STRIKE",
                1,
                10,
                1000,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
        ];
        let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));
        let find = |text: &str| {
            l.cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Text(t) if t.text == text => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{text:?} renders"))
        };
        let name_x = l.content.x + 4.0;
        let prefix = find("[R] ");
        assert_eq!(prefix.x, name_x);
        assert_eq!(prefix.color, COL_GOLD, "the relic marker is gold");
        let name = find("CRIMSON_MANTLE");
        assert_eq!(name.x, name_x + PREFIX_ADVANCE);
        assert_eq!(name.color, COL_CREAM);
        assert_eq!(prefix.y, name.y, "the two runs share the baseline");
        assert_eq!(find("STRIKE").x, name_x);
    }

    #[test]
    fn every_command_lies_inside_the_content_box() {
        for (flat_chrome, tab_sprites, gutter) in
            [(true, false, 0.0), (false, false, 0.0), (false, true, 32.0)]
        {
            let l = combat_tab_layout_mode(flat_chrome, tab_sprites, gutter);
            let chrome: Vec<Cmd> = if flat_chrome {
                crate::ui::panel_common::border_rects(l.width, l.height).to_vec()
            } else {
                Vec::new()
            };
            crate::test_util::assert_layout_bounds(
                &l.header_cmds,
                &l.cmds,
                l.content,
                l.header_bottom,
                l.height,
                l.strip_h,
                &chrome,
            );
        }
    }

    #[test]
    fn header_carries_the_title_tabs_and_meta_body_starts_at_the_sections() {
        for flat_chrome in [true, false] {
            let l = combat_tab_layout_mode(flat_chrome, false, 0.0);
            let header: Vec<&str> = texts(&l.header_cmds).collect();
            assert!(header.contains(&"Contribution"), "flat={flat_chrome}");
            assert!(header.contains(&"This Combat"), "flat={flat_chrome}");
            assert!(header.contains(&"Run Summary"), "flat={flat_chrome}");
            assert!(
                header.iter().any(|t| t.starts_with("Vs. ")),
                "the encounter line pins, flat={flat_chrome}"
            );
            assert!(
                header.iter().any(|t| t.starts_with("DPS ")),
                "the stats line pins, flat={flat_chrome}"
            );
            assert!(
                !texts(&l.cmds)
                    .any(|t| t == "Contribution" || t.starts_with("Vs. ") || t.starts_with("DPS ")),
                "header content must not scroll, flat={flat_chrome}"
            );
            assert_eq!(
                l.header_bottom,
                l.content.top + HEADER_H + 2.0 * META_H,
                "the tabs moved out of the header flow, flat={flat_chrome}"
            );
            // The tab strip itself floats in the band above the plate.
            assert_eq!(l.strip_h, TABS_H + STRIP_GAP, "flat={flat_chrome}");
            assert!(
                l.tab_hits
                    .iter()
                    .all(|hit| hit.y1 <= TABS_H && hit.y0 >= 0.0),
                "the tab boxes live entirely in the strip band, flat={flat_chrome}"
            );
            let first = l
                .cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Text(t) => Some(t),
                    _ => None,
                })
                .expect("a section title renders");
            assert_eq!(first.text, "Damage");
            assert_eq!(
                first.y,
                l.header_bottom + SECTION_TITLE_Y,
                "flat={flat_chrome}"
            );
        }
    }

    fn golden_fixture(flat_chrome: bool) -> Layout {
        let rows = [
            test_row(
                Section::Damage,
                SourceKind::Card,
                0,
                "STRIKE",
                2,
                20,
                487,
                [769, 231, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Damage,
                SourceKind::Relic,
                0,
                "SEVER_SOUL",
                1,
                12,
                292,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Defense,
                SourceKind::Card,
                0,
                "DEFEND",
                3,
                15,
                1000,
                [1000, 0, 0, 0, 0, 0, 0],
            ),
            test_row(
                Section::Defense,
                SourceKind::Card,
                ui_model::ROW_FLAG_SELF,
                "OFFERING",
                1,
                -6,
                0,
                [0, 0, 0, 0, 0, 0, 1000],
            ),
        ];
        let meta = with_encounter(
            UiMeta {
                turns: 2,
                plays: 7,
                dps_x10: 160,
                damage_taken: 6,
                ..UiMeta::default()
            },
            "BYGONE_EFFIGY",
        );
        build(BuildInput {
            footer: "Total 32 damage in 2 turns",
            hover_row: Some(0),
            flat_chrome,
            tab_sprites: !flat_chrome,
            ..build_input(UiTab::Combat, &rows, meta)
        })
    }

    #[test]
    fn right_side_columns_align_to_their_zone_right_edges() {
        let l = golden_fixture(false);
        let cmd = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == "20  (48.7%)" => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the value column renders"));
        let TextAlign::Right(w) = cmd.align else {
            panic!("the value column right-aligns")
        };
        assert_eq!(cmd.x + w, l.content.right(), "value column");
    }

    /// The dump splits header from body, so a wrong-zone command reviews.
    #[test]
    fn golden_chart_commands_flat_chrome() {
        let l = golden_fixture(true);
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }

    #[test]
    fn golden_chart_commands_plate_chrome() {
        let l = golden_fixture(false);
        insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
    }
}
