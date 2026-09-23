//! A State owns one combat ledger and its provenance. No live gameplay data is
//! global; the ABI borrows the engine exclusively for each synchronous call.
//! Player slots 0..3 follow lobby order; TEAM=4 credits ownerless sources and
//! never contributes a player to defeat detection. Host-supplied epochs must
//! increase across combats so late observations cannot reach a new ledger.

#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;

use serde::Serialize;

pub use crate::source_kind::SourceKind;

// Pinned at compile time: a refactor that breaks a wire value, width, or
// range fails the build instead of corrupting a schema.

const _: () = assert!(
    SourceKind::Card as u8 == 0,
    "SourceKind::Card must be discriminant 0"
);
const _: () = assert!(
    SourceKind::Relic as u8 == 1,
    "SourceKind::Relic must be discriminant 1"
);
const _: () = assert!(
    SourceKind::Power as u8 == 2,
    "SourceKind::Power must be discriminant 2"
);
const _: () = assert!(
    SourceKind::Potion as u8 == 3,
    "SourceKind::Potion must be discriminant 3"
);
const _: () = assert!(
    SourceKind::Osty as u8 == 4,
    "SourceKind::Osty must retain discriminant 4"
);

const _: () = assert!(SourceKind::Unknown as u8 == 5);

const _: () = assert!(
    caps::MAX_PLAYERS == 4,
    "caps::MAX_PLAYERS must stay 4: the game's lobby cap bounds the player slots"
);
const _: () = assert!(
    caps::MAX_PLAYER_SLOTS == 5,
    "caps::MAX_PLAYER_SLOTS must stay 5: the four player slots plus the TEAM slot"
);
const _: () = assert!(
    caps::MAX_PLAYER_SLOTS == TEAM_SLOT as usize + 1,
    "MAX_PLAYER_SLOTS must cover the TEAM slot as its highest value"
);
pub type SourceSlot = u8;

/// Ownerless credited sources use TEAM; it never creates a real player.
pub const TEAM_SLOT: SourceSlot = 4;

pub fn clamp_source_slot(slot: i32) -> SourceSlot {
    slot.clamp(0, TEAM_SLOT as i32) as SourceSlot
}

/// Fields and widths define source rows in the engine summary.
#[derive(Clone, Debug, Default, PartialEq, Hash, Serialize)]
pub struct CardStat {
    /// First so the serialized identity group mirrors this order.
    pub player: SourceSlot,
    pub id: Box<str>,
    pub kind: SourceKind,
    /// Own triggers, so `contribution / plays` is the expected value.
    pub plays: u32,
    pub damage_dealt: i64,
    pub damage_blocked: i64,
    pub block_gained: i64,
    /// Modifier-bonus portions land in `blk_modifier` on their own source.
    pub block_effective: i64,
    // The three segments decompose damage_dealt exactly.
    pub dmg_direct: i64,
    /// Indirect damage: poison ticks, orb triggers, doom kills.
    pub dmg_attributed: i64,
    pub dmg_modifier: i64,
    /// Credited only when the block actually absorbs damage.
    pub blk_modifier: i64,
    pub mitigate_debuff: i64,
    pub mitigate_buff: i64,
    pub mitigate_str: i64,
    pub self_damage: i64,
    pub forge: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Hash)]
pub enum CombatResult {
    #[default]
    Completed,
    Defeat,
    Interrupted,
}

impl CombatResult {
    pub(crate) fn name(self) -> &'static str {
        match self {
            CombatResult::Completed => "completed",
            CombatResult::Defeat => "defeat",
            CombatResult::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Hash)]
pub enum CombatPhase {
    #[default]
    Active,
    Finished(CombatResult),
}

#[derive(Clone, Default)]
pub struct Combat {
    pub(crate) row_capacity_logged: bool,
    pub seq: u32,
    pub encounter_id: Box<str>,
    pub encounter_type: Box<str>,
    pub started_at: i64,
    /// The record stays available for the panel after the fight.
    pub phase: CombatPhase,
    /// Bounded at [`caps::COMBAT_CARDS`].
    pub cards: Vec<CardStat>,
    pub plays: u32,
    /// Book the combat total but no row's `plays`.
    pub generated_plays: u32,
    /// The identity is `plays + generation_triggers == Σ rows + generated_plays`.
    pub generation_triggers: u32,
    pub turns: u32,
    pub damage_received: i64,
    pub block_total: i64,
    pub potions_used: u32,
    pub player_count: usize,
}

impl Combat {
    /// Finished ledgers remain available for snapshots but reject observations.
    pub fn active(current: &Option<Combat>) -> Option<&Combat> {
        current
            .as_ref()
            .filter(|combat| combat.phase == CombatPhase::Active)
    }

    pub fn active_mut(current: &mut Option<Combat>) -> Option<&mut Combat> {
        current
            .as_mut()
            .filter(|combat| combat.phase == CombatPhase::Active)
    }

    pub fn result(&self) -> Option<CombatResult> {
        match self.phase {
            CombatPhase::Active => None,
            CombatPhase::Finished(result) => Some(result),
        }
    }
}

