//! The persisted record types and their serde contracts: the read-side
//! mirrors of the JSON the core writes. Unknown fields are ignored and
//! missing fields fall back to defaults; that tolerance is boundary
//! robustness, not an evolution contract.

use serde::{Deserialize, Serialize};

use crate::data::state::{CombatResult, EndedRun, RunOutcome, RunPlayer};
use crate::source_kind::SourceKind;

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CardRec {
    pub id: String,
    pub kind: SourceKind,
    pub player: u8,
    pub plays: u32,
    pub damage_dealt: i64,
    pub damage_blocked: i64,
    pub block_gained: i64,
    pub block_effective: i64,
    pub forge: i64,
    pub dmg_direct: i64,
    pub dmg_attributed: i64,
    pub dmg_modifier: i64,
    pub blk_modifier: i64,
    pub mitigate_debuff: i64,
    pub mitigate_buff: i64,
    pub mitigate_str: i64,
    pub self_damage: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlayerRec {
    pub slot: u8,
    pub character: String,
}

/// Enough to rejoin a resumed run's fragments and synthesize the fallback
/// view's header.
#[derive(Deserialize)]
#[serde(default)]
pub struct RunRec {
    pub seq: u32,
    pub character: String,
    pub ascension: i32,
    pub game_mode: String,
    pub seed: String,
    pub profile: i32,
    pub started_at: i64,
}

impl Default for RunRec {
    fn default() -> Self {
        RunRec {
            seq: 0,
            character: String::new(),
            // -1 means "the shim never reported an ascension".
            ascension: -1,
            game_mode: String::new(),
            seed: String::new(),
            profile: -1,
            started_at: 0,
        }
    }
}

impl RunRec {
    pub(crate) fn matches_identity(&self, seed: &str, started_at: i64, profile: i32) -> bool {
        self.seq != 0
            && !seed.is_empty()
            && started_at > 0
            && profile >= 0
            && self.seed == seed
            && self.started_at == started_at
            && self.profile == profile
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct CombatRec {
    pub combat_id: u32,
    pub started_at: i64,
    pub encounter_id: String,
    pub result: CombatResult,
    pub turns: u32,
    pub damage_received: i64,
    pub run: Option<RunRec>,
    pub cards: Vec<CardRec>,
}

pub fn parse_combat_doc(content: &str) -> serde_json::Result<CombatRec> {
    serde_json::from_str(content)
}

/// A borrowed roster entry for the JSON document.
#[derive(Serialize)]
pub struct PlayerDoc<'a> {
    slot: u8,
    character: &'a str,
}

impl<'a> From<&'a PlayerRec> for PlayerDoc<'a> {
    fn from(p: &'a PlayerRec) -> Self {
        PlayerDoc {
            slot: p.slot,
            character: &p.character,
        }
    }
}

impl<'a> From<&'a RunPlayer> for PlayerDoc<'a> {
    fn from(p: &'a RunPlayer) -> Self {
        PlayerDoc {
            slot: p.slot,
            character: &p.character,
        }
    }
}

/// The runs.jsonl entry shape, in the documented field order.
#[derive(Serialize)]
struct RunDoc<'a> {
    run_id: u32,
    profile: i32,
    character: &'a str,
    ascension: i32,
    game_mode: &'a str,
    outcome: RunOutcome,
    seed: &'a str,
    started_at: i64,
    ended_at: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    players: Vec<PlayerDoc<'a>>,
}

#[cfg(test)]
#[derive(Deserialize)]
struct RunDocOwned {
    run_id: u32,
    profile: i32,
    character: String,
    ascension: i32,
    game_mode: String,
    outcome: RunOutcome,
    seed: String,
    started_at: i64,
    ended_at: i64,
    /// The emission omits the field when the roster is empty.
    #[serde(default)]
    players: Vec<PlayerRec>,
}

pub fn build_run_json(ended: &EndedRun) -> String {
    let run = &ended.context;
    let doc = RunDoc {
        run_id: run.run.seq,
        profile: run.run.profile,
        character: &run.run.character,
        ascension: run.run.ascension,
        game_mode: &run.run.game_mode,
        outcome: ended.outcome,
        seed: &run.run.seed,
        started_at: run.run.started_at,
        ended_at: ended.ended_at,
        players: run.players.iter().map(PlayerDoc::from).collect(),
    };
    serde_json::to_string(&doc).expect("run document cannot fail to serialize")
}

#[cfg(test)]
mod tests;
