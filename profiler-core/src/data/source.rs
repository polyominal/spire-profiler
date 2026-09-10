//! Immutable supplier vectors and synchronous transfer leases. A snapshot owns
//! distinct positive weights in first-seen order, reduced by their gcd, with a
//! checked u64 total. Row indices belong only to its nonzero combat epoch.
//! Completed damage groups fix each root's budget before consuming results.
//! Pool prefixes instead retain original weights and a monotone credit cursor;
//! merging a grant or correcting outer residue never changes those weights.
//!
//! Provenance capture, source normalization, power and play tracking, damage
//! allocation, block pools, Doom batches, and Osty stacks are computation
//! scopes. The target requires their storage to be bounded and retained by
//! `State`; cap, token, epoch, and arithmetic failures are measured just like
//! successful operations. Combat-epoch invalidation and
//! `clear_combat_sources` are logical reset boundaries, so a qualifying reset
//! preserves retained capacity. A diagnostic report does not hide allocation
//! performed by the computation that produced it.
//!
//! Initialization reserves transfer uploads, immutable snapshots, tracking
//! tables, and calculation captures. Slots retain their buffers behind a live
//! prefix; removal preserves live order and recycles the removed owner. The
//! flat grant table names its owning power by index, adjusted when powers move.
//! Suspended calculations copy supplier facts into their own reserved slots,
//! so grant mutation and removal cannot change an earlier capture.
//!
//! Root-budget outputs, the owned prefix-credit adapter, grouped attribution,
//! ledger transaction clones, and Doom capture outputs allocate. Clearing
//! populated Doom batches releases their nested output buffers.

use std::collections::TryReserveError;
use std::num::NonZeroU32;

use super::state::{Combat, SourceSlot, State, TEAM_SLOT, caps};

mod snapshot;
mod storage;
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
const _: () = assert!(caps::ACTIVE_PLAYS <= PAYLOAD_MAX as usize);
const _: () = assert!(caps::SOURCE_DESTINATIONS == 128);
const _: () = assert!(caps::SOURCE_TRANSFERS == 16);
const _: () = assert!(caps::POWER_GRANTS_TOTAL >= caps::POWER_INSTANCES);
const _: () = assert!(caps::POWER_GRANTS_PER_INSTANCE <= caps::POWER_GRANTS_TOTAL);
const _: () = assert!(caps::DAMAGE_RESULTS == 2);
const _: () = assert!(caps::DAMAGE_DESTINATIONS >= caps::SOURCE_DESTINATIONS);

#[derive(Clone, Copy, PartialEq)]
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
    SourceTransfer = 2,
    DamageCalculation = 3,
    CardPlay = 4,
    DoomBatch = 5,
}

