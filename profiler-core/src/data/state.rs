//! Live combat/run data and the player-slot model. [`State`] owns this data
//! behind the [`STATE`] thread-local `RefCell`; the game's logic loop is
//! single-threaded.
//!
//! Mutations hold one active `STATE` guard. Helpers given `&State` or
//! `&mut State` must not reborrow `STATE`. Release guards before callbacks
//! or writers that can reenter it; sequential borrows within an event are valid.
//!
//! Fixed-capacity tables are bounded `Vec`s with caps in [`caps`]; overflow
//! is fail-logged, never a silent grow. Cross-table references are indices
//! into the owning `Vec` — the safe-Rust way to reference sibling state
//! without self-borrowing.
//!
//! `State` is the lifetime owner for gameplay storage. The setup entrypoint is
//! [`crate::data::events::init`]. After setup, [`State::slot_index`], combat
//! and run lifecycle transitions, and source event methods are computation
//! scopes. A zero-allocation reset at `clear_combat_sources`,
//! `discard_combat`, run replacement, or a logical slot reset must clear
//! occupancy while retaining capacity. The caps bound cardinality, but do not
//! prove that pushes, clones, or drops avoid allocator calls. Admission
//! failure returns the event's failure value and leaves the state valid; it is
//! not a reason to grow a table or panic.
//!
//! Initialization has one attempt per owner: reservation failure leaves it
//! disabled, and repeated initialization never retries. Logical combat/run
//! presence is independent of retained row and roster buffers. Finish writers
//! lease separate staging buffers and restore them after releasing State guards.
//! Identity limits apply when retaining an identity; unused wire text never
//! replaces trustworthy captured supplier facts. Model IDs, joined rosters,
//! labels, seeds, and network IDs use the byte caps below, reject NUL, and never
//! truncate. Invalid metadata rejects the lifecycle event before any mutation.
//! Model IDs and custom seeds have no hard bound across game/mod inputs, so
//! their caps are profiler policy. The label budget covers the pinned v0.111.0
//! encounter-kind and game-mode enums; network IDs fit decimal u64.
//!
//! # The game facts the model relies on
//!
//! Co-op fully replicates the simulation on every peer: only actions cross
//! the network, in host-fixed order, and each peer hashes its full combat
//! state after every action (a mismatch kicks the client). RNG is identical
//! everywhere (one lobby seed, re-synced at combat start), so the event
//! stream is a pure function of the ordered actions and a peer running the
//! mod attributes every player's contribution locally with zero added
//! network traffic. CombatHistory events and Hook hooks fire from the
//! replicated command layer for all players, and the richer events carry
//! player identity.
//!
//! All living players share one play phase per round (the round ends when
//! every player readies); turn setup is per player, and plays nest within and across
//! players (start(A) … start(B) … finish(B) … finish(A)) as a pausing play
//! yields to queued actions. Death is per player; the combat and run
//! continue until ALL players are dead, and combats always involve the
//! whole team.
//!
//! Identity and per-player state: identity is Player.NetId; the canonical
//! tag is IPlayerCollection.GetPlayerSlotIndex (the index into
//! RunState.Players — lobby join order, host first, max 4), stable for the
//! session. HP and block live on each player's Creature; hand/piles,
//! energy, orbs, and turn number on PlayerCombatState; deck/relics/
//! potions/gold on Player. Osty is strictly per-player: one per player,
//! attacking through the owner's OstyAttack cards and absorbing only the
//! owner's damage pipeline. Powers record only their FIRST applier and
//! stack into one instance per enemy, so the profiler's own FIFO layers
//! (debuff/doom) exist because the game's applier tracking is lossier.
//!
//! Run lifecycle per peer: run-start patches fire on EVERY peer; run save
//! is host-only (resume means the host re-hosts and clients receive the
//! save); RunManager.OnEnded fires on every peer, so every peer closes the
//! run record for every finished run; CleanUp runs per peer, so peers that
//! leave cleanly suspend instead of closing. Abandon is host-only and
//! reaches clients via RunAbandonedMessage.
//!
//! # The player-slot model
//!
//! Team-as-one-player plus per-slot tagging: every player's events feed ONE
//! ledger; combat totals are unfiltered team sums; every ledger row and
//! every source-keyed table carries the owning slot (or [`TEAM_SLOT`] for
//! ownerless sources). A peer with the mod records the whole team even if
//! nobody else runs it — observation only, so the checksums never see the
//! mod.
//!
//! * Slot vocabulary: player slots 0..3 are the lobby join order (single-player is always 0);
//!   [`TEAM_SLOT`] = 4 names rows whose source has no player owner (enemy-power contexts, the
//!   Osty-overflow row). `MAX_PLAYER_SLOTS == 5` is compile-time pinned, and wire slots clamp into
//!   0..=4 via [`clamp_source_slot`]; a TEAM value never fabricates a player entry (that would
//!   poison the team-defeat check).
//! * Row identity is `(slot, id, kind)`; sources keep the supplier's slot independently of the
//!   physical owner or receiver of an effect.
//! * Death flags stay with State; plays and source-bearing defensive pools live in its private
//!   provenance owner.
//! * Team semantics: combat totals (damage_received, plays, block_total, ...) are TEAM totals, and
//!   the turn counter counts ROUNDS (the shim hooks the side-level boundary once per round,
//!   matching the game's RoundNumber). combat_ended marks the record "defeat" iff every roster
//!   slot's died flag is set; without a roster, it uses observed player slots.
//! * Roster: run_started parses slot → net id + character from the two comma-joined ABI lists;
//!   single-player reports the one slot-0 entry even with an empty net_ids. The run record carries
//!   slot + character; the net id stays in-memory.
//! * Player filter: [`State::player_filter`] is All | Player(slot); the header's avatar row toggles
//!   it on both tabs — pressing the active avatar again returns to All. Headline totals stay
//!   team-wide. The run-history screen keeps its own filter, so browsing history never touches live
//!   state.
//!
//! Not solved: peers with different mod builds record slightly different
//! schemas; a peer joining mid-combat after a disconnect rebuilds from full
//! state sync, so the in-memory tables start fresh (the record is marked,
//! not corrupted); timestamps differ per peer (wall-clock, and the
//! simulation never depends on them).

