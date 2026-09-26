use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Player {
    slot: u8,
    character: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Run {
    schema_version: u32,
    game_version: String,
    mod_version: String,
    pub(crate) run_id: String,
    pub(crate) preserved_run_ids: Vec<String>,
    pub(crate) profile: i32,
    pub(crate) seed: String,
    pub(crate) started_at: i64,
    pub(crate) ended_at: i64,
    character: String,
    ascension: i32,
    game_mode: String,
    pub(crate) outcome: String,
    pub(crate) players: Vec<Player>,
}

impl Run {
    pub(crate) fn validate(&self, requested: bool, imported: bool) -> Result<()> {
        let id_valid = Self::numeric_id(&self.run_id).is_some()
            || requested && self.run_id.is_empty()
            || imported && Self::archive_id(&self.run_id);
        let outcome_valid = matches!(
            self.outcome.as_str(),
            "active" | "suspended" | "victory" | "defeat" | "abandoned"
        ) || imported && self.outcome.is_empty();
        let mut aliases = HashSet::new();
        let mut slots = HashSet::new();
        if self.schema_version != 2
            || !id_valid
            || self.profile < -1
            || self.started_at < 0
            || self.ended_at < 0
            || !outcome_valid
            || self.preserved_run_ids.iter().any(|id| {
                id == "0" || id == &self.run_id || !Self::archive_id(id) || !aliases.insert(id)
            })
            || self.players.len() > 4
            || self.players.iter().any(|player| {
                player.slot > 3 || player.character.is_empty() || !slots.insert(player.slot)
            })
        {
            return Err("invalid run record".into());
        }
        Ok(())
    }

    pub(crate) fn numeric_id(id: &str) -> Option<u32> {
        id.parse::<u32>()
            .ok()
            .filter(|&number| number != 0 && number.to_string() == id)
    }

    fn archive_id(id: &str) -> bool {
        id.parse::<u32>().is_ok()
            || id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.profile == other.profile
            && self.seed == other.seed
            && self.started_at == other.started_at
    }

    pub(crate) fn finalized(&self) -> bool {
        matches!(self.outcome.as_str(), "victory" | "defeat" | "abandoned")
    }

    pub(crate) fn reidentify(&mut self, id: u32) {
        let canonical = id.to_string();
        if self.run_id != canonical && !self.preserved_run_ids.contains(&self.run_id) {
            self.preserved_run_ids.push(self.run_id.clone());
        }
        self.preserved_run_ids.retain(|alias| alias != &canonical);
        self.run_id = canonical;
    }

    pub(crate) fn history_fallback(mut self) -> Self {
        self.outcome.clear();
        self.ended_at = 0;
        self.players.clear();
        self
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Record {
    schema_version: u32,
    game_version: String,
    mod_version: String,
    pub(crate) run_id: String,
    pub(crate) ordinal: u32,
    pub(crate) run: Option<Run>,
    pub(crate) combat: Combat,
}

impl Record {
    pub(crate) fn validate(&self, imported: bool) -> Result<()> {
        let legacy = imported && self.combat.policy_version == 0;
        if self.schema_version != 2
            || self.ordinal == 0
            || self.combat.combat_id == 0
            || (!imported && self.combat.combat_id != self.ordinal)
            || (legacy && self.combat.combat_id != self.ordinal)
            || self.combat.policy_version < i32::from(!imported)
            || (!legacy
                && (self.combat.started_at < 0
                    || self.combat.damage_received < 0
                    || self.combat.block_total < 0))
            || !matches!(
                self.combat.result.as_str(),
                "completed" | "defeat" | "interrupted"
            )
        {
            return Err("invalid combat record".into());
        }
        if let Some(run) = &self.run {
            run.validate(false, imported)?;
            if run.run_id != self.run_id {
                return Err("combat and embedded run identities differ".into());
            }
        } else if self.run_id != "0" {
            return Err("combat has no run identity".into());
        }
        self.combat.coverage.validate()?;
        if legacy {
            if self
                .combat
                .cards
                .iter()
                .any(|row| row.kind > 5 || row.player > 4)
            {
                return Err("invalid legacy source identity".into());
            }
            Ok(())
        } else {
            Row::validate(&self.combat.cards)
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Combat {
    pub(crate) policy_version: i32,
    pub(crate) combat_id: u32,
    encounter_id: String,
    encounter_type: String,
    started_at: i64,
    result: String,
    turns: u32,
    plays: u32,
    potions_used: u32,
    damage_received: i64,
    block_total: i64,
    cards: Vec<Row>,
    coverage: Coverage,
}

#[derive(Deserialize, Serialize)]
struct Coverage {
    quality: Quality,
    failures: u64,
    reasons: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Quality {
    Unknown,
    Complete,
    Partial,
}

impl Coverage {
    fn validate(&self) -> Result<()> {
        if self.reasons.len() > 32
            || self.reasons.iter().any(String::is_empty)
            || matches!(self.quality, Quality::Complete)
                && (self.failures != 0 || !self.reasons.is_empty())
        {
            return Err("invalid capture coverage".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct Row {
    id: String,
    kind: u8,
    player: u8,
    plays: u32,
    damage_dealt: i64,
    damage_blocked: i64,
    block_gained: i64,
    block_effective: i64,
    forge: i64,
    dmg_direct: i64,
    dmg_attributed: i64,
    dmg_modifier: i64,
    blk_modifier: i64,
    mitigate_debuff: i64,
    mitigate_buff: i64,
    mitigate_str: i64,
    self_damage: i64,
}

impl Row {
    fn validate(rows: &[Self]) -> Result<()> {
        let mut keys = HashSet::new();
        let mut positive = [0_i128; 15];
        let mut negative = [0_i128; 15];
        let mut plays = 0_i64;
        for row in rows {
            let damage = i128::from(row.dmg_direct)
                + i128::from(row.dmg_attributed)
                + i128::from(row.dmg_modifier);
            let defense = i128::from(row.block_effective)
                + i128::from(row.blk_modifier)
                + i128::from(row.mitigate_debuff)
                + i128::from(row.mitigate_buff)
                + i128::from(row.mitigate_str);
            let nonnegative = [
                row.damage_dealt,
                row.damage_blocked,
                row.block_gained,
                row.forge,
                row.dmg_direct,
                row.dmg_attributed,
                row.dmg_modifier,
                row.blk_modifier,
                row.mitigate_debuff,
                row.mitigate_buff,
                row.mitigate_str,
                row.self_damage,
            ];
            if row.id.is_empty()
                || row.kind > 5
                || row.player > 4
                || !keys.insert((row.player, row.kind, row.id.as_str()))
                || nonnegative.iter().any(|&value| value < 0)
                || row.damage_blocked > row.damage_dealt
                || damage != i128::from(row.damage_dealt)
                || i64::try_from(defense - i128::from(row.self_damage)).is_err()
            {
                return Err("invalid source identity or accounting".into());
            }
            let values = [
                i128::from(row.damage_dealt),
                i128::from(row.damage_blocked),
                i128::from(row.block_gained),
                i128::from(row.block_effective),
                i128::from(row.dmg_direct),
                i128::from(row.dmg_attributed),
                i128::from(row.dmg_modifier),
                i128::from(row.blk_modifier),
                i128::from(row.mitigate_debuff),
                i128::from(row.mitigate_buff),
                i128::from(row.mitigate_str),
                i128::from(row.self_damage),
                i128::from(row.forge),
                damage,
                defense,
            ];
            for ((positive, negative), value) in positive.iter_mut().zip(&mut negative).zip(values)
            {
                *positive += value.max(0);
                *negative += value.min(0);
                if *positive > i128::from(i64::MAX) || *negative < i128::from(i64::MIN) {
                    return Err("source totals exceed the signed ledger domain".into());
                }
            }
            plays = plays
                .checked_add(i64::from(row.plays))
                .ok_or("source plays overflow")?;
        }
        Ok(())
    }
}
