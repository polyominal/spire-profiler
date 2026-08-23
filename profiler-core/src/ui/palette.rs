//! The panels' color vocabulary: the segment palette, its semantic slot
//! mapping, and the color legend. Every color is a verbatim game color
//! quoted from `StsColors.cs` or a shipped scene; each constant names its
//! source. [`slot_color`] is the bar palette's single resolution point:
//! bars, legend chips, and tooltip stat values all call it, so no two
//! views can drift.

use crate::engine::math::Vector2;
use crate::engine::object::TextAlign;
use crate::ui::chart_layout::{self, Cmd, RectCmd, TextCmd};
use crate::ui::theme::{self, TextRole};
use crate::ui::ui_model;

pub type Color = [f32; 4];

// Warm hues for harm (damage), cool hues for protection (defense): a
// bar's section reads from its leading segment before its header does.

/// run_history.tscn HpLabel `font_color` #FF6464.
pub const COL_DMG_DIRECT: Color = [1.0, 0.392157, 0.392157, 1.0];
/// StsColors.orange #FFA518.
pub const COL_ATTRIBUTED: Color = [1.0, 0.647059, 0.094118, 1.0];
/// StsColors.purple #EE82EE.
pub const COL_MODIFIER: Color = [0.933333, 0.509804, 0.933333, 1.0];
/// [`COL_GOLD`], aliased so the segment table reads in its own vocabulary.
pub const COL_UPGRADE: Color = COL_GOLD;
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
pub fn slot_color(slot: usize, section: u8, kind: u8) -> Color {
    match slot {
        ui_model::SEG_DIRECT => match section {
            ui_model::SECTION_DEFENSE if kind == ui_model::KIND_OSTY => COL_OSTY,
            ui_model::SECTION_DEFENSE => COL_BLOCK,
            _ => COL_DMG_DIRECT,
        },
        ui_model::SEG_ATTRIBUTED => COL_ATTRIBUTED,
        ui_model::SEG_MODIFIER => COL_MODIFIER,
        ui_model::SEG_UPGRADE => COL_UPGRADE,
        ui_model::SEG_MITIGATE_DEBUFF => COL_MITIGATE_DEBUFF,
        ui_model::SEG_MITIGATE_BUFF => COL_MITIGATE_BUFF,
        ui_model::SEG_MITIGATE_STR => COL_MITIGATE_STR,
        ui_model::SEG_SELF => COL_SELF,
        _ => COL_DMG_DIRECT,
    }
}

pub(crate) fn kind_prefix(kind: u8) -> &'static str {
    match kind {
        ui_model::KIND_RELIC => "[R] ",
        ui_model::KIND_POTION => "[P] ",
        ui_model::KIND_OSTY => "[O] ",
        _ => "",
    }
}