use std::cell::{Cell, RefCell};
use std::collections::TryReserveError;

pub use super::text::Text;

pub type ModelId = Text<{ caps::MODEL_ID_BYTES }>;
pub type RunCharacter = Text<{ caps::RUN_CHARACTER_BYTES }>;
pub type Seed = Text<{ caps::SEED_BYTES }>;
pub type Label = Text<{ caps::LABEL_BYTES }>;
pub type NetId = Text<{ caps::NET_ID_BYTES }>;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fail;
pub use crate::source_kind::SourceKind;
use crate::ui::ui_model::{self, Section, UiRow, UiTab};

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
    caps::RUN_CARDS >= caps::COMBAT_CARDS,
    "the run card table must hold at least one full combat's rows"
);
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
const _: () = assert!(
    core::mem::size_of::<UiRow>() <= 4096,
    "UiRow must stay under 4096 bytes for the panel's frame-buffer memcpy"
);

const _: () = assert!(
    UiTab::Combat as u8 == 0,
    "UiTab::Combat must be discriminant 0"
);
const _: () = assert!(UiTab::Run as u8 == 1, "UiTab::Run must be discriminant 1");

const _: () = assert!(Section::Damage as u8 == 0);
const _: () = assert!(Section::Defense as u8 == 1);
const _: () = assert!(Section::ALL.len() == 2);

const _: () = assert!(ui_model::ROW_FLAG_SELF == 2, "ROW_FLAG_SELF must be bit 1");

const _: () = assert!(
    ui_model::MAX_UI_ROWS >= ui_model::MAX_ROWS_PER_SECTION * Section::ALL.len(),
    "MAX_UI_ROWS must hold MAX_ROWS_PER_SECTION rows for every section"
);

pub type SourceSlot = u8;

/// Ownerless credited sources use TEAM; it never creates a real player.
pub const TEAM_SLOT: SourceSlot = 4;

thread_local! {
    static BAD_SLOT_LOGGED: Cell<bool> = const { Cell::new(false) };
}

/// Unlike [`State::slot_index`] this never grows `per_player`: row-key-only
/// slots have no transient state.
pub fn clamp_source_slot(slot: i32) -> SourceSlot {
    let clamped = slot.clamp(0, TEAM_SLOT as i32) as SourceSlot;
    if clamped as i32 != slot {
        crate::fail_once(
            &BAD_SLOT_LOGGED,
            format_args!("invalid source slot {slot}; clamping to {clamped} (TEAM = {TEAM_SLOT})"),
        );
    }
    clamped
}

/// Lives here because it is state owned by [`State`]; [`ui_model`] stays the
/// dependency-free leaf.
#[derive(Clone, Copy, Debug, PartialEq, Hash, Default)]
pub enum PlayerFilter {
    #[default]
    All,
    Player(u8),
}

