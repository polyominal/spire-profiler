//! The floating hover tooltip — the game's `NHoverTipSet` idiom, drawn by
//! each panel's overlay child (instant on hover, no fade). The tip and the
//! legend float OUTSIDE the plate box, so the parent Control widens sideways
//! to contain both ([`frame`]); the tip never extends it vertically — its
//! line count is capped and its y clamps into the plate's band ([`place`]).
//!
//! The payload is structured ([`RowDetail`]): a gold 22px title, then one
//! two-column stat line per group — label left, value right-aligned in the
//! chart segment's color ([`tone_color`], the same call the bars make), so
//! the tip can never drift from the bars. Engine-created fonts are never
//! dispatch targets, so `get_string_size` is unavailable and the columns
//! run static CHAR budgets derived from the shipped faces' measured 22px
//! advances.

use crate::engine::gdext::{Object, RetainedVariant};
use crate::engine::math::{Color, Rect2, Vector2};
use crate::engine::object::TextAlign;
use crate::source_kind::SourceKind;
use crate::ui::palette;
use crate::ui::panel_replay::{self, Fonts};
use crate::ui::theme::{self, TextRole};
use crate::ui::ui_model::{Section, Segment};

/// The game's hover-tip width verbatim (`_hoverTipWidth = 360f`).
pub(crate) const TIP_WIDTH: f32 = 360.0;

/// A centered modal's left edge sits near 34% of the viewport, so the
/// game's left-edge test would never flip; the panel tests the right edge.
const FLIP_THRESHOLD: f32 = 0.75;

/// Matches the plate's shadow offset.
const TIP_PAD: f32 = 8.0;

/// The game's tightened tooltip leading (`line_separation = -2`).
const TIP_LINE_H: f32 = 26.0;

/// The last line's descent can eat ~3px of the bottom margin: margins are
/// air, never the border art.
const TEXT_BASELINE: f32 = 22.0;

/// The scene's text margins plus the 8px shadow inset.
const TIP_TEXT_V_PAD: f32 =
    theme::PLATE_PAD_TOP + theme::PLATE_PAD_BOTTOM + theme::PLATE_SHADOW_OFFSET;

/// 18 × 15.6px = 280px ≤ 293px at the measured uppercase-slug advance.
const TITLE_BUDGET: usize = 18;

/// Mixed label text measures ~9.4px/char; 18 × 9.4px ≈ 170px.
const LABEL_BUDGET: usize = 18;

/// Worst case all digits: 10 × 12.1px = 121px ≤ 123px.
const VALUE_BUDGET: usize = 10;

/// A stat that escapes its column wraps whole: 24 × 12.1px = 290px ≤ 293px.
const BODY_BUDGET: usize = 24;

/// The 293px text column splits into the label column and the value box.
const LABEL_COL_W: f32 = 170.0;

/// [`StatTone::Direct`] carries (section, kind) because the direct slot is
/// section-tinted; [`StatTone::Neutral`] is cream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatTone {
    Neutral,
    Direct(Section, SourceKind),
    Attributed,
    Modifier,
    MitigateDebuff,
    MitigateBuff,
    MitigateStr,
    SelfDamage,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StatLine {
    pub label: String,
    pub value: String,
    pub tone: StatTone,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RowDetail {
    pub title: String,
    pub stats: Vec<StatLine>,
}

impl RowDetail {
    /// No hover, or an unresolved row: the tip stays hidden.
    pub(crate) fn is_empty(&self) -> bool {
        self.title.is_empty() && self.stats.is_empty()
    }

    /// Scanned by the panels' glyph-coverage fallback.
    pub(crate) fn texts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str()).chain(
            self.stats
                .iter()
                .flat_map(|stat| [stat.label.as_str(), stat.value.as_str()]),
        )
    }
}

