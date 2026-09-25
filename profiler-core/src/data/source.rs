//! Combat-owned immutable supplier vectors. A snapshot owns
//! distinct positive weights in first-seen order, reduced by their gcd, with a
//! checked u64 total. Row indices belong only to its nonzero combat epoch.
//! Exported handles retain shared snapshots until the host releases them.
//! Serials never repeat within an epoch; native consumers retain their own Rc.
//! Completed damage groups fix each root's budget before consuming results.
//! Pool prefixes instead retain original weights and a monotone credit cursor;
//! merging a grant or correcting outer residue never changes those weights.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use super::state::{Combat, SourceSlot, State, TEAM_SLOT, caps};

mod snapshot;
mod wire;
use snapshot::{PowerGrant, RootBudgets, SourcePrefix, SourceSnapshot};
use wire::{
    CreatureKind, DamageSegment, GenerationState, ProducerRole, ResultKind, SourceCaptureKind,
};

const KIND_BITS: u32 = 3;
const PAYLOAD_BITS: u32 = 29;
const PAYLOAD_MAX: u32 = (1 << PAYLOAD_BITS) - 1;
const KIND_MASK: u64 = (1 << KIND_BITS) - 1;

const _: () = assert!(KIND_BITS + PAYLOAD_BITS == 32);
const _: () = assert!(caps::COMBAT_CARDS <= PAYLOAD_MAX as usize);
const _: () = assert!(caps::SOURCE_DESTINATIONS == 128);
const _: () = assert!(caps::POWER_GRANTS_TOTAL >= caps::POWER_INSTANCES);
const _: () = assert!(caps::POWER_GRANTS_PER_INSTANCE <= caps::POWER_GRANTS_TOTAL);
const _: () = assert!(caps::DAMAGE_RESULTS == 2);
const _: () = assert!(caps::DAMAGE_DESTINATIONS >= caps::SOURCE_DESTINATIONS);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CombatEpoch(NonZeroU32);

impl CombatEpoch {
    fn from_wire(value: u64) -> Result<Self, SourceFailure> {
        u32::try_from(value)
            .ok()
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or(SourceFailure::Epoch)
    }
}

#[derive(PartialEq)]
#[repr(u8)]
enum TokenKind {
    RowDestination = 0,
    UnknownDestination = 1,
    SourceHandle = 2,
    DamageCalculation = 3,
    CardPlay = 4,
    DoomBatch = 5,
}

const _: () = assert!(TokenKind::RowDestination as u8 == 0);
const _: () = assert!(TokenKind::UnknownDestination as u8 == 1);
const _: () = assert!(TokenKind::SourceHandle as u8 == 2);
const _: () = assert!(TokenKind::DamageCalculation as u8 == 3);
const _: () = assert!(TokenKind::CardPlay as u8 == 4);
const _: () = assert!(TokenKind::DoomBatch as u8 == 5);

struct Token {
    epoch: CombatEpoch,
    kind: TokenKind,
    payload: u32,
}

impl Token {
    fn decode(wire: u64) -> Result<Self, SourceFailure> {
        let epoch = CombatEpoch::from_wire(wire >> 32)?;
        let kind = match wire & KIND_MASK {
            0 => TokenKind::RowDestination,
            1 => TokenKind::UnknownDestination,
            2 => TokenKind::SourceHandle,
            3 => TokenKind::DamageCalculation,
            4 => TokenKind::CardPlay,
            5 => TokenKind::DoomBatch,
            _ => return Err(SourceFailure::Token),
        };
        let payload = (wire as u32) >> KIND_BITS;
        if (kind == TokenKind::UnknownDestination && payload > u32::from(TEAM_SLOT))
            || (matches!(
                kind,
                TokenKind::SourceHandle
                    | TokenKind::DamageCalculation
                    | TokenKind::CardPlay
                    | TokenKind::DoomBatch
            ) && payload == 0)
        {
            return Err(SourceFailure::Token);
        }
        Ok(Self {
            epoch,
            kind,
            payload,
        })
    }