impl PlayerFilter {
    /// The avatar row's press: the active slot returns to All, any other
    /// slot selects that player.
    pub fn toggle(self, slot: u8) -> PlayerFilter {
        match self {
            PlayerFilter::Player(s) if s == slot => PlayerFilter::All,
            _ => PlayerFilter::Player(slot),
        }
    }
}

/// Field names and widths define the combat-record JSON schema.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct CardStat {
    /// First so the serialized identity group mirrors this order.
    pub player: SourceSlot,
    pub id: ModelId,
    pub kind: SourceKind,
    /// Own triggers, so `contribution / plays` is the expected value.
    pub plays: u32,
    pub damage_dealt: i64,
    pub damage_blocked: i64,
    pub block_gained: i64,
    /// Modifier-bonus portions land in `blk_modifier` on their own source.
    pub block_effective: i64,
    // The four segments decompose damage_dealt exactly.
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

impl Serialize for CombatResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for CombatResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(match name.as_str() {
            "completed" => CombatResult::Completed,
            "defeat" => CombatResult::Defeat,
            "interrupted" => CombatResult::Interrupted,
            _ => {
                fail!("unknown combat result '{name}'; reading completed");
                CombatResult::Completed
            }
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Hash)]
pub enum CombatPhase {
    #[default]
    Active,
    Finished(CombatResult),
}

#[derive(Default)]
pub struct Combat {
    pub(crate) row_capacity_logged: bool,
    pub seq: u32,
    pub encounter_id: ModelId,
    pub encounter_type: Label,
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
    /// The run identity stamped at combat start; `None` outside a run.
    pub run: Option<RunSnapshot>,
    /// In-memory only.
    pub players: Vec<RunPlayer>,
}

impl Combat {
    /// Liveness for gameplay events: present and not finished. The record
    /// stays in `current` after combat end so the panel keeps showing it,
    /// but its file is already on disk, so events must not mutate it.
    pub fn active(current: &Retained<Combat>) -> Option<&Combat> {
        current
            .as_ref()
            .filter(|combat| combat.phase == CombatPhase::Active)
    }

    pub fn active_mut(current: &mut Retained<Combat>) -> Option<&mut Combat> {
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

/// The run identity a combat was fought under. `None` serializes as the
/// absent run block (see the persistence module doc).
#[derive(Clone)]
pub struct RunSnapshot {
    pub seq: u32,
    pub character: RunCharacter,
    pub ascension: i32,
    pub game_mode: Label,
    pub seed: Seed,
    pub profile: i32,
    /// Original game StartTime in epoch seconds; 0 means unknown.
    pub started_at: i64,
}

impl Default for RunSnapshot {
    fn default() -> Self {
        RunSnapshot {
            seq: 0,
            character: Default::default(),
            // -1 means "the shim never reported an ascension".
            ascension: -1,
            game_mode: Default::default(),
            seed: Default::default(),
            profile: -1,
            started_at: 0,
        }
    }
}

/// The run record carries slot + character; the net id stays in-memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunPlayer {
    pub slot: u8,
    pub net_id: NetId,
    pub character: ModelId,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
pub enum RunOutcome {
    Victory = 0,
    Defeat = 1,
    Abandoned = 2,
}

impl RunOutcome {
    /// Anything outside the wire codes records as defeat.
    pub fn from_c(code: i32) -> RunOutcome {
        match code {
            0 => RunOutcome::Victory,
            1 => RunOutcome::Defeat,
            2 => RunOutcome::Abandoned,
            _ => {
                fail!("invalid run outcome {code}; recording defeat");
                RunOutcome::Defeat
            }
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RunOutcome::Victory => "victory",
            RunOutcome::Defeat => "defeat",
            RunOutcome::Abandoned => "abandoned",
        }
    }
}

impl Serialize for RunOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for RunOutcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(match name.as_str() {
            "victory" => RunOutcome::Victory,
            "abandoned" => RunOutcome::Abandoned,
            _ => RunOutcome::Defeat,
        })
    }
}

const _: () = assert!(
    RunOutcome::Victory as i32 == 0
        && RunOutcome::Defeat as i32 == 1
        && RunOutcome::Abandoned as i32 == 2,
    "run outcome wire codes are the ABI contract"
);

/// Serde `skip_serializing_if` predicate for the zero-omission rule.
pub(crate) fn is_zero<T>(value: &T) -> bool
where
    T: Copy + PartialEq + From<u8>,
{
    *value == 0u8.into()
}

