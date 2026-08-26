//! The panels' draw backend — the `_draw`-path plumbing shared by the two
//! panels: the per-frame font set with the panel-wide glyph-coverage
//! fallback, the text-effect passes, the chrome paints, and the draw-
//! command replay itself. Everything rides the FFI through the safe
//! `Object` newtype; the scissor math is pure and unit-tested.

use crate::engine::gdext::{Object, RetainedVariant};
use crate::engine::math::{Color, Rect2, Vector2};
use crate::engine::object::TextAlign;
use crate::ui::chart_layout::{Cmd, TextCmd};
use crate::ui::palette;
use crate::ui::theme::{IconId, Plate, ScrollbarSprites, TextRole, Theme};
use crate::warn;

/// The retained Variant's object Ref keeps the Font alive between draws.
pub(crate) enum FontState {
    Unfetched,
    Failed,
    Loaded(RetainedVariant),
}

/// A failed fetch disables text permanently.
pub(crate) fn ensure_font(object: &Object, state: &mut FontState, warning: &str) -> bool {
    match state {
        FontState::Loaded(_) => true,
        FontState::Failed => false,
        FontState::Unfetched => match object.get_theme_default_font() {
            Some(resolved) => {
                *state = FontState::Loaded(resolved);
                true
            }
            None => {
                *state = FontState::Failed;
                warn!("{warning}");
                false
            }
        },
    }
}

/// Verified against the shipped cmaps; the game swaps by locale, we fall
/// back by measured coverage. Sorted and non-overlapping for binary search.
const KREON_COVERED: &[(u32, u32)] = &[
    (0x0020, 0x007E),
    (0x00A0, 0x0107),
    (0x010A, 0x0113),
    (0x0116, 0x011B),
    (0x011E, 0x0123),
    (0x0126, 0x0127),
    (0x012A, 0x012B),
    (0x012E, 0x0131),
    (0x0136, 0x0137),
    (0x0139, 0x013E),
    (0x0141, 0x0148),
    (0x014A, 0x014D),
    (0x0150, 0x015B),
    (0x015E, 0x0167),
    (0x016A, 0x016B),
    (0x016E, 0x017E),
    (0x01CD, 0x01DC),
    (0x0218, 0x021B),
    (0x02C6, 0x02C7),
    (0x02D8, 0x02DD),
    (0x0300, 0x0304),
    (0x0306, 0x0308),
    (0x030A, 0x030C),
    (0x0312, 0x0312),
    (0x0323, 0x0324),
    (0x0326, 0x0328),
    (0x03BC, 0x03BC),
    (0x1E80, 0x1E85),
    (0x1EF2, 0x1EF3),
    (0x2013, 0x2014),
    (0x2018, 0x201A),
    (0x201C, 0x201E),
    (0x2020, 0x2022),
    (0x2026, 0x2026),
    (0x2039, 0x203A),
    (0x2044, 0x2044),
    (0x20AC, 0x20AC),
    (0x2122, 0x2122),
    (0x215B, 0x215E),
    (0x2212, 0x2212),
    (0x2260, 0x2260),
];

