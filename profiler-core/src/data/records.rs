//! The persisted record types and their serde contracts: the read-side
//! mirrors of the JSON the core writes. Unknown fields are ignored and
//! missing fields fall back to defaults — that tolerance is the schema's
//! additive evolution contract.

use serde::{Deserialize, Serialize};

use crate::data::state::{RunContext, RunPlayer, outcome_name};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CardRec {
    pub id: String,
    pub kind: u8,
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
    pub dmg_upgrade: i64,
    pub blk_modifier: i64,
    pub blk_upgrade: i64,
    pub mitigate_debuff: i64,
    pub mitigate_buff: i64,
    pub mitigate_str: i64,
    pub self_damage: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PlayerRec {
    pub slot: u8,
    pub character: String,
}

/// Enough to rejoin a resumed run's fragments and synthesize the fallback
/// view's header.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RunRec {
    pub seq: u32,
    pub character: String,
    pub ascension: i32,
    pub game_mode: String,
    pub seed: String,
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
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CombatRec {
    pub combat_id: u32,
    pub started_at: i64,
    pub encounter_id: String,
    pub result: String,
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
    outcome: &'a str,
    seed: &'a str,
    started_at: i64,
    ended_at: i64,
    /// Omitted when empty, so the field is additive.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    players: Vec<PlayerDoc<'a>>,
}

#[cfg(any(test, debug_assertions))]
#[derive(Deserialize)]
struct RunDocOwned {
    run_id: u32,
    profile: i32,
    character: String,
    ascension: i32,
    game_mode: String,
    outcome: String,
    seed: String,
    started_at: i64,
    ended_at: i64,
    /// The emission omits the field when the roster is empty.
    #[serde(default)]
    players: Vec<PlayerRec>,
}

/// `profile` is the shim-forwarded SaveManager.ProfileId (-1 when never
/// reported).
pub fn build_run_json(run: &RunContext, profile: i32) -> String {
    let doc = RunDoc {
        run_id: run.seq,
        profile,
        character: &run.character,
        ascension: run.ascension,
        game_mode: &run.game_mode,
        outcome: outcome_name(run.outcome),
        seed: &run.seed,
        started_at: run.started_at,
        ended_at: run.ended_at,
        players: run.players.iter().map(PlayerDoc::from).collect(),
    };
    let json = serde_json::to_string(&doc).expect("run document cannot fail to serialize");
    // The emitted entry must parse back to the run record that produced it.
    #[cfg(debug_assertions)]
    {
        let parsed: RunDocOwned = serde_json::from_str(&json).expect("run JSON must parse back");
        debug_assert_eq!(parsed.run_id, run.seq, "run run_id must round-trip");
        debug_assert_eq!(parsed.profile, profile, "run profile must round-trip");
        debug_assert_eq!(
            parsed.character, run.character,
            "run character must round-trip"
        );
        debug_assert_eq!(
            parsed.ascension, run.ascension,
            "run ascension must round-trip"
        );
        debug_assert_eq!(
            parsed.game_mode, run.game_mode,
            "run game_mode must round-trip"
        );
        debug_assert_eq!(
            parsed.outcome,
            outcome_name(run.outcome),
            "run outcome must round-trip"
        );
        debug_assert_eq!(parsed.seed, run.seed, "run seed must round-trip");
        debug_assert_eq!(
            parsed.started_at, run.started_at,
            "run started_at must round-trip"
        );
        debug_assert_eq!(
            parsed.ended_at, run.ended_at,
            "run ended_at must round-trip"
        );
        debug_assert_eq!(
            parsed.players.len(),
            run.players.len(),
            "run roster must round-trip"
        );
        for (parsed, wrote) in parsed.players.iter().zip(&run.players) {
            debug_assert_eq!(parsed.slot, wrote.slot, "run roster slot must round-trip");
            debug_assert_eq!(
                parsed.character, wrote.character,
                "run roster character must round-trip"
            );
        }
    }
    json
}

#[cfg(test)]
mod tests;
