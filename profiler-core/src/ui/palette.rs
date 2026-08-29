//! The panels' color vocabulary: the segment palette, its semantic slot
//! mapping, and the color legend. Every color is a verbatim game color
//! quoted from `StsColors.cs` or a shipped scene; each constant names its
//! source. [`slot_color`] is the bar palette's single resolution point:
//! bars, legend chips, and tooltip stat values all call it, so no two
//! views can drift.

use crate::engine::math::Vector2;
use crate::source_kind::SourceKind;
use crate::ui::chart_layout::{self, Cmd};
use crate::ui::theme;
use crate::ui::ui_model::{Section, Segment};

pub type Color = [f32; 4];

// Warm hues for harm (damage), cool hues for protection (defense): a
// bar's section reads from its leading segment before its header does.

/// run_history.tscn HpLabel `font_color` #FF6464.
pub const COL_DMG_DIRECT: Color = [1.0, 0.392157, 0.392157, 1.0];
/// StsColors.orange #FFA518.
pub const COL_ATTRIBUTED: Color = [1.0, 0.647059, 0.094118, 1.0];
/// StsColors.purple #EE82EE.
pub const COL_MODIFIER: Color = [0.933333, 0.509804, 0.933333, 1.0];
/// StsColors.disabledRed #BF3030.
pub const COL_SELF: Color = [0.749020, 0.188235, 0.188235, 1.0];
/// StsColors.darkBlue #67AEEB.
pub const COL_BLOCK: Color = [0.403922, 0.682353, 0.921569, 1.0];
/// StsColors.green #7FFF00.
pub const COL_MITIGATE_DEBUFF: Color = [0.498039, 1.0, 0.0, 1.0];
/// StsColors.blue #87CEEB.
pub const COL_MITIGATE_BUFF: Color = [0.529412, 0.807843, 0.921569, 1.0];
/// StsColors.aqua #2AEBBE.
pub const COL_MITIGATE_STR: Color = [0.164706, 0.921569, 0.745098, 1.0];
/// scenes/ui/top_bar.tscn FloorNumLabel `font_color` #C5DACE.
pub const COL_OSTY: Color = [0.772549, 0.854902, 0.807843, 1.0];

// 0.318 is the palette's #EFC851 blue channel (81/255), not an
// approximation of 1/π.
#[allow(clippy::approx_constant)]
pub(crate) const COL_GOLD: Color = [0.937, 0.784, 0.318, 1.0]; // StsColors.gold #EFC851
pub(crate) const COL_CREAM: Color = [1.0, 0.964706, 0.886275, 1.0]; // StsColors.cream #FFF6E2
/// StsColors.halfTransparentCream.
pub(crate) const COL_DIM: Color = [1.0, 0.964706, 0.886275, 0.5];
pub(crate) const COL_ROW_ALT: Color = [1.0, 1.0, 1.0, 0.025];
pub(crate) const COL_TRACK: Color = [1.0, 1.0, 1.0, 0.06];
pub(crate) const COL_HOVER: Color = [1.0, 1.0, 1.0, 0.08];
/// StsColors.quarterTransparentBlack.
pub(crate) const COL_HEADER_BG: Color = [0.0, 0.0, 0.0, 0.25];
/// Body-text shadow, 50% alpha.
pub(crate) const COL_SHADOW: Color = [0.0, 0.0, 0.0, 0.5];
/// Header shadow, 12.5% alpha.
pub(crate) const COL_HEADER_SHADOW: Color = [0.0, 0.0, 0.0, 0.12549];
/// vertical_popup.tscn Header `font_outline_color` #543F00.
pub(crate) const COL_HEADER_OUTLINE: Color = [0.33, 0.2475, 0.0, 1.0];
/// Tooltip shadow, 25% alpha.
pub(crate) const COL_TIP_SHADOW: Color = [0.0, 0.0, 0.0, 0.25098];
pub(crate) const COL_PANEL_BG: Color = [0.05, 0.05, 0.10, 0.78];
pub(crate) const COL_PANEL_BORDER: Color = [0.30, 0.40, 0.70, 0.60];

// Per-player tints are deliberately absent: a tint would blend a per-slot
// hue into the name/segment colors; the filter label already names the
// viewer.

/// Section-tinted, not kind-tinted; Osty's absorb shares its `[O] ` hue.
pub fn slot_color(slot: Segment, section: Section, kind: SourceKind) -> Color {
    match slot {
        Segment::Direct => match section {
            Section::Defense if kind == SourceKind::Osty => COL_OSTY,
            Section::Defense => COL_BLOCK,
            Section::Damage => COL_DMG_DIRECT,
        },
        Segment::Attributed => COL_ATTRIBUTED,
        Segment::Modifier => COL_MODIFIER,
        Segment::MitigateDebuff => COL_MITIGATE_DEBUFF,
        Segment::MitigateBuff => COL_MITIGATE_BUFF,
        Segment::MitigateStr => COL_MITIGATE_STR,
        Segment::SelfDamage => COL_SELF,
    }
}

