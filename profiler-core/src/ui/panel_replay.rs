//! The panels' draw backend — the `_draw`-path plumbing shared by the two
//! panels: the font set with the panel-wide glyph-coverage
//! fallback, the text-effect passes, the chrome paints, and the draw-
//! command replay itself. Everything rides the FFI through the safe
//! [`Object`] newtype.

use crate::engine::gdext::{Object, RetainedVariant};
use crate::engine::math::{Color, Rect2, Vector2};
use crate::engine::object::TextAlign;
use crate::ui::chart_layout::{Cmd, TextCmd};
use crate::ui::palette;
use crate::ui::theme::{AssetState, IconId, Plate, ScrollbarSprites, TextRole, Theme};
use crate::warn;

/// A failed fetch disables text permanently.
pub(crate) fn ensure_font(object: &Object, state: &mut AssetState, warning: &str) -> bool {
    match state {
        AssetState::Loaded(_) => true,
        AssetState::Failed => false,
        AssetState::Unfetched => match object.get_theme_default_font() {
            Some(resolved) => {
                *state = AssetState::Loaded(resolved);
                true
            }
            None => {
                *state = AssetState::Failed;
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
///
/// The coverage scan runs once per layout rebuild ([`FontPlan::scan`]), so
/// the chrome and body draws each resolve fonts without re-scanning.
#[derive(Clone, Copy, Default)]
pub(crate) enum FontPlan {
    #[default]
    NoText,
    Kreon,
    Fallback,
}

impl FontPlan {
    pub(crate) fn scan(
        header: &[Cmd],
        body: &[Cmd],
        detail: &crate::ui::tooltip::RowDetail,
    ) -> FontPlan {
        let mut plan = if detail.is_empty() {
            FontPlan::NoText
        } else {
            FontPlan::Kreon
        };
        for text in detail.texts() {
            if !kreon_covers(text) {
                plan = FontPlan::Fallback;
            }
        }
        for cmd in header.iter().chain(body.iter()) {
            if let Cmd::Text(text) = cmd {
                if matches!(plan, FontPlan::NoText) {
                    plan = FontPlan::Kreon;
                }
                if !kreon_covers(&text.text) {
                    plan = FontPlan::Fallback;
                }
            }
        }
        plan
    }
}

pub(crate) struct Fonts<'a> {
    default: Option<&'a RetainedVariant>,
    title: Option<&'a RetainedVariant>,
    body: Option<&'a RetainedVariant>,
    force_default: bool,
}

impl<'a> Fonts<'a> {
    pub(crate) fn new(
        object: &Object,
        default_state: &'a mut AssetState,
        theme: &'a Theme,
        plan: FontPlan,
        warn: &str,
    ) -> Fonts<'a> {
        if !matches!(plan, FontPlan::NoText) {
            ensure_font(object, default_state, warn);
        }
        let default = match &*default_state {
            AssetState::Loaded(font) => Some(font),
            _ => None,
        };
        Fonts {
            default,
            title: theme.face(TextRole::Title),
            body: theme.face(TextRole::Body),
            force_default: matches!(plan, FontPlan::Fallback),
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
    font: &RetainedVariant,
    pos: Vector2,
    text: &str,
    align: TextAlign,
    size: i32,
    color: palette::Color,
) -> usize {
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
    font: &RetainedVariant,
    pos: Vector2,
    text: &str,
    align: TextAlign,
    size: i32,
    color: palette::Color,
) -> usize {
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
                errors += replay_text_cmd(object, fonts, t, Vector2::ZERO);
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

fn replay_text_cmd(object: &Object, fonts: &Fonts, text: &TextCmd, offset: Vector2) -> usize {
    let Some(font) = fonts.for_role(text.role) else {
        return 0;
    };
    let pos = Vector2::new(text.x + offset.x, text.y + offset.y);
    if text.outline {
        return draw_outlined_text(
            object, font, pos, &text.text, text.align, text.size, text.color,
        );
    }
    if text.shadow {
        return draw_shadowed_text(
            object, font, pos, &text.text, text.align, text.size, text.color,
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

// The body child's rect, scrollbar track, and hover gate all consume this
// one band, so their boundaries cannot drift.

/// The body band's y-span in box-local coordinates — the scroll viewport:
/// the same span the scrollbar's track uses.
pub(crate) fn body_band(box_height: f32, plate: bool, header_bottom: f32) -> (f32, f32) {
    let pad_bottom = if plate {
        crate::ui::theme::PLATE_OUTER_PAD_BOTTOM
    } else {
        crate::ui::theme::FLAT_PAD
    };
    let top = header_bottom.max(0.0);
    (top, (box_height - pad_bottom).max(top))
}

/// Replays one command list translated by `offset`, returning the
/// engine-call failure count.
pub(crate) fn replay_cmds(
    object: &Object,
    fonts: &Fonts,
    cmds: &[Cmd],
    offset: Vector2,
    icons: &IconTextures,
) -> usize {
    let mut call_errors = 0usize;
    for cmd in cmds {
        match cmd {
            Cmd::Rect(rect) => {
                if !object.draw_rect(
                    Rect2::new(
                        Vector2::new(rect.x + offset.x, rect.y + offset.y),
                        Vector2::new(rect.w, rect.h),
                    ),
                    Color::from_rgba(rect.color[0], rect.color[1], rect.color[2], rect.color[3]),
                ) {
                    call_errors += 1;
                }
            }
            Cmd::Texture(tex) => {
                let rect = tex.scaled_rect(icons.scale(tex.icon));
                let Some(texture) = icons.resolve(tex.icon) else {
                    continue;
                };
                let pos = rect.position + offset;
                let modulate = icons.modulate(tex.icon);
                if !object.draw_texture_rect(texture, Rect2::new(pos, rect.size), false, modulate) {
                    call_errors += 1;
                }
            }
            Cmd::Text(text) => {
                call_errors += replay_text_cmd(object, fonts, text, offset);
            }
        }
    }
    call_errors
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
            (154.0, 400.0 - crate::ui::theme::PLATE_OUTER_PAD_BOTTOM)
        );
        assert_eq!(
            body_band(400.0, false, 150.0),
            (150.0, 400.0 - crate::ui::theme::FLAT_PAD)
        );
        // A degenerate box collapses to an empty band, never inverted.
        assert_eq!(body_band(100.0, true, 154.0), (154.0, 154.0));
    }
}
