//! Bounded observation traces contain values, never process pointers or engine
//! IDs. Replaying on a fresh State must reproduce every returned source/group
//! handle before publishing a snapshot. Truncation is explicit and unreplayable.
//! A handler panic truncates the trace because its observation may be missing.

use serde::{Deserialize, Serialize};

use super::modifiers::{ModifierObservation, WeakObservation};
use super::state::State;

const MAX_OBSERVATIONS: usize = 10_000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const TRACE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Observation {
    WeakProjection {
        observed: WeakObservation,
    },
    ModifierProjection {
        observed: ModifierObservation,
    },
    CombatDiscard,
    SourceCapture {
        combat_seq: u64,
        capture_kind: i32,
        instance: u64,
        source_id: Box<str>,
        source_kind: i32,
        source_slot: i32,
        generation_state: i32,
    },
    PowerAttached {
        combat_seq: u64,
        power_instance: u64,
        power_id: Box<str>,
        owner_creature: u64,
        owner_kind: i32,
        owner_slot: i32,
        amount: i32,
        source_transfer: u64,
    },
    PowerAmountChanged {
        combat_seq: u64,
        power_instance: u64,
        power_id: Box<str>,
        owner_creature: u64,
        owner_kind: i32,
        owner_slot: i32,
        old_amount: i32,
        new_amount: i32,
        source_transfer: u64,
    },
    PowerRemoved {
        combat_seq: u64,
        power_instance: u64,
    },
    PowerProvenanceInvalidate {
        combat_seq: u64,
        power_instance: u64,
    },
    CardGenerated {
        combat_seq: u64,
        card_instance: u64,
        source_transfer: u64,
        producer_role: i32,
    },
    CardPlayStarted {
        combat_seq: u64,
        execution_id: u64,
        card_instance: u64,
        card_id: Box<str>,
        player_slot: i32,
        play_index: i32,
        play_count: i32,
        generation_state: i32,
        source_transfer: u64,
    },
    CardPlayFinished {
        play: u64,
    },
    CardExecutionEnded {
        combat_seq: u64,
        execution_id: u64,
    },
    OrbChanneled {
        combat_seq: u64,
        orb_instance: u64,
        source_transfer: u64,
    },
    OrbContextBegin {
        combat_seq: u64,
        orb_instance: u64,
        play: u64,
        owner_slot: i32,
    },
    DamageCalculationBegin {
        combat_seq: u64,
        source_transfer: u64,
        producer_role: i32,
        segment: i32,
        original_target: u64,
    },
    DamageModifierContribution {
        calculation: u64,
        source_transfer: u64,
        amount: i32,
    },
    DamageCalculationEnemyHit {
        calculation: u64,
        dealer_creature: u64,
        base_damage: i32,
        dealer_strength: i32,
    },
    DamageCalculationWeakSource {
        calculation: u64,
        source_transfer: u64,
    },
    DamageResultAppend {
        calculation: u64,
        total: i32,
        unblocked: i32,
        blocked: i32,
        result_kind: i32,
        receiver_slot: i32,
        weak_prevented: i32,
    },
    DamageCalculationCommit {
        calculation: u64,
    },
    DamageCalculationAbort {
        calculation: u64,
    },
    DamageUnattributed {
        combat_seq: u64,
        total: i32,
        unblocked: i32,
        blocked: i32,
        result_kind: i32,
        receiver_slot: i32,
        weak_prevented: i32,
    },
    BuffMitigation {
        combat_seq: u64,
        source_transfer: u64,
        prevented: i32,
    },
    BlockModifierContribution {
        combat_seq: u64,
        source_transfer: u64,
        amount: i32,
        receiver_slot: i32,
    },
    BlockGained {
        combat_seq: u64,
        amount: i32,
        source_transfer: u64,
        receiver_slot: i32,
    },
    Forge {
        combat_seq: u64,
        source_transfer: u64,
        amount: i32,
    },
    OstySummoned {
        combat_seq: u64,
        source_transfer: u64,
        hp_amount: i32,
        owner_slot: i32,
    },
    OstyKilled {
        combat_seq: u64,
        owner_slot: i32,
        play: u64,
    },
    DoomBatchBegin {
        combat_seq: u64,
    },
    DoomTargetCapture {
        batch: u64,
        creature_instance: u64,
        doom_power_instance: u64,
        current_hp: i32,
    },
    DoomKillsCompleted {
        batch: u64,
    },
    DoomBatchAbort {
        batch: u64,
    },
    CombatEnded {
        combat_seq: u64,
    },
    TurnStarted {
        combat_seq: u64,
    },
    BlockPoolClear {
        combat_seq: u64,
        player_slot: i32,
    },
    PlayerDied {
        combat_seq: u64,
        player_slot: i32,
    },
    PotionUsed {
        combat_seq: u64,
    },
    CombatStarted {
        seq: u32,
        encounter_id: Box<str>,
        encounter_type: Box<str>,
        started_at: i64,
        player_count: i32,
    },
    SourceAccumulate {
        combat_seq: u64,
        first: u64,
        before: i32,
        second: u64,
        after: i32,
    },
    CaptureFailed {
        reason: Box<str>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    observation: Observation,
    result: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recording {
    trace_version: u32,
    policy_version: u32,
    pub(crate) truncated: bool,
    observations: Vec<Entry>,
    #[serde(skip)]
    bytes: usize,
}

impl State {
    pub(crate) fn recording_begin(&mut self) -> i32 {
        if super::state::Combat::active(&self.current).is_some() {
            return 0;
        }
        self.recording = Some(Recording {
            trace_version: TRACE_VERSION,
            policy_version: super::summary::POLICY_VERSION,
            truncated: false,
            observations: Vec::new(),
            bytes: 0,
        });
        1
    }

    pub(crate) fn recording_json(&self) -> String {
        serde_json::to_string(&self.recording)
            .expect("observations contain serializable scalar values")
    }

    pub(crate) fn record(&mut self, make: impl FnOnce() -> Observation, result: u64) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        if recording.truncated {
            return;
        }
        if recording.observations.len() == MAX_OBSERVATIONS {
            recording.truncated = true;
            return;
        }
        let entry = Entry {
            observation: make(),
            result,
        };
        let bytes = serde_json::to_vec(&entry)
            .expect("observations contain scalar values")
            .len();
        if bytes > MAX_BYTES - recording.bytes {
            recording.truncated = true;
            return;
        }
        recording.bytes += bytes;
        recording.observations.push(entry);
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one exhaustive dispatch keeps the recorded protocol auditable"
    )]
    pub(crate) fn replay(&mut self, json: &str) -> i32 {
        if json.len() > MAX_BYTES + MAX_OBSERVATIONS + 1024 {
            return 0;
        }
        let Ok(recording) = serde_json::from_str::<Recording>(json) else {
            return 0;
        };
        if recording.trace_version != TRACE_VERSION
            || recording.policy_version != super::summary::POLICY_VERSION
            || recording.truncated
            || recording.observations.len() > MAX_OBSERVATIONS
        {
            return 0;
        }
        let mut candidate = State::default();
        for entry in recording.observations {
            let result = match entry.observation {
                Observation::WeakProjection { observed } => match observed.prevention() {
                    Ok(amount) => amount as u64,
                    Err(_) => {
                        candidate.capture_failed("weak-policy");
                        u64::MAX
                    }
                },
                Observation::ModifierProjection { observed } => match observed.credit() {
                    Ok(amount) => i64::from(amount) as u64,
                    Err(_) => {
                        candidate.capture_failed("modifier-policy");
                        i64::MIN as u64
                    }
                },
                Observation::CombatDiscard => {
                    candidate.discard_combat();
                    0
                }
                Observation::SourceCapture {
                    combat_seq,
                    capture_kind,
                    instance,
                    source_id,
                    source_kind,
                    source_slot,
                    generation_state,
                } => candidate.source_capture(
                    combat_seq,
                    capture_kind,
                    instance,
                    &source_id,
                    source_kind,
                    source_slot,
                    generation_state,
                ),
                Observation::PowerAttached {
                    combat_seq,
                    power_instance,
                    power_id,
                    owner_creature,
                    owner_kind,
                    owner_slot,
                    amount,
                    source_transfer,
                } => candidate.power_attached(
                    combat_seq,
                    power_instance,
                    &power_id,
                    owner_creature,
                    owner_kind,
                    owner_slot,
                    amount,
                    source_transfer,
                ) as u64,
                Observation::PowerAmountChanged {
                    combat_seq,
                    power_instance,
                    power_id,
                    owner_creature,
                    owner_kind,
                    owner_slot,
                    old_amount,
                    new_amount,
                    source_transfer,
                } => candidate.power_amount_changed(
                    combat_seq,
                    power_instance,
                    &power_id,
                    owner_creature,
                    owner_kind,
                    owner_slot,
                    old_amount,
                    new_amount,
                    source_transfer,
                ) as u64,
                Observation::PowerRemoved {
                    combat_seq,
                    power_instance,
                } => candidate.power_removed(combat_seq, power_instance) as u64,
                Observation::PowerProvenanceInvalidate {
                    combat_seq,
                    power_instance,
                } => candidate.power_provenance_invalidate(combat_seq, power_instance) as u64,
                Observation::CardGenerated {
                    combat_seq,
                    card_instance,
                    source_transfer,
                    producer_role,
                } => candidate.card_generated(
                    combat_seq,
                    card_instance,
                    source_transfer,
                    producer_role,
                ) as u64,
                Observation::CardPlayStarted {
                    combat_seq,
                    execution_id,
                    card_instance,
                    card_id,
                    player_slot,
                    play_index,
                    play_count,
                    generation_state,
                    source_transfer,
                } => candidate.card_play_started(
                    combat_seq,
                    execution_id,
                    card_instance,
                    &card_id,
                    player_slot,
                    play_index,
                    play_count,
                    generation_state,
                    source_transfer,
                ),
                Observation::CardPlayFinished { play } => candidate.card_play_finished(play) as u64,
                Observation::CardExecutionEnded {
                    combat_seq,
                    execution_id,
                } => candidate.card_execution_ended(combat_seq, execution_id) as u64,
                Observation::OrbChanneled {
                    combat_seq,
                    orb_instance,
                    source_transfer,
                } => candidate.orb_channeled(combat_seq, orb_instance, source_transfer) as u64,
                Observation::OrbContextBegin {
                    combat_seq,
                    orb_instance,
                    play,
                    owner_slot,
                } => candidate.orb_context_begin(combat_seq, orb_instance, play, owner_slot) as u64,
                Observation::DamageCalculationBegin {
                    combat_seq,
                    source_transfer,
                    producer_role,
                    segment,
                    original_target,
                } => candidate.damage_calculation_begin(
                    combat_seq,
                    source_transfer,
                    producer_role,
                    segment,
                    original_target,
                ),
                Observation::DamageModifierContribution {
                    calculation,
                    source_transfer,
                    amount,
                } => candidate.damage_modifier_contribution(calculation, source_transfer, amount)
                    as u64,
                Observation::DamageCalculationEnemyHit {
                    calculation,
                    dealer_creature,
                    base_damage,
                    dealer_strength,
                } => candidate.damage_calculation_enemy_hit(
                    calculation,
                    dealer_creature,
                    base_damage,
                    dealer_strength,
                ) as u64,
                Observation::DamageCalculationWeakSource {
                    calculation,
                    source_transfer,
                } => candidate.damage_calculation_weak_source(calculation, source_transfer) as u64,
                Observation::DamageResultAppend {
                    calculation,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                } => candidate.damage_result_append(
                    calculation,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                ) as u64,
                Observation::DamageCalculationCommit { calculation } => {
                    candidate.damage_calculation_commit(calculation) as u64
                }
                Observation::DamageCalculationAbort { calculation } => {
                    candidate.damage_calculation_abort(calculation) as u64
                }
                Observation::DamageUnattributed {
                    combat_seq,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                } => candidate.damage_unattributed(
                    combat_seq,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                ) as u64,
                Observation::BuffMitigation {
                    combat_seq,
                    source_transfer,
                    prevented,
                } => candidate.buff_mitigation(combat_seq, source_transfer, prevented) as u64,
                Observation::BlockModifierContribution {
                    combat_seq,
                    source_transfer,
                    amount,
                    receiver_slot,
                } => candidate.block_modifier_contribution(
                    combat_seq,
                    source_transfer,
                    amount,
                    receiver_slot,
                ) as u64,
                Observation::BlockGained {
                    combat_seq,
                    amount,
                    source_transfer,
                    receiver_slot,
                } => candidate.block_gained(combat_seq, amount, source_transfer, receiver_slot)
                    as u64,
                Observation::Forge {
                    combat_seq,
                    source_transfer,
                    amount,
                } => candidate.forge(combat_seq, source_transfer, amount) as u64,
                Observation::OstySummoned {
                    combat_seq,
                    source_transfer,
                    hp_amount,
                    owner_slot,
                } => candidate.osty_summoned(combat_seq, source_transfer, hp_amount, owner_slot)
                    as u64,
                Observation::OstyKilled {
                    combat_seq,
                    owner_slot,
                    play,
                } => candidate.osty_killed(combat_seq, owner_slot, play) as u64,
                Observation::DoomBatchBegin { combat_seq } => {
                    candidate.doom_batch_begin(combat_seq)
                }
                Observation::DoomTargetCapture {
                    batch,
                    creature_instance,
                    doom_power_instance,
                    current_hp,
                } => candidate.doom_target_capture(
                    batch,
                    creature_instance,
                    doom_power_instance,
                    current_hp,
                ) as u64,
                Observation::DoomKillsCompleted { batch } => {
                    candidate.doom_kills_completed(batch) as u64
                }
                Observation::DoomBatchAbort { batch } => candidate.doom_batch_abort(batch) as u64,
                Observation::CombatEnded { combat_seq } => {
                    candidate.combat_ended(combat_seq) as u64
                }
                Observation::TurnStarted { combat_seq } => {
                    candidate.turn_started(combat_seq) as u64
                }
                Observation::BlockPoolClear {
                    combat_seq,
                    player_slot,
                } => candidate.block_pool_clear(combat_seq, player_slot) as u64,
                Observation::PlayerDied {
                    combat_seq,
                    player_slot,
                } => candidate.player_died(combat_seq, player_slot) as u64,
                Observation::PotionUsed { combat_seq } => candidate.potion_used(combat_seq) as u64,
                Observation::CombatStarted {
                    seq,
                    encounter_id,
                    encounter_type,
                    started_at,
                    player_count,
                } => candidate.combat_started(
                    seq,
                    &encounter_id,
                    &encounter_type,
                    started_at,
                    player_count,
                ),
                Observation::SourceAccumulate {
                    combat_seq,
                    first,
                    before,
                    second,
                    after,
                } => candidate.source_accumulate(combat_seq, first, before, second, after),
                Observation::CaptureFailed { reason } => {
                    candidate.capture_failed(&reason);
                    0
                }
            };
            if result != entry.result {
                return 0;
            }
        }
        candidate.revision = self.revision.saturating_add(1);
        *self = candidate;
        1
    }
}