pub(crate) struct KindPrefix {
    pub color: Color,
    pub text: &'static str,
}

pub(crate) fn kind_prefix(kind: SourceKind) -> Option<KindPrefix> {
    match kind {
        SourceKind::Relic => Some(KindPrefix {
            color: COL_GOLD,
            text: "[R] ",
        }),
        SourceKind::Potion => Some(KindPrefix {
            color: COL_MITIGATE_STR,
            text: "[P] ",
        }),
        SourceKind::Osty => Some(KindPrefix {
            color: COL_OSTY,
            text: "[O] ",
        }),
        SourceKind::Card | SourceKind::Power => None,
    }
}

/// "[O] " = 40.7px at 24px plus 1px-per-glyph tracking, ceiled; one fixed
/// column keeps the names aligned.
pub(crate) const PREFIX_ADVANCE: f32 = 41.0;

// The key is STATIC — the full vocabulary, not the current tab's entries —
// and the chips draw from the same `slot_color` calls the bars make.

const LEGEND_CHIP_W: f32 = 20.0;
const LEGEND_CHIP_H: f32 = 12.0;
pub(crate) const LEGEND_LINE_H: f32 = 24.0;
const LEGEND_TEXT_Y: f32 = 18.0;
const LEGEND_CHIP_Y: f32 = 5.0;
/// The chip + a 6px gap + the longest label ("str down" ≈ 87px).
pub(crate) const LEGEND_W: f32 = 116.0;
pub(crate) const LEGEND_H: f32 = LEGEND_ENTRIES.len() as f32 * LEGEND_LINE_H;

pub(crate) struct LegendPlate {
    pub size: Vector2,
    pub origin: Vector2,
}

pub(crate) fn legend_plate(plate: bool) -> LegendPlate {
    if plate {
        LegendPlate {
            size: Vector2::new(
                theme::PLATE_PAD_LEFT
                    + LEGEND_W
                    + theme::PLATE_PAD_RIGHT
                    + theme::PLATE_SHADOW_OFFSET,
                theme::PLATE_PAD_TOP
                    + LEGEND_H
                    + theme::PLATE_PAD_BOTTOM
                    + theme::PLATE_SHADOW_OFFSET,
            ),
            origin: Vector2::new(theme::PLATE_PAD_LEFT, theme::PLATE_PAD_TOP),
        }
    } else {
        LegendPlate {
            size: Vector2::new(
                LEGEND_W + 2.0 * theme::FLAT_PAD,
                LEGEND_H + 2.0 * theme::FLAT_PAD,
            ),
            origin: Vector2::new(theme::FLAT_PAD, theme::FLAT_PAD),
        }
    }
}

/// Resolves through [`slot_color`] at emission, never a literal.
pub(crate) struct LegendEntry {
    label: &'static str,
    slot: Segment,
    section: Section,
    kind: SourceKind,
}

/// One entry per distinct bar color, damage family first. Kind markers
/// swatch nothing: each prefix hue already appears as a segment color.
pub(crate) const LEGEND_ENTRIES: &[LegendEntry] = &[
    LegendEntry {
        label: "direct",
        slot: Segment::Direct,
        section: Section::Damage,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "indirect",
        slot: Segment::Attributed,
        section: Section::Damage,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "modifier",
        slot: Segment::Modifier,
        section: Section::Damage,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "block",
        slot: Segment::Direct,
        section: Section::Defense,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "osty",
        slot: Segment::Direct,
        section: Section::Defense,
        kind: SourceKind::Osty,
    },
    LegendEntry {
        label: "weak",
        slot: Segment::MitigateDebuff,
        section: Section::Defense,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "buff",
        slot: Segment::MitigateBuff,
        section: Section::Defense,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "str down",
        slot: Segment::MitigateStr,
        section: Section::Defense,
        kind: SourceKind::Card,
    },
    LegendEntry {
        label: "self dmg",
        slot: Segment::SelfDamage,
        section: Section::Defense,
        kind: SourceKind::Card,
    },
];