#[derive(Default)]
pub struct PlayerSlotState {
    pub died: bool,
}

#[derive(Default)]
pub struct State {
    pub current: Option<Combat>,
    pub(super) sources: super::source::SourceArena,
    pub(super) provenance: super::source::Provenance,
    pub per_player: Vec<PlayerSlotState>,
    pub(crate) revision: u64,
    pub(crate) last_combat_seq: u32,
    pub(crate) coverage: Coverage,
    pub(crate) poisoned: bool,
    pub(crate) recording: Option<super::observation::Recording>,
}

#[derive(Clone, Default, Serialize)]
pub(crate) struct Coverage {
    pub complete: bool,
    pub failures: u64,
    pub reasons: Vec<Box<str>>,
}

impl State {
    /// The TEAM slot maps to a fifth entry, so a corrupt wire slot can
    /// never index out of bounds or fabricate a player.
    pub fn slot_index(&mut self, slot: i32) -> usize {
        let index = clamp_source_slot(slot) as usize;
        while self.per_player.len() <= index {
            self.per_player.push(PlayerSlotState::default());
        }
        index
    }

    pub fn slot_state_mut(&mut self, slot: i32) -> &mut PlayerSlotState {
        let index = self.slot_index(slot);
        &mut self.per_player[index]
    }
}

pub mod caps {
    /// Distinct failure categories, with overflow represented by a fixed marker.
    pub const COVERAGE_REASONS: usize = 32;
    /// Distinct credited roots inherited by one effect.
    pub const SOURCE_DESTINATIONS: usize = 128;
    /// Simultaneously attached actual powers across players and enemies.
    pub const POWER_INSTANCES: usize = 256;
    /// Application layers retained by one attached power.
    pub const POWER_GRANTS_PER_INSTANCE: usize = 64;
    /// Live application layers, including one reserved Unknown per power.
    pub const POWER_GRANTS_TOTAL: usize = 512;
    /// Nested and replayed card frames on one physical player.
    pub const ACTIVE_PLAYS_PER_SLOT: usize = 32;
    /// Suspended live target calculations across independent commands.
    pub const DAMAGE_CALCULATIONS: usize = 64;
    /// The canonical damage target yields at most two redirected results.
    pub const DAMAGE_RESULTS: usize = 2;
    /// Actual modifying events, with supplier fanout inside each event.
    pub const DAMAGE_MODIFIERS: usize = 64;
    /// Distinct credited roots in a completed damage calculation.
    pub const DAMAGE_DESTINATIONS: usize = 128;
    /// Nested synchronous Doom command kickoffs.
    pub const DOOM_BATCHES: usize = 16;
    /// Channeling sources keyed by actual orb identity; a re-channel upserts and
    /// nothing leaves the table before the combat boundary.
    pub const ORB_SOURCES: usize = 32;
    /// The game's lobby cap; per-player state never needs a fifth PLAYER.
    pub const MAX_PLAYERS: usize = 4;
    /// The four player slots plus the TEAM slot, so a corrupt wire slot
    /// can never index out of bounds.
    pub const MAX_PLAYER_SLOTS: usize = 5;
    /// Unconsumed block chunks per slot; unmodified equal-source gains merge.
    /// Damage drains FIFO; an actual block clear resets the pool, while
    /// retained block keeps its sources.
    pub const BLOCK_POOL: usize = 64;
    /// Modifier shares awaiting the next block gain, one per recorded
    /// applier per modifier event; that gain attaches the queue to one
    /// chunk (at most a chunk's `MAX_MODS` slices) and clears it.
    pub const PENDING_BLOCK_CONTRIBS: usize = 16;
    /// One entry per actual generated card identity, updated in place when
    /// the same instance regenerates and cleared only at the combat
    /// boundary, so it grows with one combat's distinct generated copies.
    pub const GENERATED_INSTANCES: usize = 64;
    /// One capture per living doomed creature in a single DoomKill batch;
    /// the postfix drains the whole table, so the cap sizes one kill
    /// batch, never a lifetime count.
    pub const DOOM_TARGETS: usize = 16;
    /// One entry per Osty summon on the owner's slot, popped as absorbed
    /// damage depletes it and cleared when the Osty dies, so it holds only
    /// summons with unabsorbed HP.
    pub const OSTY_STACK: usize = 32;
    /// One entry per (creature, reducer source, slot) trio, merged on
    /// repeat and consumed LIFO when the enemy's Strength rises again, so
    /// it holds each creature's reductions still standing.
    pub const STR_REDUCTIONS: usize = 64;
    /// One combat's distinct (player, id, kind) rows: four slots' deck ids
    /// (upgraded variants included) plus the relic/power/potion catalogs —
    /// a few hundred in the worst real combat.
    pub const COMBAT_CARDS: usize = 512;
    /// Every creditor slot retains an Unknown fallback inside each row table.
    pub const UNKNOWN_ROWS: usize = MAX_PLAYER_SLOTS;
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Isolated fixture adapter; production engine state is explicitly owned.
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}
