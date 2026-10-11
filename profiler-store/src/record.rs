use std::collections::HashSet;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize, Serializer};

use crate::Result;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Player {
    slot: u8,
    character: String,
}

/// JSON begins as `Run<String>`; parsing retains an owned or archived ID.
/// Only this module can change identities, aliases, or lifecycle metadata.
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Run<Id = String> {
    schema_version: u32,
    game_version: String,
    mod_version: String,
    run_id: Id,
    preserved_run_ids: Vec<ArchiveId>,
    pub(crate) profile: i32,
    pub(crate) seed: String,
    pub(crate) started_at: i64,
    ended_at: i64,
    character: String,
    ascension: i32,
    game_mode: String,
    outcome: String,
    players: Vec<Player>,
}

/// Canonical storage IDs stay numeric after their JSON string boundary.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct RunId(NonZeroU32);

impl RunId {
    pub(crate) fn new(id: u32) -> Result<Self> {
        NonZeroU32::new(id)
            .map(Self)
            .ok_or_else(|| "invalid run ID".into())
    }

    pub(crate) fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<String> for RunId {
    type Error = &'static str;

    fn try_from(id: String) -> std::result::Result<Self, Self::Error> {
        id.parse::<NonZeroU32>()
            .ok()
            .filter(|number| number.to_string() == id)
            .map(Self)
            .ok_or("invalid run ID")
    }
}

impl From<RunId> for String {
    fn from(id: RunId) -> Self {
        id.0.to_string()
    }
}

/// Archive spellings are preserved, but zero never identifies a run owner.
#[derive(Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct ArchiveId(String);

impl ArchiveId {
    pub(crate) fn numeric(&self) -> Option<RunId> {
        RunId::try_from(self.0.clone()).ok()
    }
}

impl TryFrom<String> for ArchiveId {
    type Error = &'static str;

    fn try_from(id: String) -> std::result::Result<Self, Self::Error> {
        if id.parse::<NonZeroU32>().is_ok()
            || id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            Ok(Self(id))
        } else {
            Err("invalid archived run ID")
        }
    }
}

impl Serialize for ArchiveId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl Run<String> {
    pub(crate) fn parse(self, imported: bool) -> Result<Run<RunId>> {
        self.check_metadata(imported)?;
        let id = RunId::try_from(self.run_id.clone())?;
        Ok(self.with_id(id))
    }

    pub(crate) fn parse_archive(self) -> Result<Run<ArchiveId>> {
        self.check_metadata(true)?;
        let id = ArchiveId::try_from(self.run_id.clone())?;
        Ok(self.with_id(id))
    }

    pub(crate) fn parse_request(self) -> Result<Run<Option<ArchiveId>>> {
        self.check_metadata(true)?;
        let id = if self.run_id.is_empty() {
            None
        } else {
            Some(ArchiveId::try_from(self.run_id.clone())?)
        };
        Ok(self.with_id(id))
    }

    fn check_metadata(&self, imported: bool) -> Result<()> {
        let outcome_valid = matches!(
            self.outcome.as_str(),
            "active" | "suspended" | "victory" | "defeat" | "abandoned"
        ) || imported && self.outcome.is_empty();
        let mut aliases = HashSet::new();
        let mut slots = HashSet::new();
        if self.schema_version != 2
            || self.profile < -1
            || self.started_at < 0
            || self.ended_at < 0
            || !outcome_valid
            || self
                .preserved_run_ids
                .iter()
                .any(|id| id.0 == self.run_id || !aliases.insert(id))
            || self.players.len() > 4
            || self.players.iter().any(|player| {
                player.slot > 3 || player.character.is_empty() || !slots.insert(player.slot)
            })
        {
            return Err("invalid run record".into());
        }
        Ok(())
    }
}

impl<Id> Run<Id> {
    pub(crate) fn id(&self) -> &Id {
        &self.run_id
    }

    fn with_id<NewId>(self, run_id: NewId) -> Run<NewId> {
        Run {
            schema_version: self.schema_version,
            game_version: self.game_version,
            mod_version: self.mod_version,
            run_id,
            preserved_run_ids: self.preserved_run_ids,
            profile: self.profile,
            seed: self.seed,
            started_at: self.started_at,
            ended_at: self.ended_at,
            character: self.character,
            ascension: self.ascension,
            game_mode: self.game_mode,
            outcome: self.outcome,
            players: self.players,
        }
    }

    pub(crate) fn same_identity<OtherId>(&self, other: &Run<OtherId>) -> bool {
        self.profile == other.profile
            && self.seed == other.seed
            && self.started_at == other.started_at
    }

    pub(crate) fn finalized(&self) -> bool {
        matches!(self.outcome.as_str(), "victory" | "defeat" | "abandoned")
    }
}

impl Run<Option<ArchiveId>> {
    pub(crate) fn activate(mut self, id: RunId, prior: Option<Run<RunId>>) -> Run<RunId> {
        self.preserved_run_ids = prior.map_or_else(Vec::new, |run| run.preserved_run_ids);
        self.outcome = "active".into();
        self.ended_at = 0;
        self.with_id(id)
    }
}

impl Run<ArchiveId> {
    pub(crate) fn reidentify(mut self, id: RunId) -> Run<RunId> {
        let canonical = String::from(id);
        if self.run_id.0 != canonical && !self.preserved_run_ids.contains(&self.run_id) {
            self.preserved_run_ids.push(self.run_id.clone());
        }
        self.preserved_run_ids.retain(|alias| alias.0 != canonical);
        self.with_id(id)
    }
}

impl Run<RunId> {
    /// Missing final headers project to unknown metadata without changing the owner.
    pub(crate) fn history_fallback(&self) -> Run<String> {
        let mut view = self.clone().with_id(String::from(self.run_id));
        view.outcome.clear();
        view.ended_at = 0;
        view.players.clear();
        view
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct Record<Owner = Run> {
    schema_version: u32,
    game_version: String,
    mod_version: String,
    run_id: String,
    pub(crate) ordinal: u32,
    pub(crate) run: Option<Owner>,
    pub(crate) combat: Combat,
}

impl Record {
    pub(crate) fn parse(self, imported: bool) -> Result<Record<Run<RunId>>> {
        self.parse_owner(imported, |run| run.parse(imported))
    }

    pub(crate) fn parse_archive(self) -> Result<Record<Run<ArchiveId>>> {
        self.parse_owner(true, Run::parse_archive)
    }

    fn parse_owner<Owner>(
        self,
        imported: bool,
        parse: impl FnOnce(Run) -> Result<Owner>,
    ) -> Result<Record<Owner>> {
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
        } else {
            Row::validate(&self.combat.cards)?;
        }
        Ok(Record {
            schema_version: self.schema_version,
            game_version: self.game_version,
            mod_version: self.mod_version,
            run_id: self.run_id,
            ordinal: self.ordinal,
            run: self.run.map(parse).transpose()?,
            combat: self.combat,
        })
    }
}

impl Record<Run<ArchiveId>> {
    pub(crate) fn reidentify(self, id: RunId) -> Record<Run<RunId>> {
        Record {
            schema_version: self.schema_version,
            game_version: self.game_version,
            mod_version: self.mod_version,
            run_id: String::from(id),
            ordinal: self.ordinal,
            run: self.run.map(|run| run.reidentify(id)),
            combat: self.combat,
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