/// The active run's identity and roster.
#[derive(Default)]
pub struct RunContext {
    pub run: RunSnapshot,
    /// Serialized as runs.jsonl's `"players"`.
    pub players: Vec<RunPlayer>,
}

pub struct EndedRun {
    pub context: RunContext,
    pub outcome: RunOutcome,
    /// The abandon moment when the outcome is abandonment.
    pub ended_at: i64,
}

#[derive(Default)]
pub struct PlayerSlotState {
    pub died: bool,
}

pub(crate) struct StorePaths {
    pub(super) runs_dir: PathBuf,
    pub(super) runs_path: PathBuf,
}

impl StorePaths {
    pub(crate) fn new(data_dir: &Path) -> Self {
        Self {
            runs_dir: data_dir.join("runs"),
            runs_path: data_dir.join("runs.jsonl"),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub(crate) store_paths: Option<StorePaths>,
    /// The combat-id counter: seeded at boot to the store's highest id and
    /// incremented at each combat start, so the first new combat takes
    /// max+1. None means the store's highest id could not be established.
    pub next_combat_id: Option<u32>,
    pub current: Retained<Combat>,
    pub(crate) initialization: Initialization,
    pub(crate) finish_combat: Option<Combat>,
    pub(crate) finish_run: Option<EndedRun>,
    pub(crate) self_test_leases: Option<Vec<u64>>,
    pub(super) source_transfers: super::source::SourceTransfers,
    pub(super) provenance: super::source::Provenance,
    /// Run-level accumulator for the Run Summary tab, merged at combat
    /// write and cleared at run start; bounded at [`caps::RUN_CARDS`].
    pub run_cards: Vec<CardStat>,
    pub run_turns: u32,
    pub run_combats: u32,
    /// Profile metadata for the next run (-1 until known).
    pub run_profile: i32,
    pub player_filter: PlayerFilter,

    pub run_ctx: Retained<RunContext>,

    /// Death flags start with the combat roster; observed slots can extend it.
    pub per_player: Vec<PlayerSlotState>,
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
    /// Exact profiler input policy; mod IDs exceeding this byte limit are rejected.
    pub const MODEL_ID_BYTES: usize = 128;
    /// Four bounded model IDs joined with three commas.
    pub const RUN_CHARACTER_BYTES: usize = MAX_PLAYERS * MODEL_ID_BYTES + MAX_PLAYERS - 1;
    /// Profiler input policy for arbitrary custom run seeds.
    pub const SEED_BYTES: usize = 256;
    /// Profiler input policy for encounter kinds and game modes.
    pub const LABEL_BYTES: usize = 32;
    /// Decimal u64 network identity width.
    pub const NET_ID_BYTES: usize = 20;
    /// The synthetic pipeline captures eight simultaneous sources.
    pub const SELF_TEST_LEASES: usize = 8;
    /// Synchronous nested source copy/upload adapters, never suspended operations.
    pub const SOURCE_TRANSFERS: usize = 16;
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
    /// Every creditor slot can reach its independent active-play limit.
    pub const ACTIVE_PLAYS: usize = ACTIVE_PLAYS_PER_SLOT * MAX_PLAYER_SLOTS;
    /// Saved card, turn, and accumulated snapshots in the reviewed power family.
    pub const TEMPORAL_POWER_PENDING: usize = 128;
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
    /// One chunk per distinct block source still holding block in one
    /// slot's pool: same-source chunks merge, blocked damage drains FIFO,
    /// and the slot's turn boundary clears the pool.
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
    /// One run's card-stat rows (the run accumulator, the history
    /// roll-ups): the same id space across every combat of the run, so
    /// strictly more rows than one combat.
    pub const RUN_CARDS: usize = 1024;
}

thread_local! {
    /// The process's profiler state, single-threaded by contract.
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

#[derive(Default)]
pub(crate) enum Initialization {
    #[default]
    Uninitialized,
    Disabled,
    Ready,
}

#[derive(Clone, Copy, Default)]
enum Occupancy {
    #[default]
    Inactive,
    Present,
}

/// Logical absence does not release the lifetime owner's storage.
#[derive(Default)]
pub struct Retained<T> {
    pub(crate) storage: T,
    occupancy: Occupancy,
}

impl<T> Retained<T> {
    pub fn as_ref(&self) -> Option<&T> {
        matches!(self.occupancy, Occupancy::Present).then_some(&self.storage)
    }

    pub fn as_mut(&mut self) -> Option<&mut T> {
        matches!(self.occupancy, Occupancy::Present).then_some(&mut self.storage)
    }

    pub fn is_none(&self) -> bool {
        self.as_ref().is_none()
    }
    pub fn is_some(&self) -> bool {
        self.as_ref().is_some()
    }
}

impl<T: Clone + Default> Retained<T> {
    pub(crate) fn set(&mut self, value: &T) {
        self.storage.clone_from(value);
        self.occupancy = Occupancy::Present;
    }

    pub(crate) fn clear(&mut self) {
        self.occupancy = Occupancy::Inactive;
        self.storage.clone_from(&T::default());
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<T: Default> From<Option<T>> for Retained<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(storage) => Self {
                storage,
                occupancy: Occupancy::Present,
            },
            None => Self::default(),
        }
    }
}

impl Clone for Combat {
    fn clone(&self) -> Self {
        let mut cloned = Self::default();
        cloned.clone_from(self);
        cloned
    }

    fn clone_from(&mut self, source: &Self) {
        self.cards.clone_from(&source.cards);
        self.players.clone_from(&source.players);
        self.row_capacity_logged = source.row_capacity_logged;
        self.seq = source.seq;
        self.encounter_id.clone_from(&source.encounter_id);
        self.encounter_type.clone_from(&source.encounter_type);
        self.started_at = source.started_at;
        self.phase = source.phase;
        self.plays = source.plays;
        self.generated_plays = source.generated_plays;
        self.generation_triggers = source.generation_triggers;
        self.turns = source.turns;
        self.damage_received = source.damage_received;
        self.block_total = source.block_total;
        self.potions_used = source.potions_used;
        self.run.clone_from(&source.run);
    }
}

impl Clone for RunContext {
    fn clone(&self) -> Self {
        let mut cloned = Self::default();
        cloned.clone_from(self);
        cloned
    }

    fn clone_from(&mut self, source: &Self) {
        self.run.clone_from(&source.run);
        self.players.clone_from(&source.players);
    }
}

impl State {
    pub(crate) fn ready(&self) -> bool {
        matches!(self.initialization, Initialization::Ready)
    }

    pub(crate) fn reserve_lifecycle(&mut self) -> Result<(), TryReserveError> {
        self.current
            .storage
            .cards
            .try_reserve_exact(caps::COMBAT_CARDS)?;
        self.current
            .storage
            .players
            .try_reserve_exact(caps::MAX_PLAYERS)?;
        self.run_ctx
            .storage
            .players
            .try_reserve_exact(caps::MAX_PLAYERS)?;
        self.run_cards.try_reserve_exact(caps::RUN_CARDS)?;
        self.per_player.try_reserve_exact(caps::MAX_PLAYER_SLOTS)?;
        #[cfg(any(test, feature = "test-support"))]
        if FAIL_LIFECYCLE_INITIALIZATION.with(|fail| fail.replace(false)) {
            self.run_cards.try_reserve_exact(usize::MAX)?;
        }
        let combat = self.finish_combat.get_or_insert_with(Combat::default);
        combat.cards.try_reserve_exact(caps::COMBAT_CARDS)?;
        combat.players.try_reserve_exact(caps::MAX_PLAYERS)?;
        let run = self.finish_run.get_or_insert_with(|| EndedRun {
            context: RunContext::default(),
            outcome: RunOutcome::Defeat,
            ended_at: 0,
        });
        run.context.players.try_reserve_exact(caps::MAX_PLAYERS)?;
        self.self_test_leases
            .get_or_insert_with(Vec::new)
            .try_reserve_exact(caps::SELF_TEST_LEASES)?;
        self.reserve_sources()?;
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub(crate) static FAIL_LIFECYCLE_INITIALIZATION: Cell<bool> = const { Cell::new(false) };
}

impl State {
    pub(crate) fn replace_run(&mut self, run: &RunSnapshot, players: &[RunPlayer]) -> bool {
        if !self.ready() || players.len() > caps::MAX_PLAYERS {
            return false;
        }
        debug_assert!(
            self.run_ctx.storage.players.capacity() >= caps::MAX_PLAYERS,
            "initialized run storage must retain room for every roster slot"
        );
        self.run_cards.clear();
        self.run_turns = 0;
        self.run_combats = 0;
        self.player_filter = PlayerFilter::All;
        self.run_ctx.set(&RunContext {
            run: run.clone(),
            players: Vec::new(),
        });
        self.run_ctx.storage.players.extend_from_slice(players);
        true
    }
}