const _: () = assert!(TokenKind::RowDestination as u8 == 0);
const _: () = assert!(TokenKind::UnknownDestination as u8 == 1);
const _: () = assert!(TokenKind::SourceTransfer as u8 == 2);
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
            2 => TokenKind::SourceTransfer,
            3 => TokenKind::DamageCalculation,
            4 => TokenKind::CardPlay,
            5 => TokenKind::DoomBatch,
            _ => return Err(SourceFailure::Token),
        };
        let payload = (wire as u32) >> KIND_BITS;
        if (kind == TokenKind::UnknownDestination && payload > u32::from(TEAM_SLOT))
            || (matches!(
                kind,
                TokenKind::SourceTransfer
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Destination {
    Row(usize),
    Unknown(SourceSlot),
}

impl Destination {
    fn from_token(wire: u64, epoch: CombatEpoch, rows: usize) -> Result<Self, SourceFailure> {
        let token = Token::decode(wire)?;
        if token.epoch != epoch {
            return Err(SourceFailure::Epoch);
        }
        match token.kind {
            TokenKind::RowDestination if (token.payload as usize) < rows => {
                Ok(Self::Row(token.payload as usize))
            }
            TokenKind::UnknownDestination => Ok(Self::Unknown(token.payload as SourceSlot)),
            _ => Err(SourceFailure::Token),
        }
    }

    fn token(self, epoch: CombatEpoch) -> u64 {
        let (kind, payload) = match self {
            Self::Row(row) => (TokenKind::RowDestination, row as u32),
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
}

impl SourceDiagnostics {
    fn report(&mut self, failure: SourceFailure) {
        let bit = 1 << failure as u8;
        if self.reported & bit == 0 {
            self.reported |= bit;
            crate::fail!("source provenance rejected: {failure:?}");
        }
    }
}

enum TransferValue {
    Upload,
    Sealed,
    Invalid,
}

struct Transfer {
    serial: u32,
    value: TransferValue,
    upload: Vec<(Destination, u128)>,
    source: SourceSnapshot,
}

#[derive(Default)]
pub(super) struct SourceTransfers {
    epoch: Option<CombatEpoch>,
    serial: u32,
    entries: storage::Slots<Transfer>,
    scratch: snapshot::SnapshotScratch,
    diagnostics: SourceDiagnostics,
}

impl SourceTransfers {
    fn reserve(&mut self) -> Result<(), TryReserveError> {
        self.entries.try_init(caps::SOURCE_TRANSFERS, || {
            let mut upload = Vec::new();
            upload.try_reserve_exact(caps::SOURCE_DESTINATIONS)?;
            Ok(Transfer {
                serial: 0,
                value: TransferValue::Invalid,
                upload,
                source: SourceSnapshot::try_new()?,
            })
        })?;
        self.scratch = snapshot::SnapshotScratch::try_new()?;
        Ok(())
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.epoch = None;
        self.serial = 0;
        self.diagnostics = SourceDiagnostics::default();
    }

    fn read_index(&self, epoch: CombatEpoch, wire: u64) -> Result<usize, SourceFailure> {
        let token = Token::decode(wire)?;
        if token.kind != TokenKind::SourceTransfer {
            return Err(SourceFailure::Token);
        }
        if token.epoch != epoch || self.epoch != Some(epoch) {
            return Err(SourceFailure::Epoch);
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.serial == token.payload)
            .ok_or(SourceFailure::Token)?;
        let entry = &self.entries[index];
        if !matches!(entry.value, TransferValue::Sealed) {
            return Err(SourceFailure::Token);
        }
        Ok(index)
    }

    fn snapshot_into(
        &self,
        epoch: CombatEpoch,
        transfer: u64,
        destination: &mut SourceSnapshot,
    ) -> Result<(), SourceFailure> {
        if transfer == 0 {
            destination.set_unknown(epoch);
        } else {
            let index = self.read_index(epoch, transfer)?;
            destination.clone_from(&self.entries[index].source);
        }
        Ok(())
    }
}

impl State {
    fn source_epoch(&mut self, wire: u64) -> Result<CombatEpoch, SourceFailure> {
        let result = CombatEpoch::from_wire(wire).and_then(|epoch| {
            if Combat::active(&self.current).is_some_and(|combat| combat.seq == epoch.0.get()) {
                if self.source_transfers.epoch != Some(epoch) {
                    self.source_transfers.clear();
                    self.source_transfers.epoch = Some(epoch);
                }
                Ok(epoch)
            } else {
                Err(SourceFailure::Epoch)
            }
        });
        if let Err(failure) = result {
            self.source_transfers.diagnostics.report(failure);
        }
        result
    }

    fn source_transfer_token(&mut self, wire: u64) -> Result<Token, SourceFailure> {
        let result = Token::decode(wire).and_then(|token| {
            if token.kind != TokenKind::SourceTransfer {
                return Err(SourceFailure::Token);
            }
            self.source_epoch(u64::from(token.epoch.0.get()))?;
            if token.payload > self.source_transfers.serial {
                return Err(SourceFailure::Token);
            }
            Ok(token)
        });
        if let Err(failure) = result {
            self.source_transfers.diagnostics.report(failure);
        }
        result
    }

    fn source_transfer_read(&mut self, wire: u64) -> Result<&SourceSnapshot, SourceFailure> {
        let token = self.source_transfer_token(wire)?;
        match self.source_transfers.read_index(token.epoch, wire) {
            Ok(index) => Ok(&self.source_transfers.entries[index].source),
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
                Err(failure)
            }
        }
    }

    #[cfg(test)]
    fn source_snapshot(
        &mut self,
        combat_seq: u64,
        transfer: u64,
    ) -> Result<SourceSnapshot, SourceFailure> {
        self.capture_source_snapshot(combat_seq, transfer)?;
        Ok(self.provenance.incoming.clone())
    }

    fn capture_source_snapshot(
        &mut self,
        combat_seq: u64,
        transfer: u64,
    ) -> Result<(), SourceFailure> {
        let epoch = self.provenance_epoch(combat_seq)?;
        self.source_transfers
            .snapshot_into(epoch, transfer, &mut self.provenance.incoming)
    }

    pub(crate) fn source_transfer_begin(&mut self, combat_seq: u64) -> u64 {
        let Ok(epoch) = self.source_epoch(combat_seq) else {
            return 0;
        };
        let transfers = &mut self.source_transfers;
        if transfers.entries.len() == caps::SOURCE_TRANSFERS || transfers.serial == PAYLOAD_MAX {
            transfers.diagnostics.report(SourceFailure::Capacity);
            return 0;
        }
        let serial = transfers.serial + 1;
        let Some(entry) = transfers.entries.vacant_mut() else {
            transfers.diagnostics.report(SourceFailure::Capacity);
            return 0;
        };
        entry.serial = serial;
        entry.value = TransferValue::Upload;
        entry.upload.clear();
        transfers.entries.activate();
        transfers.serial = serial;
        Token {
            epoch,
            kind: TokenKind::SourceTransfer,
            payload: serial,
        }
        .encode()
    }

    pub(crate) fn source_transfer_add(
        &mut self,
        transfer: u64,
        destination: u64,
        weight: u64,
    ) -> i32 {
        let Ok(token) = self.source_transfer_token(transfer) else {
            return 0;
        };
        let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
        let result = (|| {
            let entry = self
                .source_transfers
                .entries
                .iter_mut()
                .find(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            let previous = std::mem::replace(&mut entry.value, TransferValue::Invalid);
            let TransferValue::Upload = previous else {
                return Err(SourceFailure::Token);
            };
            let entries = &mut entry.upload;
            let destination = Destination::from_token(destination, token.epoch, rows)?;
            let destination = if let Destination::Row(row) = destination {
                self.current
                    .as_ref()
                    .and_then(|combat| combat.cards.get(row))
                    .filter(|row| row.kind == crate::source_kind::SourceKind::Unknown)
                    .map_or(destination, |row| Destination::Unknown(row.player))
            } else {
                destination
            };
            if weight == 0 {
                return Err(SourceFailure::Packet);
            }
            if let Some((_, existing)) = entries.iter_mut().find(|(key, _)| *key == destination) {
                *existing = existing
                    .checked_add(u128::from(weight))
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                if entries.len() == caps::SOURCE_DESTINATIONS {
                    return Err(SourceFailure::Capacity);
                }
                entries.push((destination, u128::from(weight)));
            }
            entry.value = TransferValue::Upload;
            Ok(())
        })();
        if let Err(failure) = result {
            self.source_transfers.diagnostics.report(failure);
            return 0;
        }
        1
    }

    pub(crate) fn source_transfer_seal(&mut self, transfer: u64) -> i32 {
        let Ok(token) = self.source_transfer_token(transfer) else {
            return 0;
        };
        let rows = self.current.as_ref().map_or(0, |combat| combat.cards.len());
        let result = (|| {
            let transfers = &mut self.source_transfers;
            let entry = transfers
                .entries
                .iter_mut()
                .find(|entry| entry.serial == token.payload)
                .ok_or(SourceFailure::Token)?;
            let previous = std::mem::replace(&mut entry.value, TransferValue::Invalid);
            let TransferValue::Upload = previous else {
                return Err(SourceFailure::Token);
            };
            entry.source.normalize_into(
                token.epoch,
                rows,
                &entry.upload,
                &mut transfers.scratch,
            )?;
            entry.value = TransferValue::Sealed;
            Ok(())
        })();
        if let Err(failure) = result {
            self.source_transfers.diagnostics.report(failure);
            return 0;
        }
        1
    }

    pub(crate) fn source_transfer_release(&mut self, transfer: u64) -> i32 {
        let Ok(token) = self.source_transfer_token(transfer) else {
            return 0;
        };
        let Some(index) = self
            .source_transfers
            .entries
            .iter()
            .position(|entry| entry.serial == token.payload)
        else {
            return 0;
        };
        self.source_transfers.entries.remove(index);
        1
    }

    pub(crate) fn source_count(&mut self, transfer: u64) -> i32 {
        self.source_transfer_read(transfer)
            .map_or(-1, |snapshot| snapshot.shares().len() as i32)
    }

    pub(crate) fn source_destination(&mut self, transfer: u64, index: i32) -> u64 {
        let result = self.source_transfer_read(transfer).and_then(|snapshot| {
            let share = usize::try_from(index)
                .ok()
                .and_then(|index| snapshot.shares().get(index))
                .ok_or(SourceFailure::Packet)?;
            let epoch = CombatEpoch::from_wire(u64::from(snapshot.combat_seq()))?;
            Ok(share.destination().token(epoch))
        });
        match result {
            Ok(destination) => destination,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
                0
            }
        }
    }

    pub(crate) fn source_weight(&mut self, transfer: u64, index: i32) -> u64 {
        let result = self.source_transfer_read(transfer).and_then(|snapshot| {
            usize::try_from(index)
                .ok()
                .and_then(|index| snapshot.shares().get(index))
                .map(|share| share.weight())
                .ok_or(SourceFailure::Packet)
        });
        match result {
            Ok(weight) => weight,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
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
    powers: storage::Slots<PowerProvenance>,
    grants: storage::Slots<FlatGrant>,
    generated: storage::Slots<GeneratedSource>,
    orbs: storage::Slots<InstanceSource>,
    plays: storage::Slots<ActiveSourcePlay>,
    incoming: SourceSnapshot,
    mixture_scratch: snapshot::SnapshotScratch,
    play_serial: u32,
    calculations: storage::Slots<DamageCalculation>,
    calculation_serial: u32,
    doom_batches: Vec<DoomBatch>,
    doom_serial: u32,
    reductions: storage::Slots<StrengthReduction>,
    pools: Vec<SourcePool>,
}

struct PowerProvenance {
    instance: u64,
    id: super::state::ModelId,
    owner: u64,
    owner_kind: CreatureKind,
    owner_slot: SourceSlot,
    observed: i32,
    source: SourceSnapshot,
    trusted: bool,
}

struct FlatGrant {
    power: usize,
    grant: PowerGrant,
}

struct GeneratedSource {
    instance: u64,
    source: SourceSnapshot,
    producer_role: ProducerRole,
}

struct InstanceSource {
    instance: u64,
    source: SourceSnapshot,
}

struct ActiveSourcePlay {
    serial: u32,
    execution: u64,
    card_instance: u64,
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
    source: SourceSnapshot,
    producer_role: ProducerRole,
    segment: ProducerSegment,
    original_target: u64,
    modifiers: storage::Slots<allocation::ModifierContribution>,
    results: Vec<ObservedDamage>,
    weak: SourceSnapshot,
    strength: storage::Slots<(SourceSnapshot, u64)>,
    complete: bool,
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
    targets: Vec<DoomCapture>,
    complete: bool,
}

struct DoomCapture {
    creature: u64,
    power_instance: u64,
    debit: u32,
    allocations: Vec<(Destination, u64)>,
}

#[derive(Default)]
struct SourcePool {
    blocks: storage::Slots<SourceBlock>,
    pending: storage::Slots<(SourceSnapshot, u64)>,
    osty: storage::Slots<SourceOsty>,
}

#[derive(Default)]
struct SourceBlock {
    base: SourcePrefix,
    base_original: u64,
    base_consumed: i64,
    remaining: u64,
    mods: storage::Slots<SourceBlockMod>,
}

impl SourceBlock {
    const MAX_MODS: usize = 4;
}

#[derive(Default)]
struct SourceBlockMod {
    source: SourcePrefix,
    original: u64,
    consumed: u64,
}

#[derive(Default)]
struct SourceOsty {
    source: SourcePrefix,
    remaining: u64,
}

const _: () = assert!(caps::PENDING_BLOCK_CONTRIBS >= SourceBlock::MAX_MODS);
const _: () = assert!(caps::MAX_PLAYER_SLOTS == 5);

impl SourceDiagnostics {
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
            self.provenance.clear();
            self.provenance.epoch = Some(epoch);
        }
        Ok(epoch)
    }

    fn source_status(&mut self, result: Result<(), SourceFailure>) -> i32 {
        match result {
            Ok(()) => 1,
            Err(failure) => {
                self.source_transfers.diagnostics.report(failure);
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

    fn source_export_incoming(&mut self) -> u64 {
        let transfer = self.source_transfer_begin(u64::from(self.provenance.incoming.combat_seq()));
        if transfer == 0 {
            return 0;
        }
        let serial = (transfer as u32) >> KIND_BITS;
        if let Some(entry) = self
            .source_transfers
            .entries
            .iter_mut()
            .find(|entry| entry.serial == serial)
        {
            entry.source.clone_from(&self.provenance.incoming);
            entry.value = TransferValue::Sealed;
            transfer
        } else {
            self.source_transfers
                .diagnostics
                .report(SourceFailure::Token);
            0
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
        let capture =
            SourceCaptureKind::decode(capture_kind, &mut self.source_transfers.diagnostics);
        let generation =
            GenerationState::decode(generation_state, &mut self.source_transfers.diagnostics);
        let kind = crate::source_kind::SourceKind::from_c(source_kind);
        let slot = super::state::clamp_source_slot(source_slot);
        self.provenance.incoming.set_unknown(epoch);
        match capture {
            SourceCaptureKind::CardInstance => {
                self.card_source_into_incoming(epoch, instance, source_id, slot, generation, false);
            }
            SourceCaptureKind::PowerInstance | SourceCaptureKind::WeakHead => {
                if let Some((index, power)) = self
                    .provenance
                    .powers
                    .iter()
                    .enumerate()
                    .find(|(_, power)| power.instance == instance && power.trusted)
                {
                    if capture == SourceCaptureKind::WeakHead {
                        if power.id == "WEAK_POWER"
                            && let Some(grant) = self
                                .provenance
                                .grants
                                .iter()
                                .find(|grant| grant.power == index)
                        {
                            self.provenance.incoming.clone_from(grant.grant.source());
                        }
                    } else if power.id != "POISON_POWER"
                        || self
                            .provenance
                            .grants
                            .iter()
                            .any(|grant| grant.power == index && grant.grant.remaining() > 0)
                    {
                        self.provenance.incoming.clone_from(&power.source);
                    }
                }
            }
            SourceCaptureKind::OrbInstance => {
                if let Some(orb) = self
                    .provenance
                    .orbs
                    .iter()
                    .find(|orb| orb.instance == instance)
                {
                    self.provenance.incoming.clone_from(&orb.source);
                }
            }
            SourceCaptureKind::DirectModel
                if matches!(
                    kind,
                    crate::source_kind::SourceKind::Relic | crate::source_kind::SourceKind::Potion
                ) && !source_id.is_empty() =>
            {
                self.named_source_into_incoming(epoch, slot, source_id, kind);
            }
            _ => {}
        }
        self.source_export_incoming()
    }

    fn named_source_into_incoming(
        &mut self,
        epoch: CombatEpoch,
        slot: SourceSlot,
        id: &str,
        kind: crate::source_kind::SourceKind,
    ) {
        let Some(combat) = Combat::active_mut(&mut self.current) else {
            self.provenance.incoming.set_unknown(epoch);
            return;
        };
        let destination = crate::data::ledger::get_or_create_card_kind(combat, slot, id, kind)
            .map_or(Destination::Unknown(slot), |row| {
                if combat.cards[row].kind == crate::source_kind::SourceKind::Unknown {
                    Destination::Unknown(slot)
                } else {
                    Destination::Row(row)
                }
            });
        self.provenance.incoming.set_single(epoch, destination);
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

struct LedgerStage {
    combat: Combat,
    pools: Vec<SourcePool>,
    capacity_lost: bool,
}

impl LedgerStage {
    fn new(state: &State) -> Result<Self, SourceFailure> {
        let combat = Combat::active(&state.current)
            .ok_or(SourceFailure::Epoch)?
            .clone();
        Ok(Self {
            combat,
            pools: state.provenance.pools.clone(),
            capacity_lost: false,
        })
    }

    fn row(&mut self, destination: Destination) -> Result<usize, SourceFailure> {
        match destination {
            Destination::Row(row) if row < self.combat.cards.len() => Ok(row),
            Destination::Unknown(slot) if slot <= TEAM_SLOT => {
                crate::data::ledger::get_or_create_card_kind(
                    &mut self.combat,
                    slot,
                    "UNATTRIBUTED",
                    crate::source_kind::SourceKind::Unknown,
                )
                .ok_or(SourceFailure::Capacity)
            }
            _ => Err(SourceFailure::Token),
        }
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
        if source.combat_seq() != self.combat.seq {
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
        while self.pools.len() <= usize::from(slot) {
            self.pools.push(SourcePool::default());
        }
        Ok(&mut self.pools[usize::from(slot)])
    }

    fn commit(self, state: &mut State) -> Result<(), SourceFailure> {
        if !crate::data::state::CardStat::arithmetic_representable(&self.combat.cards) {
            return Err(SourceFailure::Arithmetic);
        }
        debug_assert!(
            !state.ready() || state.current.storage.cards.capacity() >= caps::COMBAT_CARDS,
            "ledger publication must retain the initialized live row owner"
        );
        state.current.set(&self.combat);
        state.provenance.pools.clone_from(&self.pools);
        if self.capacity_lost {
            state
                .source_transfers
                .diagnostics
                .report(SourceFailure::Capacity);
        }
        Ok(())
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
            crate::data::persistence::event_log!("  turn {} started", combat.turns);
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
            self.slot_state_mut(player_slot).died = true;
            Ok(())
        })();
        self.source_status(result)
    }

    pub(crate) fn finished_combat(&mut self, combat_seq: u64) -> bool {
        if self.provenance_epoch(combat_seq).is_err() || self.finish_combat.is_none() {
            return false;
        }
        let Some(combat) = Combat::active_mut(&mut self.current) else {
            return false;
        };
        let defeated = if combat.players.is_empty() {
            !self.per_player.is_empty()
                && self
                    .per_player
                    .iter()
                    .take(caps::MAX_PLAYERS)
                    .all(|player| player.died)
        } else {
            combat.players.iter().all(|player| {
                self.per_player
                    .get(usize::from(player.slot))
                    .is_some_and(|tracked| tracked.died)
            })
        };
        combat.phase = super::state::CombatPhase::Finished(if defeated {
            super::state::CombatResult::Defeat
        } else {
            super::state::CombatResult::Completed
        });
        self.finish_combat
            .as_mut()
            .expect("finish staging was checked before mutation")
            .clone_from(combat);
        self.clear_combat_sources();
        true
    }
}

#[cfg(test)]
mod limits;
#[cfg(test)]
mod scenarios;

impl State {
    pub(crate) fn reserve_sources(&mut self) -> Result<(), TryReserveError> {
        self.source_transfers.reserve()?;
        self.provenance.reserve_tracking()?;
        self.provenance.reserve_calculations_and_pools()?;
        self.provenance
            .doom_batches
            .try_reserve_exact(caps::DOOM_BATCHES)?;
        Ok(())
    }

    pub(crate) fn clear_combat_sources(&mut self) {
        self.source_transfers.clear();
        self.provenance.clear();
    }
}

impl Provenance {
    fn clear(&mut self) {
        self.clear_tracking();
        self.clear_calculations_and_pools();
        self.doom_batches.clear();
        self.epoch = None;
        self.play_serial = 0;
        self.calculation_serial = 0;
        self.doom_serial = 0;
    }
}

impl State {
    pub(crate) fn discard_combat(&mut self) {
        self.current.clear();
        self.clear_combat_sources();
    }
}