/// The same-hue collisions across the two channels are deliberate: one hue
/// reads as one vocabulary.
pub(crate) fn kind_prefix_color(kind: u8) -> Option<Color> {
    match kind {
        ui_model::KIND_RELIC => Some(COL_GOLD),
        ui_model::KIND_POTION => Some(COL_MITIGATE_STR),
        ui_model::KIND_OSTY => Some(COL_OSTY),
        _ => None,
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
    slot: usize,
    section: u8,
    kind: u8,
}

/// One entry per distinct bar color, damage family first. Kind markers
/// swatch nothing: each prefix hue already appears as a segment color.
pub(crate) const LEGEND_ENTRIES: &[LegendEntry] = &[
    LegendEntry {
        label: "direct",
        slot: ui_model::SEG_DIRECT,
        section: ui_model::SECTION_DAMAGE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "indirect",
        slot: ui_model::SEG_ATTRIBUTED,
        section: ui_model::SECTION_DAMAGE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "modifier",
        slot: ui_model::SEG_MODIFIER,
        section: ui_model::SECTION_DAMAGE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "upgrade",
        slot: ui_model::SEG_UPGRADE,
        section: ui_model::SECTION_DAMAGE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "block",
        slot: ui_model::SEG_DIRECT,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "osty",
        slot: ui_model::SEG_DIRECT,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_OSTY,
    },
    LegendEntry {
        label: "weak",
        slot: ui_model::SEG_MITIGATE_DEBUFF,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "buff",
        slot: ui_model::SEG_MITIGATE_BUFF,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "str down",
        slot: ui_model::SEG_MITIGATE_STR,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_CARD,
    },
    LegendEntry {
        label: "self dmg",
        slot: ui_model::SEG_SELF,
        section: ui_model::SECTION_DEFENSE,
        kind: ui_model::KIND_CARD,
    },
];

pub(crate) fn emit_legend(cmds: &mut Vec<Cmd>, x: f32, y_in: f32) -> f32 {
    let mut y = y_in;
    for entry in LEGEND_ENTRIES {
        chart_layout::push_cmd(
            cmds,
            Cmd::Rect(RectCmd {
                x,
                y: y + LEGEND_CHIP_Y,
                w: LEGEND_CHIP_W,
                h: LEGEND_CHIP_H,
                color: slot_color(entry.slot, entry.section, entry.kind),
            }),
            "legend",
        );
        chart_layout::push_cmd(
            cmds,
            Cmd::Text(TextCmd {
                x: x + LEGEND_CHIP_W + 6.0,
                y: y + LEGEND_TEXT_Y,
                size: theme::SIZE_TOOLTIP,
                color: COL_DIM,
                role: TextRole::Body,
                shadow: false,
                outline: false,
                align: TextAlign::Left,
                text: entry.label.to_owned(),
            }),
            "legend",
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
        for kind in [
            ui_model::KIND_CARD,
            ui_model::KIND_RELIC,
            ui_model::KIND_POWER,
            ui_model::KIND_POTION,
            ui_model::KIND_OSTY,
        ] {
            assert_eq!(
                slot_color(ui_model::SEG_DIRECT, ui_model::SECTION_DAMAGE, kind),
                COL_DMG_DIRECT,
                "kind {kind}"
            );
            let expect = if kind == ui_model::KIND_OSTY {
                COL_OSTY
            } else {
                COL_BLOCK
            };
            assert_eq!(
                slot_color(ui_model::SEG_DIRECT, ui_model::SECTION_DEFENSE, kind),
                expect,
                "kind {kind}"
            );
        }
        assert_eq!(
            slot_color(ui_model::SEG_ATTRIBUTED, ui_model::SECTION_DAMAGE, 0),
            COL_ATTRIBUTED
        );
        assert_eq!(
            slot_color(ui_model::SEG_MODIFIER, ui_model::SECTION_DEFENSE, 0),
            COL_MODIFIER
        );
        assert_eq!(
            slot_color(ui_model::SEG_UPGRADE, ui_model::SECTION_DAMAGE, 0),
            COL_GOLD,
            "the upgrade segment IS the game's gold accent"
        );
        assert_eq!(
            slot_color(ui_model::SEG_MITIGATE_DEBUFF, ui_model::SECTION_DEFENSE, 0),
            COL_MITIGATE_DEBUFF
        );
        assert_eq!(
            slot_color(ui_model::SEG_MITIGATE_BUFF, ui_model::SECTION_DEFENSE, 0),
            COL_MITIGATE_BUFF
        );
        assert_eq!(
            slot_color(ui_model::SEG_MITIGATE_STR, ui_model::SECTION_DEFENSE, 0),
            COL_MITIGATE_STR
        );
        assert_eq!(
            slot_color(ui_model::SEG_SELF, ui_model::SECTION_DEFENSE, 0),
            COL_SELF
        );
    }

    #[test]
    fn kind_prefix_and_its_color_cover_the_same_kinds() {
        for kind in 0..=u8::MAX {
            assert_eq!(
                kind_prefix(kind).is_empty(),
                kind_prefix_color(kind).is_none(),
                "kind {kind}"
            );
        }
    }

    /// Chips share the bars' `slot_color` call, so the key cannot lie.
    #[test]
    fn legend_chips_match_the_bars_and_cover_every_slot() {
        for slot in 0..ui_model::SEG_COUNT {
            assert!(
                LEGEND_ENTRIES.iter().any(|e| e.slot == slot),
                "segment slot {slot} has no legend entry"
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
