//! The five ledger-row source kinds: the stored `kind: u8` decoded at the
//! JSON boundary, and the C context code clamped at the ABI boundary.

use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum SourceKind {
    #[default]
    Card = 0,
    Relic = 1,
    Power = 2,
    Potion = 3,
    Osty = 4,
}

impl SourceKind {
    pub const ALL: [SourceKind; 5] = [
        SourceKind::Card,
        SourceKind::Relic,
        SourceKind::Power,
        SourceKind::Potion,
        SourceKind::Osty,
    ];

    /// The shim sends only card/relic/power for contexts; anything outside
    /// clamps to Power, matching the old boundary behavior.
    pub fn from_c(kind: i32) -> SourceKind {
        match kind.clamp(0, SourceKind::Power as i32) {
            0 => SourceKind::Card,
            1 => SourceKind::Relic,
            _ => SourceKind::Power,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Card => "card",
            SourceKind::Relic => "relic",
            SourceKind::Power => "power",
            SourceKind::Potion => "potion",
            SourceKind::Osty => "osty",
        }
    }
}

/// Unknown stored bytes read as Osty, matching the old parser tolerance.
impl From<u8> for SourceKind {
    fn from(kind: u8) -> SourceKind {
        match kind {
            0 => SourceKind::Card,
            1 => SourceKind::Relic,
            2 => SourceKind::Power,
            3 => SourceKind::Potion,
            _ => SourceKind::Osty,
        }
    }
}

impl From<SourceKind> for u8 {
    fn from(kind: SourceKind) -> u8 {
        kind as u8
    }
}
