//! Owned native engine registry. IDs are valid only on their creation thread;
//! stale or foreign IDs return failure without dereferencing host pointers.
//! Every export contains panics. No caller receives a Rust allocation or borrow.

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::data::modifiers::{ModifierObservation, WeakObservation};
use crate::data::observation::Observation;
use crate::data::state::State;

#[repr(C)]
pub struct BlockModifier {
    pub source: u64,
    pub credit: i64,
}

const _: () = assert!(std::mem::size_of::<BlockModifier>() == 16);
const _: () = assert!(std::mem::offset_of!(BlockModifier, source) == 0);
const _: () = assert!(std::mem::offset_of!(BlockModifier, credit) == 8);

#[derive(Default)]
struct Engines {
    entries: Vec<(u64, State)>,
}
static NEXT_ENGINE: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static ENGINES: RefCell<Engines> = RefCell::new(Engines::default());
    static PANIC_REPORTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Returns `on_panic` after an unwinding Rust panic. Payload destructors
/// are suppressed because they can panic; aborting panics remain unrecoverable.
pub(crate) fn contain<T: Copy>(name: &str, on_panic: T, f: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            let payload = std::mem::ManuallyDrop::new(payload);
            let logged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let detail = if let Some(msg) = payload.downcast_ref::<&str>() {
                    *msg
                } else if let Some(msg) = payload.downcast_ref::<String>() {
                    msg.as_str()
                } else {
                    "non-string panic payload"
                };
                PANIC_REPORTED.with(|reported| {
                    if !reported.replace(true) {
                        let _ = writeln!(
                            std::io::stderr().lock(),
                            "[SpireProfiler] panic in {name}: {detail}"
                        );
                    }
                });
            }));
            if let Err(payload) = logged {
                std::mem::forget(payload);
            }
            on_panic
        }
    }
}

fn with_engine<T: Copy>(
    id: u64,
    unavailable: T,
    mutate: bool,
    f: impl FnOnce(&mut State) -> T,
) -> T {
    ENGINES.with(|cell| {
        let Ok(mut engines) = cell.try_borrow_mut() else {
            return unavailable;
        };
        let Some((_, state)) = engines.entries.iter_mut().find(|(key, _)| *key == id) else {
            return unavailable;
        };
        if mutate {
            state.revision = state.revision.saturating_add(1);
        }
        struct PanicGuard<'a>(&'a mut State);
        impl Drop for PanicGuard<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.poisoned = true;
                    if let Some(recording) = &mut self.0.recording {
                        recording.truncated = true;
                    }
                    self.0.capture_failed("native-panic");
                }
            }
        }
        let guard = PanicGuard(state);
        f(guard.0)
    })
}

