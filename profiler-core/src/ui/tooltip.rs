//! The floating hover tooltip — the game's `NHoverTipSet` idiom, drawn by
//! the panels in `_draw` (instant on hover, no fade). The tip and the
//! legend float OUTSIDE the plate box, but `clip_contents` scissors the
//! Control's own drawing, so the Control widens sideways to contain both
//! ([`frame`]) and the tip never extends it vertically — its line count is
//! capped and its y clamps into the plate's band ([`place`]).
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
use crate::ui::palette;
use crate::ui::panel_replay::{self, Fonts};
use crate::ui::theme::{self, TextRole};
use crate::ui::ui_model::{self, Section};

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

/// `Direct` carries (section, kind) because the direct slot is
/// section-tinted; `Neutral` is cream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatTone {
    Neutral,
    Direct(Section, u8),
    Attributed,
    Modifier,
    Upgrade,
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
        StatTone::Direct(section, kind) => (ui_model::SEG_DIRECT, section, kind),
        StatTone::Attributed => (ui_model::SEG_ATTRIBUTED, Section::Damage, 0),
        StatTone::Modifier => (ui_model::SEG_MODIFIER, Section::Damage, 0),
        StatTone::Upgrade => (ui_model::SEG_UPGRADE, Section::Damage, 0),
        StatTone::MitigateDebuff => (ui_model::SEG_MITIGATE_DEBUFF, Section::Defense, 0),
        StatTone::MitigateBuff => (ui_model::SEG_MITIGATE_BUFF, Section::Defense, 0),
        StatTone::MitigateStr => (ui_model::SEG_MITIGATE_STR, Section::Defense, 0),
        StatTone::SelfDamage => (ui_model::SEG_SELF, Section::Defense, 0),
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