/// Control chars carry no glyphs and are ignored.
pub(crate) fn kreon_covers(text: &str) -> bool {
    text.chars().all(|c| {
        let cp = c as u32;
        if cp < 0x20 || cp == 0x7F {
            return true;
        }
        KREON_COVERED
            .binary_search_by(|&(lo, hi)| {
                if hi < cp {
                    std::cmp::Ordering::Less
                } else if lo > cp {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    })
}

/// The fallback is panel-wide: per-string selection would mix typefaces
/// across rows of one column.
pub(crate) struct Fonts<'a> {
    default: Option<&'a RetainedVariant>,
    title: Option<&'a RetainedVariant>,
    body: Option<&'a RetainedVariant>,
    force_default: bool,
}

impl<'a> Fonts<'a> {
    pub(crate) fn new(
        object: &Object,
        default_state: &'a mut FontState,
        theme: &'a Theme,
        header: &[Cmd],
        body: &[Cmd],
        detail: &crate::ui::tooltip::RowDetail,
        warn: &str,
    ) -> Fonts<'a> {
        let has_text = header
            .iter()
            .chain(body.iter())
            .any(|cmd| matches!(cmd, Cmd::Text(_)))
            || !detail.is_empty();
        if has_text {
            ensure_font(object, default_state, warn);
        }
        let default = match &*default_state {
            FontState::Loaded(font) => Some(font),
            _ => None,
        };
        let force_default = detail.texts().any(|text| !kreon_covers(text))
            || header
                .iter()
                .chain(body.iter())
                .any(|cmd| matches!(cmd, Cmd::Text(t) if !kreon_covers(&t.text)));
        Fonts {
            default,
            title: theme.face(TextRole::Title),
            body: theme.face(TextRole::Body),
            force_default,
        }
    }

    pub(crate) fn for_role(&self, role: TextRole) -> Option<&RetainedVariant> {
        if self.force_default {
            return self.default;
        }
        match role {
            TextRole::Title => self.title.or(self.default),
            TextRole::Body => self.body.or(self.default),
        }
    }
}

/// A (3,2)-offset 50%-black pass under the main pass; both passes share
/// the alignment box so the shadow never drifts off the glyphs.
#[allow(clippy::too_many_arguments)] // a draw call's full parameter list
pub(crate) fn draw_shadowed_text(
    object: &Object,
    font: Option<&RetainedVariant>,
    pos: Vector2,
    text: &str,
    align: TextAlign,
    size: i32,
    color: palette::Color,
) -> usize {
    let Some(font) = font else {
        return 0;
    };
    let mut errors = 0;
    let shadow = palette::COL_SHADOW;
    if !object.draw_string(
        font,
        pos + Vector2::new(3.0, 2.0),
        text,
        align,
        size,
        Color::from_rgba(shadow[0], shadow[1], shadow[2], shadow[3]),
    ) {
        errors += 1;
    }
    if !object.draw_string(
        font,
        pos,
        text,
        align,
        size,
        Color::from_rgba(color[0], color[1], color[2], color[3]),
    ) {
        errors += 1;
    }
    errors
}

/// `draw_string` has no outline parameter, so the rim is four 1px diagonal
/// passes under the main pass.
#[allow(clippy::too_many_arguments)] // a draw call's full parameter list
pub(crate) fn draw_outlined_text(
    object: &Object,
    font: Option<&RetainedVariant>,
    pos: Vector2,
    text: &str,
    align: TextAlign,
    size: i32,
    color: palette::Color,
) -> usize {
    let Some(font) = font else {
        return 0;
    };
    let mut errors = 0;
    let shadow = palette::COL_HEADER_SHADOW;
    if !object.draw_string(
        font,
        pos + Vector2::new(5.0, 4.0),
        text,
        align,
        size,
        Color::from_rgba(shadow[0], shadow[1], shadow[2], shadow[3]),
    ) {
        errors += 1;
    }
    let outline = palette::COL_HEADER_OUTLINE;
    let outline_color = Color::from_rgba(outline[0], outline[1], outline[2], outline[3]);
    for offset in [
        Vector2::new(-1.0, -1.0),
        Vector2::new(1.0, -1.0),
        Vector2::new(-1.0, 1.0),
        Vector2::new(1.0, 1.0),
    ] {
        if !object.draw_string(font, pos + offset, text, align, size, outline_color) {
            errors += 1;
        }
    }
    if !object.draw_string(
        font,
        pos,
        text,
        align,
        size,
        Color::from_rgba(color[0], color[1], color[2], color[3]),
    ) {
        errors += 1;
    }
    errors
}

/// `origin_x` is the plate box's x inside the Control (a flipped tooltip
/// widens it left); `y0` is the plate's top inside the Control (the
/// combat panel's floating tab strip occupies `[0, y0)`).
pub(crate) fn draw_plate(
    object: &Object,
    plate: &Plate,
    origin_x: f32,
    box_size: Vector2,
    y0: f32,
) -> usize {
    // The plate's own height excludes the top band: the Control is
    // taller than the plate (the strip floats above it), and drawing
    // the plate at the Control's height would push its bottom edge
    // past the clip.
    let plate_size = Vector2::new(box_size.x, box_size.y - y0);
    let (shadow, body) = crate::ui::theme::plate_rects(plate_size);
    let at = |r: Rect2| Rect2::new(r.position + Vector2::new(origin_x, y0), r.size);
    let mut errors = usize::from(!object.draw_style_box(&plate.shadow, at(shadow)));
    errors += usize::from(!object.draw_style_box(&plate.body, at(body)));
    errors
}

/// The plate's flat fallback; the strip band above `y0` is the pinned
/// header fill's job.
pub(crate) fn draw_flat_background(
    object: &Object,
    origin_x: f32,
    width: f32,
    box_height: f32,
    y0: f32,
) {
    object.draw_rect(
        Rect2::new(
            Vector2::new(origin_x, y0),
            Vector2::new(width, box_height - y0),
        ),
        Color::from_rgba(
            palette::COL_PANEL_BG[0],
            palette::COL_PANEL_BG[1],
            palette::COL_PANEL_BG[2],
            palette::COL_PANEL_BG[3],
        ),
    );
}

/// Plate mode skips the fill: a flat rect would blank the border art.
pub(crate) fn draw_header_fill(
    object: &Object,
    origin_x: f32,
    width: f32,
    header_bottom: f32,
) -> usize {
    let color = palette::COL_PANEL_BG;
    usize::from(!object.draw_rect(
        Rect2::new(
            Vector2::new(origin_x, 0.0),
            Vector2::new(width, header_bottom),
        ),
        Color::from_rgba(color[0], color[1], color[2], color[3]),
    ))
}

/// A skip means the load state flipped mid-frame; the rebuild catches up.
pub(crate) struct IconTextures<'a> {
    pub theme: &'a Theme,
    pub portraits: &'a [String],
    /// Parallel to `portraits`: a dimmed avatar renders at the deselect
    /// modulate (the active filter excludes that player).
    pub dimmed: &'a [bool],
    /// Parallel to `portraits`: the current selection scale.
    pub scales: &'a [f32],
}

