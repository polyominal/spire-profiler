//! Structured chart payload: the constants plus `UiTab`/`UiRow`/`UiMeta`
//! that `snapshot` fills and `chart_layout` renders. The fixed field widths
//! are the layout contract: the segment slots, the name buffers, and the
//! per-mille bar widths are indexed by position.

use crate::source_kind::SourceKind;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiTab {
    #[default]
    Combat = 0,
    Run = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Damage = 0,
    Defense = 1,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Damage, Section::Defense];

    pub fn name(self) -> &'static str {
        match self {
            Section::Damage => "Damage",
            Section::Defense => "Defense",
        }
    }
}

/// A change here is a compile error at the `[u16; 8]` call sites.
pub const SEG_COUNT: usize = 8;

pub const SEG_DIRECT: usize = 0;
pub const SEG_ATTRIBUTED: usize = 1; // indirect damage (ticks, orbs, doom)
pub const SEG_MODIFIER: usize = 2;
pub const SEG_UPGRADE: usize = 3;
pub const SEG_MITIGATE_DEBUFF: usize = 4; // Weak-style prevention
pub const SEG_MITIGATE_BUFF: usize = 5; // Buffer/Intangible prevention
pub const SEG_MITIGATE_STR: usize = 6; // enemy Strength reduction
pub const SEG_SELF: usize = 7;

pub const ROW_FLAG_SELF: u8 = 2;
/// The source has self damage but no positive defense to split off.
pub const ROW_FLAG_SELF_SOLO: u8 = 4;

pub const MAX_ROWS_PER_SECTION: usize = 128;

/// The buffer must hold every candidate row: 128 × 2 = 256.
pub const MAX_UI_ROWS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiRow {
    pub section: Section,
    pub kind: SourceKind,
    /// TEAM (4) marks ownerless rows.
    pub player: u8,
    pub flags: u8,
    pub name_len: u8,
    pub plays: u32,
    pub value: i64,
    /// Percent of the section total ×10 (425 = 42.5%).
    pub share_x10: i32,
    /// Per-mille of the section maximum; the sum may exceed 1000.
    pub seg_milli: [u16; SEG_COUNT],
    pub name: [u8; 64],
}

impl Default for UiRow {
    fn default() -> Self {
        UiRow {
            section: Section::Damage,
            kind: SourceKind::Card,
            player: 0,
            flags: 0,
            name_len: 0,
            plays: 0,
            value: 0,
            share_x10: 0,
            seg_milli: [0; 8],
            name: [0; 64],
        }
    }
}

impl UiRow {
    /// Invalid UTF-8 (which corrupted records produce) yields "".
    pub fn name_str(&self) -> &str {
        // Corrupted records can carry name_len > 64.
        let len = usize::from(self.name_len).min(self.name.len());
        std::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UiMeta {
    pub turns: u32,
    pub plays: u32,
    pub combats: u32,
    pub total_damage: i64,
    pub damage_taken: i64,
    /// -1 when turns == 0 (the panel renders "—").
    pub dps_x10: i32,
    pub encounter_len: u8,
    pub encounter: [u8; 64],
}

impl Default for UiMeta {
    fn default() -> Self {
        UiMeta {
            turns: 0,
            plays: 0,
            combats: 0,
            total_damage: 0,
            damage_taken: 0,
            dps_x10: -1,
            encounter_len: 0,
            encounter: [0; 64],
        }
    }
}

impl UiMeta {
    pub fn encounter_str(&self) -> &str {
        let len = usize::from(self.encounter_len).min(self.encounter.len());
        std::str::from_utf8(&self.encounter[..len]).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_str_clamps_to_name_len_and_64_bytes() {
        let row = UiRow {
            name: [b'X'; 64],
            name_len: 255,
            ..UiRow::default()
        };
        assert_eq!(row.name_str().len(), 64);

        let mut row = UiRow {
            name: [b'X'; 64],
            ..UiRow::default()
        };
        row.name_len = 0;
        assert_eq!(row.name_str(), "");

        let mut row = UiRow::default();
        row.name[..6].copy_from_slice(b"STRIKE");
        row.name_len = 5;
        assert_eq!(row.name_str(), "STRIK");
    }

    #[test]
    fn encounter_str_clamps_to_encounter_len_and_64_bytes() {
        let meta = UiMeta {
            encounter: [b'E'; 64],
            encounter_len: 255,
            ..UiMeta::default()
        };
        assert_eq!(meta.encounter_str().len(), 64);

        let mut meta = UiMeta {
            encounter: [b'E'; 64],
            ..UiMeta::default()
        };
        meta.encounter_len = 0;
        assert_eq!(meta.encounter_str(), "");

        let mut meta = UiMeta::default();
        meta.encounter[..6].copy_from_slice(b"SLIMES");
        meta.encounter_len = 6;
        assert_eq!(meta.encounter_str(), "SLIMES");
    }
}