/// # Safety
/// A non-null pointer covers readable bytes through a NUL within one allocation,
/// unchanged for the duration of the call. Null and malformed UTF-8 map to empty.
unsafe fn with_c_str<T>(ptr: *const c_char, f: impl for<'a> FnOnce(&'a str) -> T) -> T {
    let text = if ptr.is_null() {
        ""
    } else {
        // SAFETY: the caller guarantees a readable terminated, unmodified buffer.
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
    };
    f(text)
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_engine_create() -> u64 {
    contain("engine_create", 0, || {
        ENGINES.with(|cell| {
            let mut engines = cell.borrow_mut();
            // Hosts need one engine; extra slots permit isolated comparisons and tests.
            if engines.entries.len() == 16 {
                return 0;
            }
            let Ok(id) = NEXT_ENGINE
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            else {
                return 0;
            };
            engines.entries.push((id, State::default()));
            id
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_engine_destroy(engine: u64) {
    contain("engine_destroy", (), || {
        ENGINES.with(|cell| {
            cell.borrow_mut().entries.retain(|(id, _)| *id != engine);
        })
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_revision(engine: u64) -> u64 {
    contain("revision", 0, || {
        with_engine(engine, 0, false, |state| state.revision)
    })
}

/// Returns required UTF-8 bytes including the NUL. A null/short buffer is not
/// written. The synchronous host cannot mutate the engine between sizing/copy.
/// # Safety
/// A non-null buffer is writable for capacity bytes and does not alias inputs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_snapshot(
    engine: u64,
    buffer: *mut u8,
    capacity: i32,
) -> i32 {
    contain("snapshot", 0, || {
        with_engine(engine, 0, false, |state| {
            let json = state.snapshot();
            let Ok(size) = i32::try_from(json.len() + 1) else {
                return 0;
            };
            if !buffer.is_null() && capacity >= size {
                // SAFETY: capacity covers this byte count and the buffers cannot alias.
                unsafe {
                    std::ptr::copy_nonoverlapping(json.as_ptr(), buffer, json.len());
                    buffer.add(json.len()).write(0);
                }
            }
            size
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_source_release(engine: u64, handle: u64) -> i32 {
    contain("source_release", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.source_release(handle);
            state.record(|| Observation::SourceRelease { handle }, result as u64);
            result
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_source_capture(
    engine: u64,
    combat_seq: u64,
    capture_kind: i32,
    instance: u64,
    source_id: *const c_char,
    source_kind: i32,
    source_slot: i32,
    generation_state: i32,
) -> u64 {
    contain("source_capture", 0, || {
        with_engine(engine, 0, true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(source_id, |source_id| {
                    let result = state.source_capture(
                        combat_seq,
                        capture_kind,
                        instance,
                        source_id,
                        source_kind,
                        source_slot,
                        generation_state,
                    );
                    state.record(
                        || Observation::SourceCapture {
                            combat_seq,
                            capture_kind,
                            instance,
                            source_id: source_id.into(),
                            source_kind,
                            source_slot,
                            generation_state,
                        },
                        result,
                    );
                    result
                })
            }
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_power_attached(
    engine: u64,
    combat_seq: u64,
    power_instance: u64,
    power_id: *const c_char,
    owner_creature: u64,
    owner_kind: i32,
    owner_slot: i32,
    amount: i32,
    source_transfer: u64,
) -> i32 {
    contain("power_attached", 0, || {
        with_engine(engine, 0, true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(power_id, |power_id| {
                    let result = state.power_attached(
                        combat_seq,
                        power_instance,
                        power_id,
                        owner_creature,
                        owner_kind,
                        owner_slot,
                        amount,
                        source_transfer,
                    );
                    state.record(
                        || Observation::PowerAttached {
                            combat_seq,
                            power_instance,
                            power_id: power_id.into(),
                            owner_creature,
                            owner_kind,
                            owner_slot,
                            amount,
                            source_transfer,
                        },
                        result as u64,
                    );
                    result
                })
            }
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_power_amount_changed(
    engine: u64,
    combat_seq: u64,
    power_instance: u64,
    power_id: *const c_char,
    owner_creature: u64,
    owner_kind: i32,
    owner_slot: i32,
    old_amount: i32,
    new_amount: i32,
    source_transfer: u64,
) -> i32 {
    contain("power_amount_changed", 0, || {
        with_engine(engine, 0, true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(power_id, |power_id| {
                    let result = state.power_amount_changed(
                        combat_seq,
                        power_instance,
                        power_id,
                        owner_creature,
                        owner_kind,
                        owner_slot,
                        old_amount,
                        new_amount,
                        source_transfer,
                    );
                    state.record(
                        || Observation::PowerAmountChanged {
                            combat_seq,
                            power_instance,
                            power_id: power_id.into(),
                            owner_creature,
                            owner_kind,
                            owner_slot,
                            old_amount,
                            new_amount,
                            source_transfer,
                        },
                        result as u64,
                    );
                    result
                })
            }
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_power_removed(
    engine: u64,
    combat_seq: u64,
    power_instance: u64,
) -> i32 {
    contain("power_removed", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.power_removed(combat_seq, power_instance);
            state.record(
                || Observation::PowerRemoved {
                    combat_seq,
                    power_instance,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_power_provenance_invalidate(
    engine: u64,
    combat_seq: u64,
    power_instance: u64,
) -> i32 {
    contain("power_provenance_invalidate", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.power_provenance_invalidate(combat_seq, power_instance);
            state.record(
                || Observation::PowerProvenanceInvalidate {
                    combat_seq,
                    power_instance,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_card_generated(
    engine: u64,
    combat_seq: u64,
    card_instance: u64,
    source_transfer: u64,
    producer_role: i32,
) -> i32 {
    contain("card_generated", 0, || {
        with_engine(engine, 0, true, |state| {
            let result =
                state.card_generated(combat_seq, card_instance, source_transfer, producer_role);
            state.record(
                || Observation::CardGenerated {
                    combat_seq,
                    card_instance,
                    source_transfer,
                    producer_role,
                },
                result as u64,
            );
            result
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_card_play_started(
    engine: u64,
    combat_seq: u64,
    execution_id: u64,
    card_instance: u64,
    card_id: *const c_char,
    player_slot: i32,
    play_index: i32,
    play_count: i32,
    generation_state: i32,
    source_transfer: u64,
) -> u64 {
    contain("card_play_started", 0, || {
        with_engine(engine, 0, true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(card_id, |card_id| {
                    let result = state.card_play_started(
                        combat_seq,
                        execution_id,
                        card_instance,
                        card_id,
                        player_slot,
                        play_index,
                        play_count,
                        generation_state,
                        source_transfer,
                    );
                    state.record(
                        || Observation::CardPlayStarted {
                            combat_seq,
                            execution_id,
                            card_instance,
                            card_id: card_id.into(),
                            player_slot,
                            play_index,
                            play_count,
                            generation_state,
                            source_transfer,
                        },
                        result,
                    );
                    result
                })
            }
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_card_play_finished(engine: u64, play: u64) -> i32 {
    contain("card_play_finished", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.card_play_finished(play);
            state.record(|| Observation::CardPlayFinished { play }, result as u64);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_card_execution_ended(
    engine: u64,
    combat_seq: u64,
    execution_id: u64,
) -> i32 {
    contain("card_execution_ended", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.card_execution_ended(combat_seq, execution_id);
            state.record(
                || Observation::CardExecutionEnded {
                    combat_seq,
                    execution_id,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_orb_channeled(
    engine: u64,
    combat_seq: u64,
    orb_instance: u64,
    source_transfer: u64,
) -> i32 {
    contain("orb_channeled", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.orb_channeled(combat_seq, orb_instance, source_transfer);
            state.record(
                || Observation::OrbChanneled {
                    combat_seq,
                    orb_instance,
                    source_transfer,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_orb_context_begin(
    engine: u64,
    combat_seq: u64,
    orb_instance: u64,
    play: u64,
    owner_slot: i32,
) -> i32 {
    contain("orb_context_begin", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.orb_context_begin(combat_seq, orb_instance, play, owner_slot);
            state.record(
                || Observation::OrbContextBegin {
                    combat_seq,
                    orb_instance,
                    play,
                    owner_slot,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_calculation_begin(
    engine: u64,
    combat_seq: u64,
    source_transfer: u64,
    producer_role: i32,
    segment: i32,
    original_target: u64,
) -> u64 {
    contain("damage_calculation_begin", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_calculation_begin(
                combat_seq,
                source_transfer,
                producer_role,
                segment,
                original_target,
            );
            state.record(
                || Observation::DamageCalculationBegin {
                    combat_seq,
                    source_transfer,
                    producer_role,
                    segment,
                    original_target,
                },
                result,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_modifier_contribution(
    engine: u64,
    calculation: u64,
    source_transfer: u64,
    amount: i32,
) -> i32 {
    contain("damage_modifier_contribution", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_modifier_contribution(calculation, source_transfer, amount);
            state.record(
                || Observation::DamageModifierContribution {
                    calculation,
                    source_transfer,
                    amount,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_calculation_enemy_hit(
    engine: u64,
    calculation: u64,
    dealer_creature: u64,
    base_damage: i32,
    dealer_strength: i32,
) -> i32 {
    contain("damage_calculation_enemy_hit", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_calculation_enemy_hit(
                calculation,
                dealer_creature,
                base_damage,
                dealer_strength,
            );
            state.record(
                || Observation::DamageCalculationEnemyHit {
                    calculation,
                    dealer_creature,
                    base_damage,
                    dealer_strength,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_calculation_weak_source(
    engine: u64,
    calculation: u64,
    source_transfer: u64,
) -> i32 {
    contain("damage_calculation_weak_source", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_calculation_weak_source(calculation, source_transfer);
            state.record(
                || Observation::DamageCalculationWeakSource {
                    calculation,
                    source_transfer,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_result_append(
    engine: u64,
    calculation: u64,
    total: i32,
    unblocked: i32,
    blocked: i32,
    result_kind: i32,
    receiver_slot: i32,
    weak_prevented: i32,
) -> i32 {
    contain("damage_result_append", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_result_append(
                calculation,
                total,
                unblocked,
                blocked,
                result_kind,
                receiver_slot,
                weak_prevented,
            );
            state.record(
                || Observation::DamageResultAppend {
                    calculation,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_calculation_commit(engine: u64, calculation: u64) -> i32 {
    contain("damage_calculation_commit", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_calculation_commit(calculation);
            state.record(
                || Observation::DamageCalculationCommit { calculation },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_calculation_abort(engine: u64, calculation: u64) -> i32 {
    contain("damage_calculation_abort", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_calculation_abort(calculation);
            state.record(
                || Observation::DamageCalculationAbort { calculation },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_damage_unattributed(
    engine: u64,
    combat_seq: u64,
    total: i32,
    unblocked: i32,
    blocked: i32,
    result_kind: i32,
    receiver_slot: i32,
    weak_prevented: i32,
) -> i32 {
    contain("damage_unattributed", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.damage_unattributed(
                combat_seq,
                total,
                unblocked,
                blocked,
                result_kind,
                receiver_slot,
                weak_prevented,
            );
            state.record(
                || Observation::DamageUnattributed {
                    combat_seq,
                    total,
                    unblocked,
                    blocked,
                    result_kind,
                    receiver_slot,
                    weak_prevented,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_buff_mitigation(
    engine: u64,
    combat_seq: u64,
    source_transfer: u64,
    prevented: i32,
) -> i32 {
    contain("buff_mitigation", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.buff_mitigation(combat_seq, source_transfer, prevented);
            state.record(
                || Observation::BuffMitigation {
                    combat_seq,
                    source_transfer,
                    prevented,
                },
                result as u64,
            );
            result
        })
    })
}

/// # Safety
/// For a nonzero in-range count, a non-null aligned modifiers pointer covers
/// count initialized entries, readable and unmodified throughout this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_block_gained(
    engine: u64,
    combat_seq: u64,
    amount: i32,
    source_transfer: u64,
    receiver_slot: i32,
    modifiers: *const BlockModifier,
    count: i32,
    incomplete: i32,
) -> i32 {
    contain("block_gained", 0, || {
        with_engine(engine, 0, true, |state| {
            let valid_count =
                (0..=crate::data::state::caps::BLOCK_MODIFIERS as i32).contains(&count);
            let valid_buffer = count == 0 || (!modifiers.is_null() && modifiers.is_aligned());
            let entries: Vec<_> = if valid_count && valid_buffer && count > 0 {
                // SAFETY: the caller owns count initialized, aligned readable entries.
                unsafe { std::slice::from_raw_parts(modifiers, count as usize) }
                    .iter()
                    .map(|entry| (entry.source, entry.credit))
                    .collect()
            } else {
                Vec::new()
            };
            let incomplete = incomplete != 0 || !valid_count || !valid_buffer;
            let parsed = state.parse_block_modifiers(combat_seq, &entries, incomplete);
            let result =
                state.block_gained(combat_seq, amount, source_transfer, receiver_slot, parsed);
            state.record(
                || Observation::BlockGained {
                    combat_seq,
                    amount,
                    source_transfer,
                    receiver_slot,
                    modifiers: entries,
                    incomplete,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_forge(
    engine: u64,
    combat_seq: u64,
    source_transfer: u64,
    amount: i32,
) -> i32 {
    contain("forge", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.forge(combat_seq, source_transfer, amount);
            state.record(
                || Observation::Forge {
                    combat_seq,
                    source_transfer,
                    amount,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_osty_summoned(
    engine: u64,
    combat_seq: u64,
    source_transfer: u64,
    hp_amount: i32,
    owner_slot: i32,
) -> i32 {
    contain("osty_summoned", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.osty_summoned(combat_seq, source_transfer, hp_amount, owner_slot);
            state.record(
                || Observation::OstySummoned {
                    combat_seq,
                    source_transfer,
                    hp_amount,
                    owner_slot,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_osty_killed(
    engine: u64,
    combat_seq: u64,
    owner_slot: i32,
    play: u64,
) -> i32 {
    contain("osty_killed", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.osty_killed(combat_seq, owner_slot, play);
            state.record(
                || Observation::OstyKilled {
                    combat_seq,
                    owner_slot,
                    play,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_doom_batch_begin(engine: u64, combat_seq: u64) -> u64 {
    contain("doom_batch_begin", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.doom_batch_begin(combat_seq);
            state.record(|| Observation::DoomBatchBegin { combat_seq }, result);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_doom_target_capture(
    engine: u64,
    batch: u64,
    creature_instance: u64,
    doom_power_instance: u64,
    current_hp: i32,
) -> i32 {
    contain("doom_target_capture", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.doom_target_capture(
                batch,
                creature_instance,
                doom_power_instance,
                current_hp,
            );
            state.record(
                || Observation::DoomTargetCapture {
                    batch,
                    creature_instance,
                    doom_power_instance,
                    current_hp,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_doom_kills_completed(engine: u64, batch: u64) -> i32 {
    contain("doom_kills_completed", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.doom_kills_completed(batch);
            state.record(|| Observation::DoomKillsCompleted { batch }, result as u64);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_doom_batch_abort(engine: u64, batch: u64) -> i32 {
    contain("doom_batch_abort", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.doom_batch_abort(batch);
            state.record(|| Observation::DoomBatchAbort { batch }, result as u64);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_combat_ended(engine: u64, combat_seq: u64) -> i32 {
    contain("combat_ended", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.combat_ended(combat_seq);
            state.record(|| Observation::CombatEnded { combat_seq }, result as u64);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_turn_started(engine: u64, combat_seq: u64) -> i32 {
    contain("turn_started", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.turn_started(combat_seq);
            state.record(|| Observation::TurnStarted { combat_seq }, result as u64);
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_block_pool_clear(
    engine: u64,
    combat_seq: u64,
    player_slot: i32,
) -> i32 {
    contain("block_pool_clear", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.block_pool_clear(combat_seq, player_slot);
            state.record(
                || Observation::BlockPoolClear {
                    combat_seq,
                    player_slot,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_player_died(
    engine: u64,
    combat_seq: u64,
    player_slot: i32,
) -> i32 {
    contain("player_died", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.player_died(combat_seq, player_slot);
            state.record(
                || Observation::PlayerDied {
                    combat_seq,
                    player_slot,
                },
                result as u64,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_potion_used(engine: u64, combat_seq: u64) -> i32 {
    contain("potion_used", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.potion_used(combat_seq);
            state.record(|| Observation::PotionUsed { combat_seq }, result as u64);
            result
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_combat_started(
    engine: u64,
    seq: u32,
    encounter_id: *const c_char,
    encounter_type: *const c_char,
    started_at: i64,
    player_count: i32,
) -> u64 {
    contain("combat_started", 0, || {
        with_engine(engine, 0, true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(encounter_id, |encounter_id| {
                    with_c_str(encounter_type, |encounter_type| {
                        let result = state.combat_started(
                            seq,
                            encounter_id,
                            encounter_type,
                            started_at,
                            player_count,
                        );
                        state.record(
                            || Observation::CombatStarted {
                                seq,
                                encounter_id: encounter_id.into(),
                                encounter_type: encounter_type.into(),
                                started_at,
                                player_count,
                            },
                            result,
                        );
                        result
                    })
                })
            }
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_source_accumulate(
    engine: u64,
    combat_seq: u64,
    first: u64,
    before: i32,
    second: u64,
    after: i32,
) -> u64 {
    contain("source_accumulate", 0, || {
        with_engine(engine, 0, true, |state| {
            let result = state.source_accumulate(combat_seq, first, before, second, after);
            state.record(
                || Observation::SourceAccumulate {
                    combat_seq,
                    first,
                    before,
                    second,
                    after,
                },
                result,
            );
            result
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_weak_prevention(
    engine: u64,
    total: i32,
    receiver_slot: i32,
    receiver_player: i32,
    weak: i32,
    debilitate: i32,
    krane_slots: u32,
) -> i32 {
    contain("weak_prevention", -1, || {
        with_engine(engine, -1, true, |state| {
            let observed = WeakObservation {
                total,
                receiver_slot,
                receiver_player,
                weak,
                debilitate,
                krane_slots,
            };
            let amount = observed.prevention().unwrap_or_else(|_| {
                state.capture_failed("weak-policy");
                -1
            });
            state.record(|| Observation::WeakProjection { observed }, amount as u64);
            amount
        })
    })
}

/// # Safety
/// Non-null strings are readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_capture_failed(engine: u64, reason: *const c_char) {
    contain("capture_failed", (), || {
        with_engine(engine, (), true, |state| {
            // SAFETY: foreign strings satisfy the export contract.
            unsafe {
                with_c_str(reason, |reason| {
                    state.capture_failed(reason);
                    state.record(
                        || Observation::CaptureFailed {
                            reason: reason.into(),
                        },
                        0,
                    );
                })
            }
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_recording_begin(engine: u64) -> i32 {
    contain("recording_begin", 0, || {
        with_engine(engine, 0, false, |state| state.recording_begin())
    })
}

/// # Safety
/// A non-null buffer is writable for capacity bytes and does not alias inputs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_recording(
    engine: u64,
    buffer: *mut u8,
    capacity: i32,
) -> i32 {
    contain("recording", 0, || {
        with_engine(engine, 0, false, |state| {
            let json = state.recording_json();
            let Ok(size) = i32::try_from(json.len() + 1) else {
                return 0;
            };
            if !buffer.is_null() && capacity >= size {
                // SAFETY: capacity covers this byte count and the buffers cannot alias.
                unsafe {
                    std::ptr::copy_nonoverlapping(json.as_ptr(), buffer, json.len());
                    buffer.add(json.len()).write(0);
                }
            }
            size
        })
    })
}

/// # Safety
/// A non-null string is readable through a NUL and unmodified during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spire_profiler_replay(engine: u64, recording: *const c_char) -> i32 {
    contain("replay", 0, || {
        with_engine(engine, 0, false, |state| {
            // SAFETY: foreign string satisfies the export contract.
            unsafe { with_c_str(recording, |json| state.replay(json)) }
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_combat_discard(engine: u64) {
    contain("combat_discard", (), || {
        with_engine(engine, (), true, |state| {
            state.discard_combat();
            state.record(|| Observation::CombatDiscard, 0);
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn spire_profiler_modifier_credit(
    engine: u64,
    basis_low: u64,
    basis_high: u64,
    value_low: u64,
    value_high: u64,
    limit_low: u64,
    limit_high: u64,
    kind: i32,
) -> i64 {
    contain("modifier_credit", i64::MIN, || {
        with_engine(engine, i64::MIN, true, |state| {
            let observed = ModifierObservation {
                basis_low,
                basis_high,
                value_low,
                value_high,
                limit_low,
                limit_high,
                kind,
            };
            let amount = observed.credit().map(i64::from).unwrap_or_else(|_| {
                state.capture_failed("modifier-policy");
                i64::MIN
            });
            state.record(
                || Observation::ModifierProjection { observed },
                amount as u64,
            );
            amount
        })
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    fn json(engine: u64, recording: bool) -> String {
        let read = if recording {
            spire_profiler_recording
        } else {
            spire_profiler_snapshot
        };
        // SAFETY: null asks for length, then the vector owns the complete output range.
        unsafe {
            let length = read(engine, std::ptr::null_mut(), 0);
            assert!(length > 0);
            let mut bytes = vec![0; length as usize];
            assert_eq!(read(engine, bytes.as_mut_ptr(), length), length);
            assert_eq!(bytes.pop(), Some(0));
            String::from_utf8(bytes).expect("snapshot is UTF-8 JSON")
        }
    }

    fn started(engine: u64, epoch: u32) -> u64 {
        // SAFETY: string literals contain terminated immutable bytes.
        unsafe {
            spire_profiler_combat_started(
                engine,
                epoch,
                c"ENCOUNTER".as_ptr(),
                c"normal".as_ptr(),
                1234,
                2,
            )
        }
    }

    #[test]
    fn engines_own_independent_ledgers_and_reject_destroyed_or_foreign_ids() {
        let first = spire_profiler_engine_create();
        let second = spire_profiler_engine_create();
        assert_ne!(first, second);
        assert_eq!(started(first, 7), 7);
        assert_eq!(started(second, 7), 7);
        assert_eq!(spire_profiler_turn_started(first, 7), 1);
        assert_eq!(spire_profiler_turn_started(first, 7), 1);
        assert_eq!(spire_profiler_turn_started(second, 7), 1);
        let a: serde_json::Value = serde_json::from_str(&json(first, false)).expect("JSON parses");
        let b: serde_json::Value = serde_json::from_str(&json(second, false)).expect("JSON parses");
        assert_eq!(a["turns"], 2);
        assert_eq!(b["turns"], 1);
        std::thread::spawn(move || {
            let local = spire_profiler_engine_create();
            assert_ne!(first, local);
            assert_eq!(spire_profiler_turn_started(first, 7), 0);
            spire_profiler_engine_destroy(local);
        })
        .join()
        .expect("foreign-thread probe completes");
        spire_profiler_engine_destroy(first);
        assert_eq!(spire_profiler_turn_started(first, 7), 0);
        assert_eq!(spire_profiler_turn_started(second, 7), 1);
        spire_profiler_engine_destroy(second);
    }

    #[test]
    fn short_buffers_are_untouched_and_malformed_strings_decode_empty() {
        let engine = spire_profiler_engine_create();
        // SAFETY: null input is permitted, malformed input remains readable and terminated.
        unsafe {
            assert_eq!(
                spire_profiler_combat_started(engine, 1, std::ptr::null(), c"\xff".as_ptr(), 7, 1),
                1
            );
        }
        let mut buffer = [0x55; 4];
        // SAFETY: the four-byte array owns the declared output range.
        let required = unsafe { spire_profiler_snapshot(engine, buffer.as_mut_ptr(), 4) };
        assert!(required > 4);
        assert_eq!(buffer, [0x55; 4]);
        let summary: serde_json::Value =
            serde_json::from_str(&json(engine, false)).expect("JSON parses");
        assert_eq!(summary["encounter_id"], "");
        assert_eq!(summary["encounter_type"], "");
        spire_profiler_engine_destroy(engine);
    }

    #[test]
    fn panic_marks_coverage_and_quarantines_observations_until_next_combat() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 1);
        let before = CString::new(json(engine, true)).expect("JSON has no literal NUL");
        let replay = spire_profiler_engine_create();
        // SAFETY: CString owns the terminated recording through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, before.as_ptr()) }, 1);
        assert_eq!(
            contain("test", 0, || with_engine(engine, 0, true, |_| panic!(
                "fixture fault"
            ))),
            0
        );
        assert_eq!(spire_profiler_turn_started(engine, 1), 0);
        let trace = CString::new(json(engine, true)).expect("JSON has no literal NUL");
        let recording: serde_json::Value =
            serde_json::from_slice(trace.as_bytes()).expect("trace parses");
        assert_eq!(recording["truncated"], true);
        // SAFETY: CString owns the terminated recording through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, trace.as_ptr()) }, 0);
        let summary: serde_json::Value =
            serde_json::from_str(&json(engine, false)).expect("JSON parses");
        assert_eq!(summary["coverage"]["complete"], false);
        assert!(
            summary["coverage"]["reasons"]
                .as_array()
                .expect("reasons array")
                .iter()
                .any(|reason| reason == "native-panic")
        );
        assert_eq!(started(engine, 2), 2);
        assert_eq!(spire_profiler_turn_started(engine, 2), 1);
        spire_profiler_engine_destroy(replay);
        spire_profiler_engine_destroy(engine);
    }

    #[test]
    fn adversarial_panic_payload_cannot_unwind_into_the_host() {
        struct Payload;
        impl Drop for Payload {
            fn drop(&mut self) {
                panic!("payload destructor");
            }
        }
        assert_eq!(
            contain("payload", 19, || std::panic::panic_any(Payload)),
            19
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the end-to-end fixture keeps source, projection, and replay ordering visible"
    )]
    fn recorded_observations_replay_source_handles_and_snapshot_exactly() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 9);
        // SAFETY: immutable literals own every native string input through each call.
        unsafe {
            let a = spire_profiler_source_capture(engine, 9, 1, 11, c"A".as_ptr(), 0, 0, 0);
            let b = spire_profiler_source_capture(engine, 9, 1, 12, c"B".as_ptr(), 0, 1, 0);
            let mixed = spire_profiler_source_accumulate(engine, 9, a, 2, b, 5);
            assert_ne!(mixed, 0);
            let credit = spire_profiler_modifier_credit(engine, 0, 0, 3, 0, 0, 0, 1);
            assert_eq!(credit, 3);
            let modifiers = [BlockModifier { source: a, credit }];
            assert_eq!(
                spire_profiler_block_gained(engine, 9, 13, mixed, 1, modifiers.as_ptr(), 1, 0),
                1
            );
            for source in [a, b, mixed] {
                assert_eq!(spire_profiler_source_release(engine, source), 1);
            }
            let recaptured =
                spire_profiler_source_capture(engine, 9, 1, 11, c"A".as_ptr(), 0, 0, 0);
            assert!(
                recaptured > mixed,
                "replay must reproduce non-reused source serials"
            );
            let weak = spire_profiler_weak_prevention(engine, 7, 1, 1, 1, 0, 2);
            assert_eq!(weak, 5);
            assert_eq!(
                spire_profiler_damage_unattributed(engine, 9, 7, 0, 7, 1, 1, weak),
                1
            );
            assert_eq!(
                spire_profiler_weak_prevention(engine, i32::MAX, 1, 1, 1, 1, 2),
                -1
            );
            spire_profiler_capture_failed(engine, c"missing-hook".as_ptr());
        }
        assert_eq!(spire_profiler_combat_ended(engine, 9), 1);
        let expected = json(engine, false);
        let trace = CString::new(json(engine, true)).expect("JSON has no literal NUL");
        let replay = spire_profiler_engine_create();
        // SAFETY: CString owns the terminated recording through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, trace.as_ptr()) }, 1);
        assert_eq!(json(replay, false), expected);
        let mut corrupt: serde_json::Value =
            serde_json::from_slice(trace.as_bytes()).expect("trace parses");
        corrupt["observations"][1]["result"] = 0.into();
        let corrupt = CString::new(corrupt.to_string()).expect("JSON has no literal NUL");
        assert_eq!(
            // SAFETY: CString owns the terminated recording through replay.
            unsafe { spire_profiler_replay(replay, corrupt.as_ptr()) },
            0
        );
        assert_eq!(
            json(replay, false),
            expected,
            "failed replay leaves the old state untouched"
        );
        let mut corrupt: serde_json::Value =
            serde_json::from_slice(trace.as_bytes()).expect("trace parses");
        corrupt["observations"][4]["result"] = 99.into();
        let corrupt = CString::new(corrupt.to_string()).expect("JSON has no literal NUL");
        assert_eq!(
            // SAFETY: CString owns the terminated recording through replay.
            unsafe { spire_profiler_replay(replay, corrupt.as_ptr()) },
            0
        );
        spire_profiler_engine_destroy(engine);
        spire_profiler_engine_destroy(replay);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one malformed-packet table checks physical fallback and exact replay together"
    )]
    fn malformed_block_batches_preserve_physical_gains_and_replay_without_leaking() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 9);
        // SAFETY: literals own strings; arrays own each declared in-range buffer.
        // Invalid counts and misaligned pointers are rejected before dereference.
        unsafe {
            let base = spire_profiler_source_capture(engine, 9, 1, 1, c"BASE".as_ptr(), 0, 0, 0);
            let modifier = spire_profiler_source_capture(engine, 9, 1, 2, c"MOD".as_ptr(), 0, 1, 0);
            let valid = [BlockModifier {
                source: modifier,
                credit: 2,
            }];
            let negative = [BlockModifier {
                source: modifier,
                credit: -1,
            }];
            let excessive = [BlockModifier {
                source: modifier,
                credit: i64::MAX,
            }];
            let stale = [BlockModifier {
                source: 17,
                credit: 1,
            }];
            let misaligned = valid.as_ptr().cast::<u8>().wrapping_add(1).cast();
            for (buffer, count, incomplete) in [
                (std::ptr::null(), 1, 0),
                (valid.as_ptr(), -1, 0),
                (std::ptr::dangling(), 17, 0),
                (misaligned, 1, 0),
                (negative.as_ptr(), 1, 0),
                (excessive.as_ptr(), 1, 0),
                (stale.as_ptr(), 1, 0),
                (valid.as_ptr(), 1, 1),
                (valid.as_ptr(), 1, -1),
            ] {
                assert_eq!(
                    spire_profiler_block_gained(engine, 9, 3, base, 0, buffer, count, incomplete),
                    1
                );
                assert_eq!(
                    spire_profiler_damage_unattributed(engine, 9, 3, 0, 3, 1, 0, 0),
                    1
                );
            }
            assert_eq!(
                spire_profiler_block_gained(engine, 9, 0, base, 0, valid.as_ptr(), 1, 0),
                1
            );
            assert_eq!(
                spire_profiler_block_gained(engine, 9, 2, base, 0, std::ptr::null(), 0, 0),
                1
            );
            assert_eq!(
                spire_profiler_damage_unattributed(engine, 9, 2, 0, 2, 1, 0, 0),
                1
            );
        }
        let expected = json(engine, false);
        let summary: serde_json::Value = serde_json::from_str(&expected).expect("summary parses");
        assert_eq!(summary["block_total"], 29);
        assert_eq!(summary["cards"][0]["block_gained"], 29);
        assert_eq!(summary["cards"][0]["block_effective"], 2);
        assert_eq!(summary["cards"][1]["blk_modifier"], 0);
        assert_eq!(summary["cards"][2]["block_effective"], 27);
        assert_eq!(summary["cards"][2]["block_gained"], 0);
        assert_eq!(summary["coverage"]["complete"], false);
        let trace = CString::new(json(engine, true)).expect("JSON contains no NUL");
        let replay = spire_profiler_engine_create();
        // SAFETY: CString owns the terminated trace through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, trace.as_ptr()) }, 1);
        assert_eq!(json(replay, false), expected);
        spire_profiler_engine_destroy(engine);
        spire_profiler_engine_destroy(replay);
    }

    #[test]
    fn old_attribution_policies_are_rejected_without_reinterpreting_or_mutating_state() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 9);
        assert_eq!(spire_profiler_turn_started(engine, 9), 1);
        let expected = json(engine, false);
        let mut trace: serde_json::Value =
            serde_json::from_str(&json(engine, true)).expect("trace parses");
        assert_eq!(trace["policy_version"], 3);
        for policy in [1, 2] {
            trace["policy_version"] = policy.into();
            let input = CString::new(trace.to_string()).expect("JSON has no NUL");
            // SAFETY: CString owns the terminated trace through replay.
            assert_eq!(unsafe { spire_profiler_replay(engine, input.as_ptr()) }, 0);
            assert_eq!(json(engine, false), expected);
        }
        spire_profiler_engine_destroy(engine);
    }

    #[test]
    fn source_collection_does_not_consume_the_gameplay_recording_budget() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 1);
        for _ in 0..9_998 {
            // SAFETY: the literal owns the terminated identifier through capture.
            let source =
                unsafe { spire_profiler_source_capture(engine, 1, 1, 1, c"A".as_ptr(), 0, 0, 0) };
            assert_ne!(source, 0);
            assert_eq!(spire_profiler_source_release(engine, source), 1);
        }
        assert_eq!(spire_profiler_combat_ended(engine, 1), 1);
        let trace = json(engine, true);
        let doc: serde_json::Value = serde_json::from_str(&trace).expect("trace parses");
        assert_eq!(doc["truncated"], false);
        assert_eq!(
            doc["observations"].as_array().expect("entry array").len(),
            19_998
        );
        let trace = CString::new(trace).expect("JSON has no literal NUL");
        let replay = spire_profiler_engine_create();
        // SAFETY: CString owns the terminated recording through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, trace.as_ptr()) }, 1);
        assert_eq!(json(replay, false), json(engine, false));
        spire_profiler_turn_started(engine, 1);
        let overflow = CString::new(json(engine, true)).expect("JSON has no literal NUL");
        assert_eq!(
            // SAFETY: CString owns the terminated recording through replay.
            unsafe { spire_profiler_replay(replay, overflow.as_ptr()) },
            0
        );
        spire_profiler_engine_destroy(engine);
        spire_profiler_engine_destroy(replay);
    }

    #[test]
    fn recording_overflow_is_explicit_and_cannot_replay_as_complete() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 1);
        for _ in 0..10_010 {
            spire_profiler_turn_started(engine, 1);
        }
        let trace = json(engine, true);
        let doc: serde_json::Value = serde_json::from_str(&trace).expect("trace parses");
        assert_eq!(doc["truncated"], true);
        assert_eq!(
            doc["observations"]
                .as_array()
                .expect("observation array")
                .len(),
            10_000
        );
        let trace = CString::new(trace).expect("JSON has no literal NUL");
        // SAFETY: CString owns the terminated recording through replay.
        assert_eq!(unsafe { spire_profiler_replay(engine, trace.as_ptr()) }, 0);
        spire_profiler_engine_destroy(engine);
    }
    #[test]
    fn scalar_modifier_failures_do_not_erase_prior_successful_observations() {
        let engine = spire_profiler_engine_create();
        assert_eq!(spire_profiler_recording_begin(engine), 1);
        started(engine, 1);
        assert_eq!(
            spire_profiler_modifier_credit(engine, 0, 0, 3, 0, 0, 0, 0),
            3
        );
        assert_eq!(
            spire_profiler_modifier_credit(engine, 0, 0, 3, 0, 0, 0, 99),
            i64::MIN
        );
        let trace = CString::new(json(engine, true)).expect("JSON has no literal NUL");
        let replay = spire_profiler_engine_create();
        // SAFETY: CString owns the terminated trace through replay.
        assert_eq!(unsafe { spire_profiler_replay(replay, trace.as_ptr()) }, 1);
        assert_eq!(json(engine, false), json(replay, false));
        spire_profiler_engine_destroy(engine);
        spire_profiler_engine_destroy(replay);
    }
}