impl IconTextures<'_> {
    fn resolve(&self, icon: IconId) -> Option<&RetainedVariant> {
        match icon {
            IconId::Character(i) => self
                .portraits
                .get(usize::from(i))
                .and_then(|path| self.theme.dynamic(path)),
            IconId::TabPlate => self.theme.tab_plate(),
            IconId::TabStroke => self.theme.tab_stroke(),
        }
    }

    fn modulate(&self, icon: IconId) -> Color {
        let m = match icon {
            IconId::TabPlate => crate::ui::theme::TAB_PLATE_MODULATE,
            IconId::TabStroke => crate::ui::theme::TAB_STROKE_MODULATE,
            IconId::Character(i) => {
                let i = usize::from(i);
                debug_assert!(
                    i < self.dimmed.len(),
                    "character icons index the parallel dim mask"
                );
                if self.dimmed.get(i) == Some(&true) {
                    crate::ui::theme::AVATAR_DIM_MODULATE
                } else {
                    [1.0, 1.0, 1.0, 1.0]
                }
            }
        };
        Color::from_rgba(m[0], m[1], m[2], m[3])
    }

    fn scale(&self, icon: IconId) -> f32 {
        match icon {
            IconId::Character(index) => {
                let i = usize::from(index);
                debug_assert!(
                    i < self.scales.len(),
                    "character icons index the parallel scale row"
                );
                self.scales.get(i).copied().unwrap_or(1.0)
            }
            IconId::TabPlate | IconId::TabStroke => 1.0,
        }
    }
}