pub(crate) fn emit_legend(cmds: &mut Vec<Cmd>, x: f32, y_in: f32) -> f32 {
    let mut sink = chart_layout::CmdSink::new(cmds, "legend");
    let mut y = y_in;
    for entry in LEGEND_ENTRIES {
        sink.rect(
            x,
            y + LEGEND_CHIP_Y,
            LEGEND_CHIP_W,
            LEGEND_CHIP_H,
            slot_color(entry.slot, entry.section, entry.kind),
        );
        sink.text(
            x + LEGEND_CHIP_W + 6.0,
            y + LEGEND_TEXT_Y,
            theme::SIZE_TOOLTIP,
            COL_DIM,
            entry.label,
        );
        y += LEGEND_LINE_H;
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_color_maps_the_section_families() {
        for kind in SourceKind::ALL {
            assert_eq!(
                slot_color(Segment::Direct, Section::Damage, kind),
                COL_DMG_DIRECT,
                "kind {kind:?}"
            );
            let expect = if kind == SourceKind::Osty {
                COL_OSTY
            } else {
                COL_BLOCK
            };
            assert_eq!(
                slot_color(Segment::Direct, Section::Defense, kind),
                expect,
                "kind {kind:?}"
            );
        }
        assert_eq!(
            slot_color(Segment::Attributed, Section::Damage, SourceKind::Card),
            COL_ATTRIBUTED
        );
        assert_eq!(
            slot_color(Segment::Modifier, Section::Defense, SourceKind::Card),
            COL_MODIFIER
        );
        assert_eq!(
            slot_color(Segment::MitigateDebuff, Section::Defense, SourceKind::Card),
            COL_MITIGATE_DEBUFF
        );
        assert_eq!(
            slot_color(Segment::MitigateBuff, Section::Defense, SourceKind::Card),
            COL_MITIGATE_BUFF
        );
        assert_eq!(
            slot_color(Segment::MitigateStr, Section::Defense, SourceKind::Card),
            COL_MITIGATE_STR
        );
        assert_eq!(
            slot_color(Segment::SelfDamage, Section::Defense, SourceKind::Card),
            COL_SELF
        );
    }

    /// Chips share the bars' `slot_color` call, so the key cannot lie.
    #[test]
    fn legend_chips_match_the_bars_and_cover_every_slot() {
        for slot in Segment::ALL {
            assert!(
                LEGEND_ENTRIES.iter().any(|e| e.slot == slot),
                "segment slot {slot:?} has no legend entry"
            );
        }
        let mut cmds = Vec::new();
        let bottom = emit_legend(&mut cmds, 100.0, 40.0);
        assert_eq!(cmds.len(), 2 * LEGEND_ENTRIES.len());
        assert_eq!(bottom, 40.0 + LEGEND_H);
        for (i, entry) in LEGEND_ENTRIES.iter().enumerate() {
            let chip = cmds.iter().any(|cmd| {
                matches!(cmd, Cmd::Rect(r) if r.x == 100.0
                    && r.y == 40.0 + i as f32 * LEGEND_LINE_H + LEGEND_CHIP_Y
                    && r.w == LEGEND_CHIP_W && r.h == LEGEND_CHIP_H
                    && r.color == slot_color(entry.slot, entry.section, entry.kind))
            });
            assert!(chip, "entry {} has no bar-colored chip", entry.label);
            let label = cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Text(t) if t.text == entry.label => Some(t),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("entry {} renders", entry.label));
            assert_eq!(label.color, COL_DIM);
            assert_eq!(label.size, theme::SIZE_TOOLTIP);
            assert_eq!(label.x, 100.0 + LEGEND_CHIP_W + 6.0);
            assert_eq!(label.y, 40.0 + i as f32 * LEGEND_LINE_H + LEGEND_TEXT_Y);
        }
    }

    #[test]
    fn legend_plate_sizes_around_the_key() {
        let plate = legend_plate(true);
        assert_eq!(
            plate.origin,
            Vector2::new(theme::PLATE_PAD_LEFT, theme::PLATE_PAD_TOP)
        );
        assert_eq!(
            plate.size,
            Vector2::new(
                theme::PLATE_PAD_LEFT
                    + LEGEND_W
                    + theme::PLATE_PAD_RIGHT
                    + theme::PLATE_SHADOW_OFFSET,
                theme::PLATE_PAD_TOP
                    + LEGEND_H
                    + theme::PLATE_PAD_BOTTOM
                    + theme::PLATE_SHADOW_OFFSET,
            )
        );
        assert_eq!(
            plate.origin.x + LEGEND_W + theme::PLATE_PAD_RIGHT,
            plate.size.x - theme::PLATE_SHADOW_OFFSET
        );
        assert_eq!(
            plate.origin.y + LEGEND_H + theme::PLATE_PAD_BOTTOM,
            plate.size.y - theme::PLATE_SHADOW_OFFSET
        );

        let flat = legend_plate(false);
        assert_eq!(flat.origin, Vector2::new(theme::FLAT_PAD, theme::FLAT_PAD));
        assert_eq!(
            flat.size,
            Vector2::new(
                LEGEND_W + 2.0 * theme::FLAT_PAD,
                LEGEND_H + 2.0 * theme::FLAT_PAD
            )
        );
    }
}