/// The union is horizontal-only for the tip by contract: a vertical
/// extension would open clip corners scrolled content could spill into.
pub(crate) fn frame(plate: Rect2, legend: Option<Rect2>, tip: Option<Rect2>) -> (Rect2, f32) {
    if legend.is_none() && tip.is_none() {
        return (plate, 0.0);
    }
    if let Some(tip) = tip {
        debug_assert!(
            tip.position.y >= plate.position.y
                && tip.position.y + tip.size.y <= plate.position.y + plate.size.y,
            "the tip stays in the plate's y-band (`place` clamps it there); \
             a vertical union would leak scrolled content into the clip corners"
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
mod tests {
    use super::*;

    fn chars(s: &str) -> usize {
        s.chars().count()
    }

    fn card_detail() -> RowDetail {
        use StatTone::*;
        let stat = |label: &str, value: &str, tone: StatTone| StatLine {
            label: label.to_owned(),
            value: value.to_owned(),
            tone,
        };
        RowDetail {
            title: "\"STRIKE\" x12".to_owned(),
            stats: vec![
                stat("dmg (12345 unblk)", "12345", Direct(Section::Damage, 0)),
                stat("direct", "12345", Direct(Section::Damage, 0)),
                stat("indirect", "12345", Attributed),
                stat("mod", "0", Modifier),
                stat("upg", "0", Upgrade),
                stat("block (100 eff)", "120", Direct(Section::Defense, 0)),
                stat("blk mod", "10", Modifier),
                stat("blk upg", "10", Upgrade),
                stat("weak", "30", MitigateDebuff),
                stat("buff", "0", MitigateBuff),
                stat("str", "5", MitigateStr),
                stat("self dmg", "12", SelfDamage),
                stat("forge", "2", Neutral),
            ],
        }
    }

    /// Every tone equals the bar's color, so a palette rework can never
    /// leave the tip stale.
    #[test]
    fn tone_colors_match_the_chart_segments() {
        use StatTone::*;
        use ui_model as m;
        for (tone, slot, section, kind) in [
            (
                Direct(m::Section::Damage, 0),
                m::SEG_DIRECT,
                m::Section::Damage,
                0,
            ),
            (
                Direct(m::Section::Damage, 1),
                m::SEG_DIRECT,
                m::Section::Damage,
                1,
            ),
            (
                Direct(m::Section::Defense, 0),
                m::SEG_DIRECT,
                m::Section::Defense,
                0,
            ),
            (
                Direct(m::Section::Defense, 4),
                m::SEG_DIRECT,
                m::Section::Defense,
                4,
            ),
            (Attributed, m::SEG_ATTRIBUTED, m::Section::Damage, 0),
            (Modifier, m::SEG_MODIFIER, m::Section::Damage, 0),
            (Upgrade, m::SEG_UPGRADE, m::Section::Damage, 0),
            (
                MitigateDebuff,
                m::SEG_MITIGATE_DEBUFF,
                m::Section::Defense,
                0,
            ),
            (MitigateBuff, m::SEG_MITIGATE_BUFF, m::Section::Defense, 0),
            (MitigateStr, m::SEG_MITIGATE_STR, m::Section::Defense, 0),
            (SelfDamage, m::SEG_SELF, m::Section::Defense, 0),
        ] {
            assert_eq!(
                tone_color(tone),
                palette::slot_color(slot, section, kind),
                "{tone:?} must equal the bar's color"
            );
        }
        // The Osty-absorb exception flows through.
        assert_eq!(
            tone_color(StatTone::Direct(Section::Defense, 4)),
            palette::COL_OSTY
        );
        assert_eq!(tone_color(StatTone::Neutral), palette::COL_CREAM);
    }

    #[test]
    fn shape_emits_one_two_column_line_per_stat() {
        let detail = card_detail();
        let lines = shape(&detail, 64);
        assert_eq!(lines.len(), 1 + detail.stats.len());
        assert!(lines[0].title);
        assert_eq!(lines[0].color, palette::COL_GOLD);
        assert_eq!(lines[0].text, "\"STRIKE\" x12");
        for (line, stat) in lines[1..].iter().zip(detail.stats.iter()) {
            assert!(!line.title);
            assert_eq!(line.text, stat.label, "the label is the left column");
            assert_eq!(
                line.color,
                palette::COL_CREAM,
                "labels are cream; the value carries the semantic color"
            );
            let value = line.value.as_ref().expect("a two-column stat line");
            assert_eq!(value.text, stat.value);
            assert_eq!(value.color, tone_color(stat.tone));
            assert!(chars(&line.text) <= LABEL_BUDGET, "label over budget");
            assert!(chars(&value.text) <= VALUE_BUDGET, "value over budget");
        }
    }

    #[test]
    fn shape_hard_breaks_overlong_title_slugs() {
        let id = "A".repeat(64);
        let detail = RowDetail {
            title: format!("\"{id}\" x3"),
            stats: vec![StatLine {
                label: "forge".to_owned(),
                value: "1".to_owned(),
                tone: StatTone::Neutral,
            }],
        };
        let lines = shape(&detail, 64);
        assert!(lines.len() > 4, "the slug spans several lines: {lines:?}");
        for line in &lines {
            let budget = if line.title {
                TITLE_BUDGET
            } else {
                LABEL_BUDGET
            };
            assert!(chars(&line.text) <= budget, "over budget: {:?}", line.text);
        }
        let title: String = lines
            .iter()
            .filter(|l| l.title)
            .map(|l| l.text.replace(' ', ""))
            .collect();
        assert_eq!(title, format!("\"{id}\"x3"));
    }

    #[test]
    fn shape_wraps_over_budget_stats_as_single_color_lines() {
        let detail = RowDetail {
            title: "\"GREED\" x1".to_owned(),
            stats: vec![StatLine {
                label: "dmg (999999999999 unblk)".to_owned(),
                value: "999999999999".to_owned(),
                tone: StatTone::Direct(Section::Damage, 0),
            }],
        };
        let lines = shape(&detail, 64);
        assert_eq!(lines[0].text, "\"GREED\" x1");
        let wrapped = &lines[1..];
        assert!(wrapped.len() > 1, "the stat wraps: {lines:?}");
        for line in wrapped {
            assert!(line.value.is_none(), "backstop lines carry no column");
            assert_eq!(
                line.color,
                palette::slot_color(ui_model::SEG_DIRECT, Section::Damage, 0)
            );
            assert!(chars(&line.text) <= BODY_BUDGET, "over budget: {line:?}");
        }
        let joined: String = wrapped.iter().map(|l| l.text.replace(' ', "")).collect();
        assert_eq!(joined, "dmg(999999999999unblk)999999999999");
    }

    #[test]
    fn shape_truncates_to_max_lines_with_the_ellipsis_marker() {
        let lines = shape(&card_detail(), 4);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].title);
        assert_eq!(lines[3].text, crate::ui::chart_layout::TRUNCATION_MARK);
        assert!(
            panel_replay::kreon_covers(&lines[3].text),
            "the marker renders in the native faces"
        );
        assert!(!lines[3].title);
        assert!(lines[3].value.is_none());
    }

    #[test]
    fn shape_handles_the_terse_self_damage_detail() {
        let detail = RowDetail {
            title: "\"CRIMSON_MANTLE\"".to_owned(),
            stats: vec![StatLine {
                label: "self dmg".to_owned(),
                value: "3".to_owned(),
                tone: StatTone::SelfDamage,
            }],
        };
        let lines = shape(&detail, 64);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].title);
        let value = lines[1].value.as_ref().expect("two-column");
        assert_eq!(value.text, "3");
        assert_eq!(value.color, palette::COL_SELF);
    }

    #[test]
    fn row_detail_texts_cover_everything_the_tip_draws() {
        let detail = card_detail();
        let texts: Vec<&str> = detail.texts().collect();
        assert_eq!(texts.len(), 1 + 2 * detail.stats.len());
        assert_eq!(texts[0], detail.title);
        assert!(RowDetail::default().is_empty());
        assert!(!detail.is_empty());
    }

    fn plate() -> Rect2 {
        Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 400.0))
    }

    fn viewport() -> Vector2 {
        Vector2::new(1920.0, 1080.0)
    }

    fn tip_size() -> Vector2 {
        Vector2::new(TIP_WIDTH, 200.0)
    }

    #[test]
    fn place_defaults_to_the_plate_right_at_the_row_y() {
        let tip = place(viewport(), plate(), 300.0, tip_size(), None);
        assert_eq!(tip.position.x, 1260.0);
        assert_eq!(tip.position.y, 300.0);
    }

    #[test]
    fn place_flips_left_past_the_threshold() {
        // Plate right edge at 1500 > 1440: the tip anchors left.
        let plate = Rect2::new(Vector2::new(800.0, 200.0), Vector2::new(700.0, 400.0));
        let tip = place(viewport(), plate, 300.0, tip_size(), None);
        assert_eq!(tip.position.x, 800.0 - TIP_WIDTH);
        assert_eq!(tip.position.y, 300.0);
        let plate = Rect2::new(Vector2::new(700.0, 200.0), Vector2::new(700.0, 400.0));
        let tip = place(viewport(), plate, 300.0, tip_size(), None);
        assert_eq!(tip.position.x, 1400.0);
    }

    #[test]
    fn place_clamps_vertically_into_the_plate_band() {
        let top = place(viewport(), plate(), -100.0, tip_size(), None);
        assert_eq!(top.position.y, 200.0 + TIP_PAD);
        let bottom = place(viewport(), plate(), 10_000.0, tip_size(), None);
        assert_eq!(bottom.position.y, 600.0 - 200.0 - TIP_PAD);
    }

    #[test]
    fn place_floors_on_a_tiny_viewport() {
        let small = Vector2::new(300.0, 200.0);
        let plate = Rect2::new(Vector2::new(0.0, 48.0), Vector2::new(300.0, 104.0));
        let tip = place(small, plate, 60.0, Vector2::new(TIP_WIDTH, 96.0), None);
        assert_eq!(tip.position.x, 0.0);
        assert!(tip.position.y >= plate.position.y + TIP_PAD);
    }

    #[test]
    fn place_never_leaves_the_viewport() {
        let viewports = [
            Vector2::new(1920.0, 1080.0),
            Vector2::new(1280.0, 720.0),
            Vector2::new(800.0, 600.0),
            Vector2::new(400.0, 300.0),
        ];
        for v in viewports {
            for px in [0.0, v.x * 0.2, v.x * 0.5, v.x * 0.9] {
                let plate = Rect2::new(
                    Vector2::new(px, 48.0),
                    Vector2::new(600.0_f32.min(v.x), (v.y - 96.0).max(96.0)),
                );
                for row_y in [0.0, 48.0, v.y * 0.5, v.y, v.y * 2.0] {
                    let tip = place(v, plate, row_y, tip_size(), None);
                    assert!(tip.position.x >= 0.0, "x offscreen: {tip:?} in {v:?}");
                    if v.x >= TIP_WIDTH {
                        assert!(
                            tip.position.x + tip.size.x <= v.x,
                            "right edge offscreen: {tip:?} in {v:?}"
                        );
                    }
                    if tip.size.y <= plate.size.y - 2.0 * TIP_PAD {
                        assert!(tip.position.y >= plate.position.y + TIP_PAD);
                        assert!(
                            tip.position.y + tip.size.y
                                <= plate.position.y + plate.size.y - TIP_PAD
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn placed_tips_satisfy_the_frame_band_contract() {
        for row_y in [-50.0, 200.0, 400.0, 900.0] {
            let tip = place(viewport(), plate(), row_y, tip_size(), None);
            let (control, _) = frame(plate(), None, Some(tip));
            assert_eq!(control.size.y, plate().size.y);
        }
    }

    #[test]
    fn place_legend_shares_the_main_plates_top_edge() {
        let size = Vector2::new(183.0, 284.0);
        let legend = place_legend(viewport(), plate(), size);
        assert_eq!(legend.position, Vector2::new(1260.0, 200.0));
        // The plate (660..1260) sits past 75% of a 1280-wide viewport.
        let narrow = Vector2::new(1280.0, 720.0);
        let legend = place_legend(narrow, plate(), size);
        assert_eq!(legend.position, Vector2::new(660.0 - 183.0, 200.0));
        let tiny = Vector2::new(700.0, 500.0);
        let leftish = Rect2::new(Vector2::new(100.0, 48.0), Vector2::new(500.0, 300.0));
        let legend = place_legend(tiny, leftish, size);
        assert_eq!(legend.position.x, 0.0);
        assert_eq!(legend.position.y, 48.0, "the top line still shares");
        let short = Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 200.0));
        let legend = place_legend(viewport(), short, size);
        assert_eq!(legend.position.y, short.position.y);
    }

    fn tall_plate() -> Rect2 {
        Rect2::new(Vector2::new(660.0, 140.0), Vector2::new(600.0, 800.0))
    }

    fn tall_legend() -> Rect2 {
        Rect2::new(Vector2::new(1260.0, 140.0), Vector2::new(183.0, 284.0))
    }

    #[test]
    fn place_stacks_the_tip_under_the_legend() {
        let legend = tall_legend();
        let floor = 140.0 + 284.0 + STACK_GAP; // 429
        let tip = place(viewport(), tall_plate(), 148.0, tip_size(), Some(legend));
        assert_eq!(tip.position.y, floor);
        let tip = place(
            viewport(),
            tall_plate(),
            floor - 1.0,
            tip_size(),
            Some(legend),
        );
        assert_eq!(tip.position.y, floor);
        let tip = place(
            viewport(),
            tall_plate(),
            floor + 1.0,
            tip_size(),
            Some(legend),
        );
        assert_eq!(tip.position.y, floor + 1.0);
        let tip = place(viewport(), tall_plate(), 10_000.0, tip_size(), Some(legend));
        assert_eq!(tip.position.y, 140.0 + 800.0 - 200.0 - TIP_PAD);
        assert_eq!(
            place(viewport(), tall_plate(), 300.0, tip_size(), None)
                .position
                .y,
            300.0
        );
        assert_eq!(
            place(viewport(), tall_plate(), 0.0, tip_size(), None)
                .position
                .y,
            140.0 + TIP_PAD
        );
    }

    #[test]
    fn place_stacks_the_tip_on_the_flipped_side_too() {
        // The plate's right edge passes 75% of a 1280-wide viewport.
        let narrow = Vector2::new(1280.0, 900.0);
        let plate = Rect2::new(Vector2::new(400.0, 140.0), Vector2::new(600.0, 700.0));
        let legend = place_legend(narrow, plate, Vector2::new(183.0, 284.0));
        assert_eq!(legend.position.x, 400.0 - 183.0, "the legend flipped");
        let floor = 140.0 + 284.0 + STACK_GAP;
        let tip = place(narrow, plate, 148.0, tip_size(), Some(legend));
        assert_eq!(tip.position.x, 400.0 - TIP_WIDTH, "the tip flipped too");
        assert_eq!(tip.position.y, floor, "the floor applies on the left");
    }

    #[test]
    fn place_falls_back_to_row_anchored_when_the_stack_cannot_fit() {
        // The legend's floor (200 + 284 + 5 = 489) is past the band's hi
        // (392), so the stack cannot fit.
        let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
        let tip = place(viewport(), plate(), 300.0, tip_size(), Some(legend));
        assert_eq!(tip.position.y, 300.0, "row-anchored, over the legend");
        let tip = place(viewport(), plate(), 0.0, tip_size(), Some(legend));
        assert_eq!(
            tip.position.y,
            200.0 + TIP_PAD,
            "the band clamp, not the floor"
        );
    }

    #[test]
    fn frame_widens_the_control_sideways_only() {
        let plate = plate();
        let (control, origin_x) = frame(plate, None, None);
        assert_eq!(control, plate);
        assert_eq!(origin_x, 0.0);
        let tip = Rect2::new(Vector2::new(1260.0, 300.0), tip_size());
        let (control, origin_x) = frame(plate, None, Some(tip));
        assert_eq!(control.position, plate.position);
        assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
        assert_eq!(control.size.y, plate.size.y, "never a vertical extension");
        assert_eq!(origin_x, 0.0);
        let tip = Rect2::new(Vector2::new(300.0, 300.0), tip_size());
        let (control, origin_x) = frame(plate, None, Some(tip));
        assert_eq!(control.position.x, 300.0);
        assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
        assert_eq!(origin_x, 660.0 - 300.0);
    }

    #[test]
    fn frame_unions_the_legend_and_the_tip() {
        let plate = plate();
        let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
        let tip = Rect2::new(Vector2::new(1260.0, 400.0), tip_size());
        let (control, origin_x) = frame(plate, Some(legend), Some(tip));
        assert_eq!(control.position, plate.position);
        assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
        assert_eq!(control.size.y, plate.size.y);
        assert_eq!(origin_x, 0.0);
        let left = Rect2::new(Vector2::new(477.0, 200.0), Vector2::new(183.0, 284.0));
        let (control, origin_x) = frame(plate, Some(left), None);
        assert_eq!(control.position.x, 477.0);
        assert_eq!(control.size.x, plate.size.x + 183.0);
        assert_eq!(origin_x, 660.0 - 477.0);
        let short = Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 200.0));
        let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
        let (control, origin_x) = frame(short, Some(legend), None);
        assert_eq!(control.position, short.position);
        assert_eq!(control.size.y, 200.0 + 284.0 - 200.0);
        assert_eq!(origin_x, 0.0);
    }

    #[test]
    fn max_tip_lines_scales_with_the_box_and_floors() {
        assert_eq!(max_tip_lines(96.0), 1);
        assert_eq!(max_tip_lines(0.0), 1);
        let full = max_tip_lines(984.0);
        assert!(full >= 30, "a full-height box fits a full detail: {full}");
        let box_h = 300.0;
        assert!(tip_height(max_tip_lines(box_h)) <= box_h - 2.0 * TIP_PAD);
    }
}