/// A failed plate asset degrades the side plates to the panel's own flat
/// fallback look.
pub(crate) fn draw_flat_chrome(object: &Object, rect: Rect2) -> usize {
    let bg = palette::COL_PANEL_BG;
    let border = palette::COL_PANEL_BORDER;
    let mut errors = 0;
    let mut draw = |pos: Vector2, size: Vector2, color: palette::Color| {
        errors += usize::from(!object.draw_rect(
            Rect2::new(pos, size),
            Color::from_rgba(color[0], color[1], color[2], color[3]),
        ));
    };
    draw(rect.position, rect.size, bg);
    let right = rect.position.x + rect.size.x;
    let bottom = rect.position.y + rect.size.y;
    draw(rect.position, Vector2::new(rect.size.x, 1.0), border);
    draw(
        Vector2::new(rect.position.x, bottom - 1.0),
        Vector2::new(rect.size.x, 1.0),
        border,
    );
    draw(rect.position, Vector2::new(1.0, rect.size.y), border);
    draw(
        Vector2::new(right - 1.0, rect.position.y),
        Vector2::new(1.0, rect.size.y),
        border,
    );
    errors
}

/// Mouse-transparent like the tooltip: a press on it dismisses. `scratch`
/// is the caller's reused buffer, so a steady frame never allocates.
pub(crate) fn draw_legend(
    object: &Object,
    fonts: &Fonts,
    plate: Option<&Plate>,
    rect: Rect2,
    scratch: &mut Vec<Cmd>,
) -> usize {
    let lp = palette::legend_plate(plate.is_some());
    scratch.clear();
    palette::emit_legend(
        scratch,
        rect.position.x + lp.origin.x,
        rect.position.y + lp.origin.y,
    );
    let mut errors = 0;
    match plate {
        Some(plate) => {
            let (shadow, body) = crate::ui::theme::plate_rects(rect.size);
            let at = |r: Rect2| Rect2::new(r.position + rect.position, r.size);
            errors += usize::from(!object.draw_style_box(&plate.shadow, at(shadow)));
            errors += usize::from(!object.draw_style_box(&plate.body, at(body)));
        }
        None => {
            errors += draw_flat_chrome(object, rect);
        }
    }
    for cmd in scratch.iter() {
        match cmd {
            Cmd::Rect(r) => {
                errors += usize::from(!object.draw_rect(
                    Rect2::new(Vector2::new(r.x, r.y), Vector2::new(r.w, r.h)),
                    Color::from_rgba(r.color[0], r.color[1], r.color[2], r.color[3]),
                ));
            }
            Cmd::Text(t) => {
                errors += replay_text_cmd(object, fonts, t, 0.0, 0.0);
            }
            Cmd::Texture(_) => {}
        }
    }
    errors
}