    fn encode(self) -> u64 {
        debug_assert!(
            self.payload <= PAYLOAD_MAX,
            "token payload must fit below the epoch"
        );
        (u64::from(self.epoch.0.get()) << 32)
            | (u64::from(self.payload) << KIND_BITS)
            | self.kind as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Destination {
    Row(u32),
    Unknown(SourceSlot),
}

impl Destination {
    #[cfg(any(test, feature = "test-support"))]
    fn token(self, epoch: CombatEpoch) -> u64 {
        let (kind, payload) = match self {
            Self::Row(row) => (TokenKind::RowDestination, row),
            Self::Unknown(slot) => (TokenKind::UnknownDestination, u32::from(slot)),
        };
        Token {
            epoch,
            kind,
            payload,
        }
        .encode()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SourceFailure {
    Epoch,
    Token,
    Packet,
    Capacity,
    Arithmetic,
}

#[derive(Default)]
struct SourceDiagnostics {
    reported: u8,
    failures: u64,
}

impl SourceDiagnostics {
    fn report(&mut self, failure: SourceFailure) {
        self.failures = self.failures.saturating_add(1);
        let bit = 1 << failure as u8;
        if self.reported & bit == 0 {
            self.reported |= bit;
        }
    }
}

#[derive(Default)]
pub(super) struct SourceArena {
    epoch: Option<CombatEpoch>,
    entries: BTreeMap<u32, SourceSnapshot>,
    index: BTreeMap<SourceSnapshot, u32>,
    serial: u32,
    diagnostics: SourceDiagnostics,
}

impl State {
    fn source_epoch(&mut self, wire: u64) -> Result<CombatEpoch, SourceFailure> {
        if self.poisoned {
            return Err(SourceFailure::Packet);
        }
        let result = CombatEpoch::from_wire(wire).and_then(|epoch| {
            if Combat::active(&self.current).is_some_and(|combat| combat.seq == epoch.0.get()) {
                if self.sources.epoch != Some(epoch) {
                    self.sources = SourceArena {
                        epoch: Some(epoch),
                        ..SourceArena::default()
                    };
                }
                Ok(epoch)
            } else {
                Err(SourceFailure::Epoch)
            }
        });
        if let Err(failure) = result {
            self.sources.diagnostics.report(failure);
        }
        result
    }

    fn source_handle_token(&mut self, wire: u64) -> Result<Token, SourceFailure> {
        let result = Token::decode(wire).and_then(|token| {
            if token.kind != TokenKind::SourceHandle {
                return Err(SourceFailure::Token);
            }
            self.source_epoch(u64::from(token.epoch.0.get()))?;
            if !self.sources.entries.contains_key(&token.payload) {
                return Err(SourceFailure::Token);
            }
            Ok(token)
        });
        if let Err(failure) = result {
            self.sources.diagnostics.report(failure);
        }
        result
    }

    fn source_handle_read(&mut self, wire: u64) -> Result<&SourceSnapshot, SourceFailure> {
        let token = self.source_handle_token(wire)?;
        Ok(self
            .sources
            .entries
            .get(&token.payload)
            .expect("the validated handle owns a live snapshot"))
    }

    fn source_snapshot(
        &mut self,
        epoch: CombatEpoch,
        transfer: u64,
    ) -> Result<SourceSnapshot, SourceFailure> {
        debug_assert_eq!(
            self.provenance.epoch,
            Some(epoch),
            "source requests must use the provenance epoch checked at the event boundary"
        );
        if transfer == 0 {
            return Ok(SourceSnapshot::unknown(epoch));
        }
        self.source_handle_read(transfer).cloned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn source_count(&mut self, transfer: u64) -> i32 {
        self.source_handle_read(transfer)
            .map_or(-1, |snapshot| snapshot.shares().len() as i32)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn source_destination(&mut self, transfer: u64, index: i32) -> u64 {
        let result = self.source_handle_read(transfer).and_then(|snapshot| {
            let share = usize::try_from(index)
                .ok()
                .and_then(|index| snapshot.shares().get(index))
                .ok_or(SourceFailure::Packet)?;
            Ok(share.destination().token(snapshot.epoch()))
        });
        match result {
            Ok(destination) => destination,
            Err(failure) => {
                self.sources.diagnostics.report(failure);
                0
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn source_weight(&mut self, transfer: u64, index: i32) -> u64 {
        let result = self.source_handle_read(transfer).and_then(|snapshot| {
            usize::try_from(index)
                .ok()
                .and_then(|index| snapshot.shares().get(index))
                .map(|share| share.weight())
                .ok_or(SourceFailure::Packet)
        });
        match result {
            Ok(weight) => weight,
            Err(failure) => {
                self.sources.diagnostics.report(failure);
                0
            }
        }
    }
}

mod allocation;
mod damage;
mod play;
mod pools;
mod power;

#[derive(Default)]
pub(super) struct Provenance {
    epoch: Option<CombatEpoch>,
    powers: Vec<PowerProvenance>,
    generated: Vec<GeneratedSource>,
    orbs: Vec<InstanceSource>,
    plays: Vec<ActiveSourcePlay>,
    play_serial: u32,
    calculations: Vec<DamageCalculation>,
    calculation_serial: u32,
    doom_batches: Vec<DoomBatch>,
    doom_serial: u32,
    reductions: Vec<StrengthReduction>,
    pools: Vec<SourcePool>,
}

#[derive(Clone)]
struct PowerProvenance {
    instance: u64,
    id: Box<str>,
    owner: u64,
    owner_kind: CreatureKind,
    owner_slot: SourceSlot,
    observed: i32,
    grants: Vec<PowerGrant>,
    source: SourceSnapshot,
    trusted: bool,
}

struct GeneratedSource {
    instance: u64,
    source: SourceSnapshot,
}

struct InstanceSource {
    instance: u64,
    source: SourceSnapshot,
}

struct ActiveSourcePlay {
    serial: u32,
    execution: u64,
    owner_slot: SourceSlot,
    source: SourceSnapshot,
    first_orb_used: bool,
}

#[derive(Clone)]
struct StrengthReduction {
    power_instance: u64,
    creature: u64,
    amount: u64,
    source: SourceSnapshot,
}

#[derive(Clone, Copy, PartialEq)]
enum ProducerSegment {
    Direct,
    Attributed,
}

impl From<ProducerSegment> for DamageSegment {
    fn from(segment: ProducerSegment) -> Self {
        match segment {
            ProducerSegment::Direct => Self::Direct,
            ProducerSegment::Attributed => Self::Attributed,
        }
    }
}

struct DamageCalculation {
    serial: u32,
    group: Option<DamageGroup>,
}

struct DamageGroup {
    source: SourceSnapshot,
    segment: ProducerSegment,
    modifiers: Vec<allocation::ModifierContribution>,
    results: Vec<ObservedDamage>,
    weak: Option<SourceSnapshot>,
    strength: Vec<(SourceSnapshot, u64)>,
}

struct ObservedDamage {
    total: u64,
    unblocked: u64,
    blocked: u64,
    kind: ResultKind,
    receiver: SourceSlot,
    weak_prevented: u64,
}

struct DoomBatch {
    serial: u32,
    targets: Option<Vec<DoomCapture>>,
}

struct DoomCapture {
    creature: u64,
    power_instance: u64,
    debit: u32,
    allocations: Vec<(Destination, u64)>,
}

#[derive(Clone, Default)]
struct SourcePool {
    blocks: Vec<SourceBlock>,
    pending: Vec<(SourceSnapshot, u64)>,
    osty: Vec<SourceOsty>,
}

#[derive(Clone)]
struct SourceBlock {
    base: SourcePrefix,
    base_original: u64,
    base_consumed: i64,
    remaining: u64,
    mods: Box<[SourceBlockMod]>,
}

impl SourceBlock {
    const MAX_MODS: usize = 4;
}

#[derive(Clone)]
struct SourceBlockMod {
    source: SourcePrefix,
    original: u64,
    consumed: u64,
}

#[derive(Clone)]
struct SourceOsty {
    source: SourcePrefix,
    remaining: u64,
}

const _: () = assert!(caps::PENDING_BLOCK_CONTRIBS >= SourceBlock::MAX_MODS);
const _: () = assert!(caps::MAX_PLAYER_SLOTS == 5);

impl SourceDiagnostics {
    fn slot(&mut self, slot: i32) -> SourceSlot {
        self.clamp(slot, i32::from(TEAM_SLOT)) as SourceSlot
    }
    fn clamp(&mut self, value: i32, maximum: i32) -> i32 {
        let clamped = value.clamp(0, maximum);
        if clamped != value {
            self.report(SourceFailure::Packet);
        }
        clamped
    }
}

impl State {
    fn provenance_epoch(&mut self, wire: u64) -> Result<CombatEpoch, SourceFailure> {
        let epoch = self.source_epoch(wire)?;
        if self.provenance.epoch != Some(epoch) {
            self.provenance = Provenance {
                epoch: Some(epoch),
                ..Provenance::default()
            };
        }
        Ok(epoch)
    }

    fn source_status(&mut self, result: Result<(), SourceFailure>) -> i32 {
        match result {
            Ok(()) => 1,
            Err(failure) => {
                self.sources.diagnostics.report(failure);
                0
            }
        }
    }

    fn provenance_token(&mut self, wire: u64, kind: TokenKind) -> Result<Token, SourceFailure> {
        let token = Token::decode(wire)?;
        if token.kind != kind {
            return Err(SourceFailure::Token);
        }
        self.provenance_epoch(u64::from(token.epoch.0.get()))?;
        Ok(token)
    }

    fn source_export(&mut self, source: SourceSnapshot) -> u64 {
        let arena = &mut self.sources;
        if let Some(&serial) = arena.index.get(&source) {
            return Token {
                epoch: source.epoch(),
                kind: TokenKind::SourceHandle,
                payload: serial,
            }
            .encode();
        }
        if arena.serial == PAYLOAD_MAX {
            arena.diagnostics.report(SourceFailure::Capacity);
            return 0;
        }
        arena.serial += 1;
        let token = Token {
            epoch: source.epoch(),
            kind: TokenKind::SourceHandle,
            payload: arena.serial,
        }
        .encode();
        arena.index.insert(source.clone(), arena.serial);
        arena.entries.insert(arena.serial, source);
        token
    }

    pub(crate) fn source_release(&mut self, handle: u64) -> i32 {
        let Ok(token) = self.source_handle_token(handle) else {
            return 0;
        };
        let source = self
            .sources
            .entries
            .remove(&token.payload)
            .expect("the validated handle owns a live snapshot");
        let serial = self.sources.index.remove(&source);
        debug_assert_eq!(
            serial,
            Some(token.payload),
            "the content index must identify the released handle"
        );
        1
    }

    pub(crate) fn source_accumulate(
        &mut self,
        combat_seq: u64,
        first: u64,
        before: i32,
        second: u64,
        after: i32,
    ) -> u64 {
        let result = (|| {
            let epoch = self.provenance_epoch(combat_seq)?;
            let added = after.checked_sub(before).ok_or(SourceFailure::Arithmetic)?;
            let before = before.max(0) as u32;
            let added = u32::try_from(added).map_err(|_| SourceFailure::Packet)?;
            if before == 0 {
                return Ok(second);
            }
            if added == 0 {
                return Ok(first);
            }
            let first = self.source_snapshot(epoch, first)?;
            let second = self.source_snapshot(epoch, second)?;
            let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
            SourceSnapshot::combine(epoch, rows, first, before, second, added)
                .map(|source| self.source_export(source))
        })();
        match result {
            Ok(source) => source,
            Err(failure) => {
                self.sources.diagnostics.report(failure);
                0
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn source_capture(
        &mut self,
        combat_seq: u64,
        capture_kind: i32,
        instance: u64,
        source_id: &str,
        source_kind: i32,
        source_slot: i32,
        generation_state: i32,
    ) -> u64 {
        let Ok(epoch) = self.provenance_epoch(combat_seq) else {
            return 0;
        };
        let capture = SourceCaptureKind::decode(capture_kind, &mut self.sources.diagnostics);
        let generation = GenerationState::decode(generation_state, &mut self.sources.diagnostics);
        let kind =
            crate::source_kind::SourceKind::from_c(self.sources.diagnostics.clamp(source_kind, 5));
        let slot = self.sources.diagnostics.slot(source_slot);
        let unknown = SourceSnapshot::unknown(epoch);
        let source = match capture {
            SourceCaptureKind::CardInstance => {
                self.card_source(epoch, instance, source_id, slot, generation, None)
            }
            SourceCaptureKind::PowerInstance | SourceCaptureKind::WeakHead => self
                .provenance
                .powers
                .iter()
                .find(|power| power.instance == instance && power.trusted)
                .map(|power| {
                    if capture == SourceCaptureKind::WeakHead {
                        if power.id.as_ref() == "WEAK_POWER" {
                            power
                                .grants
                                .first()
                                .map_or_else(|| unknown.clone(), |grant| grant.source().clone())
                        } else {
                            unknown.clone()
                        }
                    } else if power.id.as_ref() == "POISON_POWER"
                        && !power.grants.iter().any(|grant| grant.remaining() > 0)
                    {
                        unknown.clone()
                    } else {
                        power.source.clone()
                    }
                })
                .unwrap_or(unknown),
            SourceCaptureKind::OrbInstance => self
                .provenance
                .orbs
                .iter()
                .find(|orb| orb.instance == instance)
                .map_or(unknown, |orb| orb.source.clone()),
            SourceCaptureKind::DirectModel
                if matches!(
                    kind,
                    crate::source_kind::SourceKind::Relic | crate::source_kind::SourceKind::Potion
                ) && !source_id.is_empty() =>
            {
                self.named_source(epoch, slot, source_id, kind)
            }
            _ => unknown,
        };
        self.source_export(source)
    }

    fn named_source(
        &mut self,
        epoch: CombatEpoch,
        slot: SourceSlot,
        id: &str,
        kind: crate::source_kind::SourceKind,
    ) -> SourceSnapshot {
        let Some(combat) = Combat::active_mut(&mut self.current) else {
            return SourceSnapshot::unknown(epoch);
        };
        let destination = crate::data::ledger::get_or_create_card_kind(combat, slot, id, kind)
            .map_or(Destination::Unknown(slot), |row| {
                if combat.cards[row].kind == crate::source_kind::SourceKind::Unknown {
                    Destination::Unknown(slot)
                } else {
                    Destination::Row(row as u32)
                }
            });
        SourceSnapshot::normalized(epoch, combat.cards.len(), vec![(destination, 1)])
            .unwrap_or_else(|_| SourceSnapshot::unknown(epoch))
    }
}

#[derive(Clone, Copy)]
enum CreditField {
    BlockGained,
    BlockEffective,
    BlockModifier,
    MitigateWeak,
    MitigateBuff,
    MitigateStrength,
    SelfDamage,
    Forge,
}

struct CombatCounters {
    plays: u32,
    generated_plays: u32,
    generation_triggers: u32,
    damage_received: i64,
    block_total: i64,
    row_capacity_logged: bool,
}

struct LedgerStage<'a> {
    combat: &'a mut Combat,
    pools: &'a mut Vec<SourcePool>,
    diagnostics: &'a mut SourceDiagnostics,
    counters: CombatCounters,
    original_rows: Vec<(usize, super::state::CardStat)>,
    original_pools: Vec<(usize, SourcePool)>,
    row_count: usize,
    pool_count: usize,
    committed: bool,
    capacity_lost: bool,
}

impl<'a> LedgerStage<'a> {
    fn new(state: &'a mut State) -> Result<Self, SourceFailure> {
        let combat = Combat::active_mut(&mut state.current).ok_or(SourceFailure::Epoch)?;
        let counters = CombatCounters {
            plays: combat.plays,
            generated_plays: combat.generated_plays,
            generation_triggers: combat.generation_triggers,
            damage_received: combat.damage_received,
            block_total: combat.block_total,
            row_capacity_logged: combat.row_capacity_logged,
        };
        let row_count = combat.cards.len();
        let pool_count = state.provenance.pools.len();
        Ok(Self {
            combat,
            pools: &mut state.provenance.pools,
            diagnostics: &mut state.sources.diagnostics,
            counters,
            row_count,
            pool_count,
            original_rows: Vec::new(),
            original_pools: Vec::new(),
            committed: false,
            capacity_lost: false,
        })
    }

    fn row(&mut self, destination: Destination) -> Result<usize, SourceFailure> {
        let index = match destination {
            Destination::Row(row) if (row as usize) < self.combat.cards.len() => row as usize,
            Destination::Unknown(slot) if slot <= TEAM_SLOT => {
                crate::data::ledger::get_or_create_card_kind(
                    self.combat,
                    slot,
                    "UNATTRIBUTED",
                    crate::source_kind::SourceKind::Unknown,
                )
                .ok_or(SourceFailure::Capacity)?
            }
            _ => return Err(SourceFailure::Token),
        };
        if index < self.row_count && !self.original_rows.iter().any(|(row, _)| *row == index) {
            self.original_rows
                .push((index, self.combat.cards[index].clone()));
        }
        Ok(index)
    }

    fn credit(
        &mut self,
        destination: Destination,
        field: CreditField,
        amount: i64,
    ) -> Result<(), SourceFailure> {
        if amount == 0 {
            return Ok(());
        }
        let row = self.row(destination)?;
        let row = &mut self.combat.cards[row];
        let value = match field {
            CreditField::BlockGained => &mut row.block_gained,
            CreditField::BlockEffective => &mut row.block_effective,
            CreditField::BlockModifier => &mut row.blk_modifier,
            CreditField::MitigateWeak => &mut row.mitigate_debuff,
            CreditField::MitigateBuff => &mut row.mitigate_buff,
            CreditField::MitigateStrength => &mut row.mitigate_str,
            CreditField::SelfDamage => &mut row.self_damage,
            CreditField::Forge => &mut row.forge,
        };
        *value = value.checked_add(amount).ok_or(SourceFailure::Arithmetic)?;
        Ok(())
    }

    fn row_play(&mut self, destination: Destination) -> Result<(), SourceFailure> {
        let row = self.row(destination)?;
        self.combat.cards[row].plays = self.combat.cards[row]
            .plays
            .checked_add(1)
            .ok_or(SourceFailure::Arithmetic)?;
        Ok(())
    }

    fn source_credit(
        &mut self,
        source: &SourceSnapshot,
        field: CreditField,
        amount: u64,
    ) -> Result<(), SourceFailure> {
        if source.epoch().0.get() != self.combat.seq {
            return Err(SourceFailure::Epoch);
        }
        for (destination, share) in source.budgets(amount).take(amount)? {
            self.credit(
                destination,
                field,
                i64::try_from(share).map_err(|_| SourceFailure::Arithmetic)?,
            )?;
        }
        Ok(())
    }

    fn damage(
        &mut self,
        destination: Destination,
        segment: DamageSegment,
        amount: u64,
        blocked: u64,
    ) -> Result<(), SourceFailure> {
        if blocked > amount {
            return Err(SourceFailure::Packet);
        }
        if amount == 0 {
            return Ok(());
        }
        let amount = i64::try_from(amount).map_err(|_| SourceFailure::Arithmetic)?;
        let blocked = i64::try_from(blocked).map_err(|_| SourceFailure::Arithmetic)?;
        let row = self.row(destination)?;
        let row = &mut self.combat.cards[row];
        row.damage_dealt = row
            .damage_dealt
            .checked_add(amount)
            .ok_or(SourceFailure::Arithmetic)?;
        row.damage_blocked = row
            .damage_blocked
            .checked_add(blocked)
            .ok_or(SourceFailure::Arithmetic)?;
        let value = match segment {
            DamageSegment::Direct => &mut row.dmg_direct,
            DamageSegment::Attributed => &mut row.dmg_attributed,
            DamageSegment::Modifier => &mut row.dmg_modifier,
        };
        *value = value.checked_add(amount).ok_or(SourceFailure::Arithmetic)?;
        debug_assert_eq!(
            i128::from(row.damage_dealt),
            i128::from(row.dmg_direct)
                + i128::from(row.dmg_attributed)
                + i128::from(row.dmg_modifier),
            "damage segments must preserve the row total"
        );
        Ok(())
    }

    fn pool(&mut self, slot: SourceSlot) -> Result<&mut SourcePool, SourceFailure> {
        if slot > TEAM_SLOT {
            return Err(SourceFailure::Packet);
        }
        let slot = usize::from(slot);
        if slot < self.pool_count && !self.original_pools.iter().any(|(index, _)| *index == slot) {
            self.original_pools.push((slot, self.pools[slot].clone()));
        }
        while self.pools.len() <= slot {
            self.pools.push(SourcePool::default());
        }
        Ok(&mut self.pools[slot])
    }

    fn commit(mut self) -> Result<(), SourceFailure> {
        if !crate::data::state::CardStat::arithmetic_representable(&self.combat.cards) {
            return Err(SourceFailure::Arithmetic);
        }
        if self.capacity_lost {
            self.diagnostics.report(SourceFailure::Capacity);
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for LedgerStage<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // The exclusive State borrow hides provisional values. Failure or unwind
        // restores only touched rows/slots and removes appended entries.
        self.combat.cards.truncate(self.row_count);
        for (index, row) in self.original_rows.drain(..) {
            self.combat.cards[index] = row;
        }
        self.pools.truncate(self.pool_count);
        for (index, pool) in self.original_pools.drain(..) {
            self.pools[index] = pool;
        }
        self.combat.plays = self.counters.plays;
        self.combat.generated_plays = self.counters.generated_plays;
        self.combat.generation_triggers = self.counters.generation_triggers;
        self.combat.damage_received = self.counters.damage_received;
        self.combat.block_total = self.counters.block_total;
        self.combat.row_capacity_logged = self.counters.row_capacity_logged;
    }
}

impl State {
    pub(crate) fn turn_started(&mut self, combat_seq: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let combat = Combat::active_mut(&mut self.current).ok_or(SourceFailure::Epoch)?;
            combat.turns = combat
                .turns
                .checked_add(1)
                .ok_or(SourceFailure::Arithmetic)?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn potion_used(&mut self, combat_seq: u64) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let combat = Combat::active_mut(&mut self.current).ok_or(SourceFailure::Epoch)?;
            combat.potions_used = combat
                .potions_used
                .checked_add(1)
                .ok_or(SourceFailure::Arithmetic)?;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn player_died(&mut self, combat_seq: u64, player_slot: i32) -> i32 {
        let result = (|| {
            self.provenance_epoch(combat_seq)?;
            let slot = self.sources.diagnostics.slot(player_slot);
            self.slot_state_mut(i32::from(slot)).died = true;
            Ok(())
        })();
        self.source_status(result)
    }

    pub fn combat_ended(&mut self, combat_seq: u64) -> i32 {
        if self.provenance_epoch(combat_seq).is_err() {
            return 0;
        }
        let Some(combat) = Combat::active_mut(&mut self.current) else {
            return 0;
        };
        let defeated = if combat.player_count == 0 {
            !self.per_player.is_empty()
                && self
                    .per_player
                    .iter()
                    .take(caps::MAX_PLAYERS)
                    .all(|player| player.died)
        } else {
            (0..combat.player_count).all(|slot| {
                self.per_player
                    .get(slot)
                    .is_some_and(|tracked| tracked.died)
            })
        };
        combat.phase = super::state::CombatPhase::Finished(if defeated {
            super::state::CombatResult::Defeat
        } else {
            super::state::CombatResult::Completed
        });
        self.provenance = Provenance::default();
        self.sources.entries.clear();
        self.sources.index.clear();
        1
    }
}

#[cfg(test)]
mod limits;
#[cfg(test)]
mod scenarios;

impl State {
    pub(crate) fn clear_combat_sources(&mut self) {
        self.sources = SourceArena::default();
        self.provenance = Provenance::default();
    }
}

impl State {
    pub(crate) fn discard_combat(&mut self) {
        self.current = None;
        self.clear_combat_sources();
    }
}

impl SourceArena {
    pub(super) fn append_coverage(&self, coverage: &mut super::state::Coverage) {
        if self.diagnostics.failures == 0 {
            return;
        }
        coverage.complete = false;
        coverage.failures = coverage.failures.saturating_add(self.diagnostics.failures);
        for (index, reason) in ["epoch", "source-handle", "packet", "capacity", "arithmetic"]
            .into_iter()
            .enumerate()
        {
            if self.diagnostics.reported & (1 << index) != 0 {
                coverage.add_reason(reason);
            }
        }
    }
}
