//! Global profiler state: the state types, the file-scope globals, and the
//! player-slot model. All game state lives in one [`State`] behind the
//! [`STATE`] thread-local `RefCell`; the game's logic loop is
//! single-threaded. Fixed-capacity tables are bounded `Vec`s with caps in
//! [`caps`]; overflow is fail-logged, never a silent grow. Cross-table
//! references are indices into the owning `Vec` — the safe-Rust way to
//! reference sibling state without self-borrowing.
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
//! every player readies); turn setup is per player, and plays nest across
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
//! * Rows are `(slot, id, kind)` triples, looked up by `(slot, id)`: [`CardStat::player`] carries
//!   the slot, and the resolution chains key each branch at the resolved source's slot — the
//!   generator's recorded slot for a generated instance's play, the event's slot for an explicit
//!   source (an ally block keys at the owning card's slot), the context entry's own slot for
//!   contexts. Every table keyed by source id carries the slot dimension.
//! * Per-slot transient state: [`State::per_player`] (bounded at MAX_PLAYER_SLOTS, sized on first
//!   slot sight, cleared at the combat boundary) holds one instance of every transient the game
//!   keeps per player. Because plays only nest across players, per-slot play stacks restore the
//!   invariants a single global stack loses.
//! * Team semantics: combat totals (damage_received, plays, block_total, ...) are TEAM totals, and
//!   the turn counter counts ROUNDS (the shim hooks the side-level boundary once per round,
//!   matching the game's RoundNumber). combat_ended marks the record "defeat" iff every
//!   participating slot's died flag is set.
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
use std::path::PathBuf;

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
    "SourceKind::Osty must be the last of the five kinds (discriminant 4)"
);

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
// One block_gained attaches every queued contrib to one chunk.
const _: () = assert!(
    caps::PENDING_BLOCK_CONTRIBS >= BlockEntry::MAX_MODS,
    "the pending-block-contribs queue must hold at least one chunk's MAX_MODS breakdown"
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

/// The shim sends this value only for `context_begin` (enemy-owned powers);
/// the core keys the OSTY overflow row at it directly.
pub const TEAM_SLOT: SourceSlot = 4;

thread_local! {
    static BAD_SLOT_LOGGED: Cell<bool> = const { Cell::new(false) };
    static BAD_MODIFIER_KIND_LOGGED: Cell<bool> = const { Cell::new(false) };
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

/// The modifier contributions' kind codes are the context enum's relic and
/// power values (1 = Relic, 2 = Power); the shim sends nothing else. A
/// modifier credit is never a card, so unknown codes clamp to Power.
pub fn clamp_modifier_kind(kind: i32) -> SourceKind {
    match kind {
        1 => SourceKind::Relic,
        2 => SourceKind::Power,
        _ => {
            crate::fail_once(
                &BAD_MODIFIER_KIND_LOGGED,
                format_args!("invalid modifier kind {kind}; clamping to power"),
            );
            SourceKind::Power
        }
    }
}

/// Lives here because it is state owned by [`State`]; `ui_model` stays the
/// dependency-free leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
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

/// Applications to enemies record FIFO layers for turn-end decrements.
pub const DURATION_DEBUFFS: [&str; 3] = ["VULNERABLE_POWER", "WEAK_POWER", "POISON_POWER"];

/// Field names and widths define the combat-record JSON schema.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CardStat {
    /// First so the serialized identity group mirrors this order.
    pub player: SourceSlot,
    pub id: String,
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

#[derive(Clone, Debug)]
pub struct Combat {
    pub seq: u32,
    pub encounter_id: String,
    pub encounter_type: String,
    pub started_at: i64,
    /// The record stays available for the panel after the fight.
    pub finished: bool,
    pub result: String,
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
    // Run context stamped at combat start.
    pub run_seq: u32,
    pub run_character: String,
    pub run_ascension: i32,
    pub run_game_mode: String,
    /// So a resumed run's fragments re-join by seed.
    pub run_seed: String,
    /// In-memory only.
    pub players: Vec<RunPlayer>,
}

impl Default for Combat {
    fn default() -> Self {
        Combat {
            seq: 0,
            encounter_id: String::new(),
            encounter_type: String::new(),
            started_at: 0,
            finished: false,
            result: "completed".to_owned(),
            cards: Vec::new(),
            plays: 0,
            generated_plays: 0,
            generation_triggers: 0,
            turns: 0,
            damage_received: 0,
            block_total: 0,
            potions_used: 0,
            run_seq: 0,
            run_character: String::new(),
            run_ascension: -1,
            run_game_mode: String::new(),
            run_seed: String::new(),
            players: Vec::new(),
        }
    }
}

/// The run record carries slot + character; the net id stays in-memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunPlayer {
    pub slot: u8,
    pub net_id: String,
    pub character: String,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum RunOutcome {
    Victory = 0,
    #[default]
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

/// Only meaningful while `active`.
#[derive(Clone, Debug)]
pub struct RunContext {
    pub active: bool,
    /// Derived from the store so it never repeats.
    pub seq: u32,
    pub character: String,
    pub ascension: i32,
    pub game_mode: String,
    /// So a resumed run rejoins its earlier fragments.
    pub seed: String,
    /// Falls back to `now_seconds()` when the shim reports no time.
    pub started_at: i64,
    pub ended_at: i64,
    /// `ended_at` is the abandon moment.
    pub outcome: RunOutcome,
    /// Serialized as runs.jsonl's `"players"`.
    pub players: Vec<RunPlayer>,
}

impl Default for RunContext {
    /// -1 means "the shim never reported an ascension".
    fn default() -> Self {
        RunContext {
            active: false,
            seq: 0,
            character: String::new(),
            ascension: -1,
            game_mode: String::new(),
            seed: String::new(),
            started_at: 0,
            ended_at: 0,
            outcome: RunOutcome::Defeat,
            players: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContextEntry {
    pub id: String,
    pub kind: SourceKind,
    /// The context branch keys its row here.
    pub slot: SourceSlot,
}

#[derive(Clone, Debug)]
pub struct OrbSource {
    pub hash: i32,
    pub id: String,
    pub kind: SourceKind,
}

#[derive(Clone, Debug)]
pub struct BlockMod {
    pub id: String,
    pub kind: SourceKind,
    /// The blk_modifier credit row keys at it.
    pub player: SourceSlot,
    pub original: i64,
    pub consumed: i64,
}

/// Consumed FIFO when the player's block absorbs damage.
#[derive(Clone, Debug)]
pub struct BlockEntry {
    pub id: String,
    pub kind: SourceKind,
    /// An ally block keys the credit row at the owning card's slot while
    /// the chunk sits in the receiver's pool.
    pub player: SourceSlot,
    pub base_original: i64,
    pub base_consumed: i64,
    pub remaining: i64,
    pub mods: Vec<BlockMod>,
}

impl BlockEntry {
    pub const MAX_MODS: usize = 4;

    pub fn total_original(&self) -> i64 {
        self.base_original + self.mods.iter().map(|m| m.original).sum::<i64>()
    }
}

/// The credit row keys at the applier's slot, so a cross-player modifier
/// credits its own row.
#[derive(Clone, Debug)]
pub struct PendingContrib {
    pub id: String,
    pub kind: SourceKind,
    pub player: SourceSlot,
    pub amount: i64,
}

/// Plays nest across slots, so each slot owns its transient combat state.
#[derive(Clone, Debug, Default)]
pub struct PlayerSlotState {
    pub active_play_source: Option<(String, SourceKind)>,
    /// The row slot of the play's source: the generator's recorded slot,
    /// else the playing player's own.
    pub active_play_source_slot: SourceSlot,
    /// The playing card's own id, which the chains treat as "the card
    /// being played", not an override.
    pub active_play_card_id: Option<String>,
    /// True once an orb trigger fired during the current play; only the
    /// first trigger credits the channeling source.
    pub orb_first_trigger_used: bool,
    /// How many of this slot's plays are nested; plays interleave across
    /// slots, never within one.
    pub play_depth: u32,
    /// This slot's block pool (bounded at [`caps::BLOCK_POOL`] chunks).
    pub block_pool: Vec<BlockEntry>,
    pub pending_block_contribs: Vec<PendingContrib>,
    /// Queued per-hit damage-modifier contributions, applied to the next
    /// hit this slot's dealer lands.
    pub pending_contribs: Vec<PendingContrib>,
    /// Index into the global [`State::orb_sources`].
    pub orb_fallback: Option<usize>,
    pub potion_fallback: Option<usize>,
    /// This slot's Osty defensive HP stack; absorbed damage consumes LIFO.
    pub osty_stack: Vec<OstyEntry>,
    /// True once the slot's creature died; the team record is "defeat" iff
    /// every participating slot's flag is set.
    pub died: bool,
}

/// Which source applied each power (and how much); proportional splits use
/// the recorded amounts.
#[derive(Clone, Debug)]
pub struct PowerSourceEntry {
    pub power_id: String,
    pub source_id: String,
    pub kind: SourceKind,
    /// The applier's slot; part of the record's identity.
    pub player: SourceSlot,
    pub amount: i64,
}

#[derive(Clone, Debug)]
pub struct GeneratedInstance {
    pub hash: i32,
    pub source_id: String,
    pub kind: SourceKind,
    /// The creator's slot; the instance's later play keys its row here.
    pub player: SourceSlot,
}

/// One recorded Doom application on an enemy; kill damage (the enemy's
/// current HP) attributes FIFO across the applications.
#[derive(Clone, Debug)]
pub struct DoomLayer {
    pub creature_hash: u64,
    pub source_id: String,
    pub kind: SourceKind,
    /// The applier's slot; the DoomKill credit row keys at it.
    pub player: SourceSlot,
    pub amount: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct DoomTarget {
    pub creature_hash: u64,
    pub hp: i64,
}

/// Osty defensive HP pool: each Summon pushes an entry; absorbed damage
/// consumes LIFO, crediting block_effective to the summoning sources.
#[derive(Clone, Debug)]
pub struct OstyEntry {
    pub id: String,
    pub kind: SourceKind,
    /// The summoning source's row slot; the absorb credit keys at it.
    pub player: SourceSlot,
    pub remaining: i64,
}

/// Enemy Strength reductions, per creature. Prevented damage credits the
/// reducer's mitigate_str; a positive delta consumes reductions LIFO.
#[derive(Clone, Debug)]
pub struct StrReduction {
    pub creature_hash: u64,
    pub source_id: String,
    pub kind: SourceKind,
    /// The reducer's row slot; the mitigation credit keys at it.
    pub player: SourceSlot,
    pub amount: i64,
}

/// FIFO debuff layers per (creature, power); turn-end decrements consume
/// from the head, and poison tick damage splits by duration fraction.
#[derive(Clone, Debug)]
pub struct DebuffLayer {
    pub creature_hash: u64,
    pub power_id: String,
    pub source_id: String,
    pub kind: SourceKind,
    /// The applier's slot; tick and mitigation credit rows key at it.
    pub player: SourceSlot,
    pub duration: i64,
}

/// One captured enemy→player hit: base damage and the dealer's Strength
/// at ModifyDamage time, used to prorate str-reduction mitigation.
#[derive(Clone, Copy, Debug)]
pub struct EnemyHit {
    pub base: i64,
    pub str: i64,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub initialized: bool,
    pub data_dir: PathBuf,
    pub runs_dir_full: PathBuf,
    pub runs_path_full: PathBuf,
    /// The combat-id counter: seeded at boot to the store's highest id and
    /// incremented at each combat start, so the first new combat takes
    /// max+1.
    pub next_combat_id: u32,
    pub current: Option<Combat>,
    /// Run-level accumulator for the Run Summary tab, merged at combat
    /// write and cleared at run start; bounded at [`caps::RUN_CARDS`].
    pub run_cards: Vec<CardStat>,
    pub run_turns: u32,
    pub run_combats: u32,
    pub run_seq_accumulated: u32,
    /// The session's profile id (-1 until known); run-history matching
    /// filters on it so profiles never mix.
    pub run_profile: i32,
    pub player_filter: PlayerFilter,

    pub run_ctx: RunContext,

    /// Per-slot transient state; sized on first slot sight and cleared at
    /// the combat boundary.
    pub per_player: Vec<PlayerSlotState>,

    pub context_stack: Vec<ContextEntry>,
    /// Most recent attribution source, remembered across async gaps so
    /// effects firing after a hook's pop still resolve to their cause.
    pub last_source: Option<ContextEntry>,

    /// Channeling source per orb hash; the orb fallback indexes this table.
    pub orb_sources: Vec<OrbSource>,

    pub power_sources: Vec<PowerSourceEntry>,

    pub generated_instances: Vec<GeneratedInstance>,

    pub doom_layers: Vec<DoomLayer>,
    pub doom_targets: Vec<DoomTarget>,

    pub str_reductions: Vec<StrReduction>,

    /// A stale capture at worst mis-prorates one hit; it never accumulates.
    pub enemy_hit: Option<EnemyHit>,

    pub debuff_layers: Vec<DebuffLayer>,
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

    /// The slot whose play is currently attributing; the earlier slot
    /// wins when plays interleave.
    pub fn ambient_slot(&self) -> usize {
        self.per_player
            .iter()
            .position(|slot| slot.play_depth > 0)
            .unwrap_or(0)
    }
}

pub mod caps {
    /// Open hook contexts at one instant: each relic/power hook's begin
    /// push pairs with an end pop, so the cap bounds how deep hooks nest
    /// into each other, not the combat's hook count.
    pub const CONTEXT_STACK: usize = 32;
    /// Channeling sources keyed by orb hash (a re-channel upserts) plus
    /// two hash-0 potion entries per use; nothing leaves the table before
    /// the combat boundary, so whole-combat potion uses drive the worst
    /// case.
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
    /// One entry per (power, source, slot) applier trio: repeat
    /// applications merge into an existing entry and the combat boundary
    /// clears the table, so it holds one combat's distinct appliers per
    /// power.
    pub const POWER_SOURCES: usize = 128;
    /// One entry per generated card instance hash, updated in place when
    /// the same instance regenerates and cleared only at the combat
    /// boundary, so it grows with one combat's distinct generated copies.
    pub const GENERATED_INSTANCES: usize = 64;
    /// One layer per recorded Doom application on an enemy, drained FIFO
    /// at that creature's Doom kill (depleted layers leave) and cleared at
    /// the combat boundary, so it holds applications still awaiting a
    /// kill.
    pub const DOOM_LAYERS: usize = 64;
    /// One capture per living doomed creature in a single DoomKill batch;
    /// the postfix drains the whole table, so the cap sizes one kill
    /// batch, never a lifetime count.
    pub const DOOM_TARGETS: usize = 16;
    /// One entry per Osty summon on the owner's slot, popped as absorbed
    /// damage depletes it and cleared when the Osty dies, so it holds only
    /// summons with unabsorbed HP.
    pub const OSTY_STACK: usize = 32;
    /// Modifier shares awaiting the dealer's next landed hit, one per
    /// recorded applier per modifier event; the hit carves them out of its
    /// damage and an unlanded one drops them, so the queue never spans
    /// hits.
    pub const PENDING_CONTRIBS: usize = 16;
    /// One entry per (creature, reducer source, slot) trio, merged on
    /// repeat and consumed LIFO when the enemy's Strength rises again, so
    /// it holds each creature's reductions still standing.
    pub const STR_REDUCTIONS: usize = 64;
    /// One layer per duration-debuff application on an enemy (vulnerable,
    /// weak, poison): turn-end decrements consume the layers FIFO and drop
    /// depleted ones, and poison ticks split by duration fraction.
    pub const DEBUFF_LAYERS: usize = 64;
    /// One combat's distinct (player, id, kind) rows: four slots' deck ids
    /// (upgraded variants included) plus the relic/power/potion catalogs —
    /// a few hundred in the worst real combat.
    pub const COMBAT_CARDS: usize = 512;
    /// One run's card-stat rows (the run accumulator, the history
    /// roll-ups): the same id space across every combat of the run, so
    /// strictly more rows than one combat.
    pub const RUN_CARDS: usize = 1024;
}

thread_local! {
    /// The process's profiler state, single-threaded by contract.
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_c_clamps_every_input_to_a_catalogued_kind() {
        assert_eq!(SourceKind::from_c(i32::MIN), SourceKind::Card);
        assert_eq!(SourceKind::from_c(-1), SourceKind::Card);
        assert_eq!(SourceKind::from_c(0), SourceKind::Card);
        assert_eq!(SourceKind::from_c(1), SourceKind::Relic);
        assert_eq!(SourceKind::from_c(2), SourceKind::Power);
        // Potion and Osty kinds from the host clamp to Power (the shim only
        // sends 0/1/2).
        assert_eq!(SourceKind::from_c(3), SourceKind::Power);
        assert_eq!(SourceKind::from_c(4), SourceKind::Power);
        assert_eq!(SourceKind::from_c(i32::MAX), SourceKind::Power);
        for kind in -64..=64 {
            let k = SourceKind::from_c(kind);
            debug_assert!(
                matches!(k, SourceKind::Card | SourceKind::Relic | SourceKind::Power),
                "from_c({kind}) must clamp to a catalogued kind"
            );
        }
    }

    #[test]
    fn modifier_kind_codes_map_power_and_relic_and_clamp_unknowns() {
        assert_eq!(clamp_modifier_kind(1), SourceKind::Relic);
        assert_eq!(clamp_modifier_kind(2), SourceKind::Power);
        assert_eq!(clamp_modifier_kind(0), SourceKind::Power);
        assert_eq!(clamp_modifier_kind(-1), SourceKind::Power);
        assert_eq!(clamp_modifier_kind(i32::MAX), SourceKind::Power);
    }

    #[test]
    fn from_c_maps_wire_codes_and_defaults_unknowns_to_defeat() {
        assert_eq!(RunOutcome::from_c(0), RunOutcome::Victory);
        assert_eq!(RunOutcome::from_c(1), RunOutcome::Defeat);
        assert_eq!(RunOutcome::from_c(2), RunOutcome::Abandoned);
        assert_eq!(RunOutcome::from_c(-1), RunOutcome::Defeat);
        assert_eq!(RunOutcome::from_c(3), RunOutcome::Defeat);
    }

    #[test]
    fn outcome_serde_round_trips_lowercase_and_reads_unknowns_as_defeat() {
        assert_eq!(
            serde_json::to_string(&RunOutcome::Victory).expect("victory serializes"),
            "\"victory\""
        );
        let out: RunOutcome =
            serde_json::from_str("\"bogus\"").expect("unknown outcome string decodes");
        assert_eq!(out, RunOutcome::Defeat);
    }
}