/// The scrollbar, then the side plates, the hover tooltip LAST.
#[allow(clippy::too_many_arguments)] // one frame's overlay draw context; bundling it further is artificial
pub(crate) fn draw_overlays(
    object: &Object,
    fonts: &Fonts,
    plate: Option<&Plate>,
    scrollbar: Option<(ScrollbarSprites<'_>, crate::ui::scroll::ScrollbarGeom)>,
    origin_x: f32,
    legend: Option<Rect2>,
    legend_scratch: &mut Vec<Cmd>,
    tip: Option<Rect2>,
    tip_lines: &[crate::ui::tooltip::TipLine],
) -> usize {
    let mut errors = 0;
    if let Some((sprites, geom)) = scrollbar {
        errors += draw_scrollbar(object, &sprites, &geom, origin_x);
    }
    if let Some(legend) = legend {
        errors += draw_legend(object, fonts, plate, legend, legend_scratch);
    }
    if let Some(tip) = tip {
        errors += crate::ui::tooltip::draw(object, fonts, plate, tip_lines, tip);
    }
    errors
}

fn replay_text_cmd(
    object: &Object,
    fonts: &Fonts,
    text: &TextCmd,
    origin_x: f32,
    scroll: f32,
) -> usize {
    let Some(font) = fonts.for_role(text.role) else {
        return 0;
    };
    let pos = Vector2::new(
        text.x + origin_x,
        crate::ui::scroll::screen_y(text.y, scroll),
    );
    if text.outline {
        return draw_outlined_text(
            object,
            Some(font),
            pos,
            &text.text,
            text.align,
            text.size,
            text.color,
        );
    }
    if text.shadow {
        return draw_shadowed_text(
            object,
            Some(font),
            pos,
            &text.text,
            text.align,
            text.size,
            text.color,
        );
    }
    usize::from(!object.draw_string(
        font,
        pos,
        &text.text,
        text.align,
        text.size,
        Color::from_rgba(text.color[0], text.color[1], text.color[2], text.color[3]),
    ))
}

/// The bottom cap reuses the sprite unflipped: a flipped draw would need
/// a negative src_rect and a second asset for a 20px cap.
pub(crate) fn draw_scrollbar(
    object: &Object,
    sprites: &crate::ui::theme::ScrollbarSprites,
    geom: &crate::ui::scroll::ScrollbarGeom,
    origin_x: f32,
) -> usize {
    let teal = crate::ui::theme::SCROLL_TRACK_MODULATE;
    let teal = Color::from_rgba(teal[0], teal[1], teal[2], teal[3]);
    let white = Color::from_rgba(1.0, 1.0, 1.0, 1.0);
    let offset = Vector2::new(origin_x, 0.0);
    let at = |r: Rect2| Rect2::new(r.position + offset, r.size);
    let mut errors = 0;
    for (texture, rect) in [
        (sprites.center, geom.body),
        (sprites.edge, geom.cap_top),
        (sprites.edge, geom.cap_bottom),
    ] {
        errors += usize::from(!object.draw_texture_rect(texture, at(rect), false, teal));
    }
    errors += usize::from(!object.draw_texture_rect(sprites.train, at(geom.grabber), false, white));
    errors
}

// The engine's `clip_contents` scissors at the Control's rect, but the
// plate's visible viewport is inset from that rect (content pads, the
// plate art's transparent tail, the drop shadow). Scrolled rows straddling
// those bands read as glyphs outside the plate on the dimmer. One canvas
// item has one clip rect, and draws are only legal in the item's own
// `_draw`, so a Mask/Content split would need a third extension class:
// the replay scissors the body to the band instead. Rects clip exactly;
// text and icons draw only fully inside, so a straddling row pops out
// whole rather than bleeding mid-glyph. The engine clip stays as the
// outer net; the tooltip never passes through the replay.

/// The body band's y-span in box-local coordinates — the scroll viewport:
/// the same span the scrollbar's track uses.
pub(crate) fn body_band(box_height: f32, plate: bool, header_bottom: f32) -> (f32, f32) {
    let pad_bottom = if plate {
        crate::ui::theme::PLATE_PAD_BOTTOM
    } else {
        crate::ui::theme::FLAT_PAD
    };
    let top = header_bottom.max(0.0);
    (top, (box_height - pad_bottom).max(top))
}

pub(crate) fn clip_y_to_band(y: f32, h: f32, band: (f32, f32)) -> Option<(f32, f32)> {
    let bottom = y + h;
    let y0 = y.max(band.0);
    let y1 = bottom.min(band.1);
    if y1 <= y0 {
        return None;
    }
    if y0 == y && y1 == bottom {
        Some((y, h))
    } else {
        Some((y0, y1 - y0))
    }
}

// Kreon's vertical metrics per 1024 upm: ascent 997, descent 293. The
// fallback font differs slightly; the cull is a scissor, so slack is
// cosmetic.
const FONT_ASCENT_PER_EM: f32 = 997.0 / 1024.0;
const FONT_DESCENT_PER_EM: f32 = 293.0 / 1024.0;
/// The text effects' overshoot past the glyph band: the outline rim's 1px
/// diagonal passes above, the header shadow's (5,4) offset below.
const TEXT_RIM_UP: f32 = 1.0;
const TEXT_SHADOW_DOWN: f32 = 4.0;

/// Text is never clipped mid-glyph: a straddling command does not draw.
pub(crate) fn text_inside_band(baseline: f32, size: i32, band: (f32, f32)) -> bool {
    let s = f32::max(size as f32, 1.0);
    let top = baseline - s * FONT_ASCENT_PER_EM - TEXT_RIM_UP;
    let bottom = baseline + s * FONT_DESCENT_PER_EM + TEXT_SHADOW_DOWN;
    top >= band.0 && bottom <= band.1
}

/// Replays one command list, returning the engine-call failure count.
#[allow(clippy::too_many_arguments)] // a draw call's full parameter list
pub(crate) fn replay_cmds(
    object: &Object,
    fonts: &Fonts,
    cmds: &[Cmd],
    origin_x: f32,
    scroll: f32,
    icons: &IconTextures,
    band: (f32, f32),
) -> usize {
    let mut call_errors = 0usize;
    for cmd in cmds {
        match cmd {
            Cmd::Rect(rect) => {
                let y = crate::ui::scroll::screen_y(rect.y, scroll);
                let Some((y, h)) = clip_y_to_band(y, rect.h, band) else {
                    continue;
                };
                if !object.draw_rect(
                    Rect2::new(Vector2::new(rect.x + origin_x, y), Vector2::new(rect.w, h)),
                    Color::from_rgba(rect.color[0], rect.color[1], rect.color[2], rect.color[3]),
                ) {
                    call_errors += 1;
                }
            }
            Cmd::Texture(tex) => {
                let rect = tex.scaled_rect(icons.scale(tex.icon));
                let y = crate::ui::scroll::screen_y(rect.position.y, scroll);
                // Icons never clip mid-sprite: they pop out whole.
                if clip_y_to_band(y, rect.size.y, band)
                    .is_none_or(|(y0, h)| y0 != y || h != rect.size.y)
                {
                    continue;
                }
                let Some(texture) = icons.resolve(tex.icon) else {
                    continue;
                };
                let pos = Vector2::new(rect.position.x + origin_x, y);
                let modulate = icons.modulate(tex.icon);
                if !object.draw_texture_rect(texture, Rect2::new(pos, rect.size), false, modulate) {
                    call_errors += 1;
                }
            }
            Cmd::Text(text) => {
                let baseline = crate::ui::scroll::screen_y(text.y, scroll);
                if !text_inside_band(baseline, text.size, band) {
                    continue;
                }
                call_errors += replay_text_cmd(object, fonts, text, origin_x, scroll);
            }
        }
    }
    call_errors
}

/// Body first, then the pinned header over it; the flat fallback repaints
/// the header zone over any body pixels that straddled upward.
#[allow(clippy::too_many_arguments)] // one frame's full replay context; bundling it further is artificial
pub(crate) fn replay_split(
    object: &Object,
    fonts: &Fonts,
    header: &[Cmd],
    body: &[Cmd],
    header_bottom: f32,
    width: f32,
    plate: bool,
    origin_x: f32,
    scroll: f32,
    box_height: f32,
    icons: &IconTextures,
) -> usize {
    let band = body_band(box_height, plate, header_bottom);
    let mut errors = replay_cmds(object, fonts, body, origin_x, scroll, icons, band);
    if !plate {
        errors += draw_header_fill(object, origin_x, width, header_bottom);
    }
    errors += replay_cmds(
        object,
        fonts,
        header,
        origin_x,
        0.0,
        icons,
        (0.0, box_height),
    );
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_character_icons_use_the_animation_scales() {
        let theme = Theme::new();
        let icons = IconTextures {
            theme: &theme,
            portraits: &[
                "res://images/ui/top_panel/character_icon_ironclad.png".to_owned(),
                "res://images/ui/top_panel/character_icon_silent.png".to_owned(),
            ],
            dimmed: &[true, false],
            scales: &[1.1, 0.95],
        };

        assert_eq!(icons.scale(IconId::Character(0)), 1.1);
        assert_eq!(icons.scale(IconId::Character(1)), 0.95);
        assert_eq!(icons.scale(IconId::TabPlate), 1.0);
        assert_eq!(icons.scale(IconId::TabStroke), 1.0);
        assert_eq!(
            icons.modulate(IconId::Character(0)).as_array(),
            crate::ui::theme::AVATAR_DIM_MODULATE,
        );
        assert_eq!(
            icons.modulate(IconId::Character(1)).as_array(),
            [1.0, 1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn kreon_covers_mirrors_the_measured_cmaps() {
        assert!(kreon_covers("Vs. LAGAVULIN"));
        assert!(kreon_covers("DPS — · 6 turns · 3 plays · took 27"));
        assert!(kreon_covers("#125 CONSTRUCT_MENAGERIE_NORMAL · completed"));
        // The game's custom "⋯" (U+22EF) is not in Kreon's cmap; "…" is.
        assert!(kreon_covers(crate::ui::chart_layout::TRUNCATION_MARK));
        for c in [
            '·', '—', '–', '×', '…', '‘', '’', '“', '”', '€', '™', 'é', 'ü', 'ł', 'ą', '½',
        ] {
            assert!(
                kreon_covers(&c.to_string()),
                "{c:?} is in the measured cmap"
            );
        }
        for c in ['中', 'あ', '한', 'ก', 'Ж', 'я'] {
            assert!(!kreon_covers(&c.to_string()), "{c:?} needs the fallback");
        }
        assert!(!kreon_covers("STRIKE · 中"));
        assert!(kreon_covers("line one\nline two"));
        assert!(kreon_covers(""));
    }

    #[test]
    fn kreon_covered_ranges_are_sorted_and_disjoint() {
        for pair in KREON_COVERED.windows(2) {
            assert!(pair[0].0 <= pair[0].1, "range ordered: {:?}", pair[0]);
            assert!(
                pair[0].1 < pair[1].0,
                "ranges sorted and disjoint: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn body_band_spans_from_the_header_to_the_bottom_pad() {
        assert_eq!(
            body_band(400.0, true, 154.0),
            (154.0, 400.0 - crate::ui::theme::PLATE_PAD_BOTTOM)
        );
        assert_eq!(
            body_band(400.0, false, 150.0),
            (150.0, 400.0 - crate::ui::theme::FLAT_PAD)
        );
        // A degenerate box collapses to an empty band, never inverted.
        assert_eq!(body_band(100.0, true, 154.0), (154.0, 154.0));
    }

    #[test]
    fn clip_y_to_band_clips_or_drops() {
        let band = (16.0, 380.0);
        assert_eq!(clip_y_to_band(100.0, 32.0, band), Some((100.0, 32.0)));
        assert_eq!(clip_y_to_band(-10.0, 40.0, band), Some((16.0, 14.0)));
        assert_eq!(clip_y_to_band(370.0, 40.0, band), Some((370.0, 10.0)));
        assert_eq!(clip_y_to_band(-50.0, 30.0, band), None);
        assert_eq!(clip_y_to_band(400.0, 30.0, band), None);
        assert_eq!(clip_y_to_band(0.0, 500.0, band), Some((16.0, 364.0)));
    }

    #[test]
    fn an_unclipped_fractional_icon_keeps_its_exact_height() {
        let rect = crate::ui::chart_layout::TextureCmd {
            x: 22.0,
            y: 158.0,
            w: 64.0,
            h: 64.0,
            icon: IconId::Character(0),
        }
        .scaled_rect(0.95);

        assert_eq!(
            clip_y_to_band(rect.position.y, rect.size.y, (0.0, 300.0)),
            Some((rect.position.y, rect.size.y))
        );
    }

    #[test]
    fn text_inside_band_requires_the_full_glyph_band() {
        let band = (16.0, 380.0);
        // A 24px row's glyph band spans baseline −24.4 .. +10.9 (ascent
        // 23.4 + 1 rim, descent 6.9 + 4 shadow).
        assert!(text_inside_band(60.0, 24, band));
        assert!(!text_inside_band(30.0, 24, band), "ascent crosses the top");
        assert!(
            !text_inside_band(375.0, 24, band),
            "shadow crosses the bottom"
        );
        assert!(text_inside_band(60.0, 32, band));
        assert!(!text_inside_band(40.0, 32, band));
        let ascent = 24.0 * FONT_ASCENT_PER_EM + TEXT_RIM_UP;
        let descent = 24.0 * FONT_DESCENT_PER_EM + TEXT_SHADOW_DOWN;
        assert!(text_inside_band(band.0 + ascent, 24, band));
        assert!(text_inside_band(band.1 - descent, 24, band));
        assert!(!text_inside_band(band.0 + ascent - 0.5, 24, band));
    }
}