/// The same call the bars make.
fn tone_color(tone: StatTone) -> palette::Color {
    let (slot, section, kind) = match tone {
        StatTone::Neutral => return palette::COL_CREAM,
        StatTone::Direct(section, kind) => (Segment::Direct, section, kind),
        StatTone::Attributed => (Segment::Attributed, Section::Damage, SourceKind::Card),
        StatTone::Modifier => (Segment::Modifier, Section::Damage, SourceKind::Card),
        StatTone::MitigateDebuff => (Segment::MitigateDebuff, Section::Defense, SourceKind::Card),
        StatTone::MitigateBuff => (Segment::MitigateBuff, Section::Defense, SourceKind::Card),
        StatTone::MitigateStr => (Segment::MitigateStr, Section::Defense, SourceKind::Card),
        StatTone::SelfDamage => (Segment::SelfDamage, Section::Defense, SourceKind::Card),
    };
    palette::slot_color(slot, section, kind)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TipValue {
    pub text: String,
    pub color: palette::Color,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TipLine {
    pub text: String,
    pub title: bool,
    pub color: palette::Color,
    pub value: Option<TipValue>,
}

/// The title, then one two-column line per stat; escaping stats wrap, and
/// `max_lines` drops the tail with the truncation marker.
pub(crate) fn shape(detail: &RowDetail, max_lines: usize) -> Vec<TipLine> {
    let mut out: Vec<TipLine> = Vec::new();
    wrap(
        detail.title.trim(),
        TITLE_BUDGET,
        true,
        palette::COL_GOLD,
        &mut out,
    );
    for stat in &detail.stats {
        let color = tone_color(stat.tone);
        if stat.label.chars().count() <= LABEL_BUDGET && stat.value.chars().count() <= VALUE_BUDGET
        {
            out.push(TipLine {
                text: stat.label.clone(),
                title: false,
                color: palette::COL_CREAM,
                value: Some(TipValue {
                    text: stat.value.clone(),
                    color,
                }),
            });
        } else {
            wrap(
                &format!("{} {}", stat.label, stat.value),
                BODY_BUDGET,
                false,
                color,
                &mut out,
            );
        }
    }
    if out.len() > max_lines {
        out.truncate(max_lines);
        if let Some(last) = out.last_mut() {
            *last = TipLine {
                text: crate::ui::chart_layout::TRUNCATION_MARK.to_owned(),
                title: false,
                color: palette::COL_DIM,
                value: None,
            };
        }
    }
    out
}

/// Greedy word-wrap at the char budget; overlong words hard-break.
fn wrap(text: &str, budget: usize, title: bool, color: palette::Color, out: &mut Vec<TipLine>) {
    debug_assert!(budget >= 4, "the budget holds at least one char");
    let mut line = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let mut rest = word;
        while !rest.is_empty() {
            let line_chars = line.chars().count();
            let sep = usize::from(!line.is_empty());
            if line_chars + sep + rest.chars().count() <= budget {
                if sep == 1 {
                    line.push(' ');
                }
                line.push_str(rest);
                break;
            }
            if !line.is_empty() && rest.chars().count() <= budget {
                // A short word wraps whole onto the next line.
                flush(&mut line, title, color, out);
                continue;
            }
            // The word alone exceeds the budget: hard-break it.
            let room = budget.saturating_sub(line_chars + sep);
            if !line.is_empty() {
                // Fill the current line first when it has room for a
                // piece (the break is byte-based at a char boundary, so
                // a multibyte char that fits no room breaks nothing).
                if room > 0 {
                    let cut = rest.floor_char_boundary(room.min(rest.len()));
                    if cut > 0 {
                        line.push(' ');
                        line.push_str(&rest[..cut]);
                        rest = &rest[cut..];
                    }
                }
                flush(&mut line, title, color, out);
                continue;
            }
            let cut = rest.floor_char_boundary(budget.min(rest.len()));
            debug_assert!(cut > 0, "the budget holds at least one char");
            line.push_str(&rest[..cut]);
            rest = &rest[cut..];
            flush(&mut line, title, color, out);
        }
    }
    flush(&mut line, title, color, out);
}

fn flush(line: &mut String, title: bool, color: palette::Color, out: &mut Vec<TipLine>) {
    if !line.is_empty() {
        out.push(TipLine {
            text: std::mem::take(line),
            title,
            color,
            value: None,
        });
    }
}

pub(crate) fn tip_height(lines: usize) -> f32 {
    lines as f32 * TIP_LINE_H + TIP_TEXT_V_PAD
}

/// Floored at 1 so the title always renders; the cap keeps the tip inside
/// the plate's y-band.
pub(crate) fn max_tip_lines(box_h: f32) -> usize {
    (((box_h - 2.0 * TIP_PAD - TIP_TEXT_V_PAD) / TIP_LINE_H)
        .floor()
        .max(1.0)) as usize
}

/// Both plates run it on the same box, so they always share a side.
fn side_x(viewport: Vector2, plate: Rect2, width: f32) -> f32 {
    let plate_right = plate.position.x + plate.size.x;
    let x = if plate_right > viewport.x * FLIP_THRESHOLD {
        plate.position.x - width
    } else {
        plate_right
    };
    x.clamp(0.0, (viewport.x - width).max(0.0))
}

/// The game's 5px tip-stacking spacing.
const STACK_GAP: f32 = 5.0;

/// While the legend shows, the tip stacks under it; a stack that cannot
/// fit falls back to row-anchored placement over the legend.
pub(crate) fn place(
    viewport: Vector2,
    plate: Rect2,
    row_y: f32,
    tip_size: Vector2,
    legend: Option<Rect2>,
) -> Rect2 {
    let x = side_x(viewport, plate, tip_size.x);
    let lo = plate.position.y + TIP_PAD;
    let hi = (plate.position.y + plate.size.y - tip_size.y - TIP_PAD).max(lo);
    let y = match legend {
        Some(legend) if legend.position.y + legend.size.y + STACK_GAP <= hi => row_y
            .max(legend.position.y + legend.size.y + STACK_GAP)
            .clamp(lo, hi),
        _ => row_y.clamp(lo, hi),
    };
    Rect2::new(Vector2::new(x, y), tip_size)
}

/// Shared side x, y at the main plate's top edge, never scrolling;
/// mouse-transparent.
pub(crate) fn place_legend(viewport: Vector2, plate: Rect2, size: Vector2) -> Rect2 {
    Rect2::new(
        Vector2::new(side_x(viewport, plate, size.x), plate.position.y),
        size,
    )
}

/// The union never moves the y origin off the plate's top (the strip
/// shift and the tooltip's row anchor rely on it); only a tall legend
/// extends the bottom — the tip never does.
pub(crate) fn frame(plate: Rect2, legend: Option<Rect2>, tip: Option<Rect2>) -> (Rect2, f32) {
    if legend.is_none() && tip.is_none() {
        return (plate, 0.0);
    }
    if let Some(tip) = tip {
        debug_assert!(
            tip.position.y >= plate.position.y
                && tip.position.y + tip.size.y <= plate.position.y + plate.size.y,
            "the tip stays in the plate's y-band (`place` clamps it there), \
             so the frame's bottom is the plate's own unless the legend is taller"
        );
    }
    if let Some(legend) = legend {
        debug_assert!(
            legend.position.y >= plate.position.y,
            "the legend's top IS the plate's top edge (`place_legend`), \
             so the Control keeps the plate's y origin"
        );
    }
    let mut x0 = plate.position.x;
    let mut x1 = plate.position.x + plate.size.x;
    let mut y1 = plate.position.y + plate.size.y;
    for side in [legend, tip].into_iter().flatten() {
        x0 = x0.min(side.position.x);
        x1 = x1.max(side.position.x + side.size.x);
        y1 = y1.max(side.position.y + side.size.y);
    }
    (
        Rect2::new(
            Vector2::new(x0, plate.position.y),
            Vector2::new(x1 - x0, y1 - plate.position.y),
        ),
        plate.position.x - x0,
    )
}

/// A failed plate asset falls back to flat-chrome rects.
pub(crate) fn draw(
    object: &Object,
    fonts: &Fonts,
    plate: Option<&theme::Plate>,
    lines: &[TipLine],
    rect: Rect2,
) -> usize {
    let mut errors = 0;
    match plate {
        Some(plate) => {
            let (shadow, body) = theme::plate_rects(rect.size);
            let at = |r: Rect2| Rect2::new(r.position + rect.position, r.size);
            errors += usize::from(!object.draw_style_box(&plate.shadow, at(shadow)));
            errors += usize::from(!object.draw_style_box(&plate.body, at(body)));
        }
        None => {
            errors += panel_replay::draw_flat_chrome(object, rect);
        }
    }
    let title_font = fonts.for_role(TextRole::Title);
    let body_font = fonts.for_role(TextRole::Body);
    let text_x = rect.position.x + theme::PLATE_PAD_LEFT;
    let value_x = text_x + LABEL_COL_W;
    let value_w = (TIP_WIDTH
        - theme::PLATE_SHADOW_OFFSET
        - theme::PLATE_PAD_RIGHT
        - theme::PLATE_PAD_LEFT
        - LABEL_COL_W)
        .max(0.0);
    let mut baseline = rect.position.y + theme::PLATE_PAD_TOP + TEXT_BASELINE;
    for line in lines {
        let font = if line.title { title_font } else { body_font };
        if let Some(font) = font {
            errors += draw_text(
                object,
                font,
                Vector2::new(text_x, baseline),
                TextAlign::Left,
                &line.text,
                line.color,
            );
            if let Some(value) = &line.value {
                errors += draw_text(
                    object,
                    font,
                    Vector2::new(value_x, baseline),
                    TextAlign::Right(value_w),
                    &value.text,
                    value.color,
                );
            }
        }
        baseline += TIP_LINE_H;
    }
    errors
}

/// The game's (3,2) quarter-black shadow under the main pass.
fn draw_text(
    object: &Object,
    font: &RetainedVariant,
    pos: Vector2,
    align: TextAlign,
    text: &str,
    color: palette::Color,
) -> usize {
    let shadow = palette::COL_TIP_SHADOW;
    let mut errors = 0;
    if !object.draw_string(
        font,
        pos + Vector2::new(3.0, 2.0),
        text,
        align,
        theme::SIZE_TOOLTIP,
        Color::from_rgba(shadow[0], shadow[1], shadow[2], shadow[3]),
    ) {
        errors += 1;
    }
    if !object.draw_string(
        font,
        pos,
        text,
        align,
        theme::SIZE_TOOLTIP,
        Color::from_rgba(color[0], color[1], color[2], color[3]),
    ) {
        errors += 1;
    }
    errors
}

#[cfg(test)]
mod tests;
