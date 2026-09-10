//! The ledger-row source kinds: the stored `kind: u8` decoded at the
//! JSON boundary, and the C source code clamped at the ABI boundary.

use std::cell::Cell;

use serde::{Deserialize, Serialize};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum SourceKind {
    #[default]
    Card = 0,
    Relic = 1,
    Power = 2,
    Potion = 3,
    Osty = 4,
    Unknown = 5,
}

std::thread_local! {
    static BAD_KIND_LOGGED: Cell<bool> = const { Cell::new(false) };
}

impl SourceKind {
    pub const ALL: [SourceKind; 6] = [
        SourceKind::Card,
        SourceKind::Relic,
        SourceKind::Power,
        SourceKind::Potion,
        SourceKind::Osty,
        SourceKind::Unknown,
    ];

    pub fn from_c(kind: i32) -> SourceKind {
        let clamped = kind.clamp(0, SourceKind::Unknown as i32);
        if clamped != kind {
            crate::fail_once(
                &BAD_KIND_LOGGED,
                format_args!("invalid source kind {kind}; clamping to {clamped}"),
            );
        }
        match clamped {
            0 => SourceKind::Card,
            1 => SourceKind::Relic,
            2 => SourceKind::Power,
            3 => SourceKind::Potion,
            4 => SourceKind::Osty,
            _ => SourceKind::Unknown,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Card => "card",
            SourceKind::Relic => "relic",
            SourceKind::Power => "power",
            SourceKind::Potion => "potion",
            SourceKind::Osty => "osty",
            SourceKind::Unknown => "unknown",
        }
    }
}

impl From<u8> for SourceKind {
    fn from(kind: u8) -> SourceKind {
        match kind {
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
