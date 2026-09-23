//! Source row kinds keep their numeric identity in the native boundary and
//! immutable summary. Wire decoding happens before ledger operations.

use serde::Serialize;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Hash, Default, Serialize)]
#[serde(into = "u8")]
pub enum SourceKind {
    #[default]
    Card = 0,
    Relic = 1,
    Power = 2,
    Potion = 3,
    Osty = 4,
    Unknown = 5,
}

impl SourceKind {
    pub fn from_c(kind: i32) -> SourceKind {
        let clamped = kind.clamp(0, SourceKind::Unknown as i32);
        match clamped {
            0 => SourceKind::Card,
            1 => SourceKind::Relic,
            2 => SourceKind::Power,
            3 => SourceKind::Potion,
            4 => SourceKind::Osty,
            _ => SourceKind::Unknown,
        }
    }
}

impl From<SourceKind> for u8 {
    fn from(kind: SourceKind) -> u8 {
        kind as u8
    }
}
