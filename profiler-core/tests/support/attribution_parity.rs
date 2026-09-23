use profiler_core::data::events as e;
use profiler_core::data::state::{Combat, STATE, State};
use serde_json::{Value, json};

struct Source {
    #[cfg(feature = "baseline")]
    shares: Vec<(u64, u64)>,
    #[cfg(not(feature = "baseline"))]
    handle: u64,
}
impl Source {
    fn capture(id: &str, slot: i32) -> Self {
        Self::retained(e::source_capture(1, 1, 1, id, 0, slot, 0))
    }
    fn retained(handle: u64) -> Self {
        assert_ne!(handle, 0);
        #[cfg(feature = "baseline")]
        {
            let shares = (0..e::source_count(handle))
                .map(|index| {
                    (
                        e::source_destination(handle, index),
                        e::source_weight(handle, index),
                    )
                })
                .collect();
            assert_eq!(e::source_transfer_release(handle), 1);
            Self { shares }
        }
        #[cfg(not(feature = "baseline"))]
        {
            Self { handle }
        }
    }
    fn apply<T>(&self, f: impl FnOnce(u64) -> T) -> T {
        #[cfg(feature = "baseline")]
        let handle = {
            let handle = e::source_transfer_begin(1);
            assert_ne!(handle, 0);
            for &(destination, weight) in &self.shares {
                assert_eq!(e::source_transfer_add(handle, destination, weight), 1);
            }
            assert_eq!(e::source_transfer_seal(handle), 1);
            handle
        };
        #[cfg(not(feature = "baseline"))]
        let handle = self.handle;
        let result = f(handle);
        #[cfg(feature = "baseline")]
        assert_eq!(e::source_transfer_release(handle), 1);
        result
    }
}
fn reset() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        *state = State::default();
        let mut combat = Combat::default();
        combat.seq = 1;
        state.current = Some(combat);
    });
}
fn emit(label: &str, status: i32) {
    let snapshot: Value = STATE.with(|state| {
        let state = state.borrow();
        let c = state.current.as_ref().expect("fixture has combat");
        let rows: Vec<_> = c
            .cards
            .iter()
            .map(|r| {
                json!([
                    r.player,
                    r.id,
                    r.kind as u8,
                    r.plays,
                    r.damage_dealt,
                    r.damage_blocked,
                    r.block_gained,
                    r.block_effective,
                    r.dmg_direct,
                    r.dmg_attributed,
                    r.dmg_modifier,
                    r.blk_modifier,
                    r.mitigate_debuff,
                    r.mitigate_buff,
                    r.mitigate_str,
                    r.self_damage,
                    r.forge
                ])
            })
            .collect();
        json!({"case":label, "status":status,"rows":rows,"plays":c.plays,
            "generated_plays":c.generated_plays,"generation_triggers":c.generation_triggers,
            "turns":c.turns,"damage_received":c.damage_received,"block_total":c.block_total,
            "potions":c.potions_used})
    });
    println!("{snapshot}");
}
fn hit(
    source: &Source,
    modifier: Option<&Source>,
    total: i32,
    blocked: i32,
    kind: i32,
    slot: i32,
) -> i32 {
    let group = source.apply(|source| e::damage_calculation_begin(1, source, 1, 0, 999));
    assert_ne!(group, 0);
    if let Some(modifier) = modifier {
        modifier.apply(|source| e::damage_modifier_contribution(group, source, 3));
    }
    e::damage_result_append(group, total, total - blocked, blocked, kind, slot, 0);
    e::damage_calculation_commit(group)
}
fn main() {
    reset();
    for index in 0..67 {
        let source = Source::capture(&format!("BLOCK_{index}"), 0);
        emit(
            "pool-capacity-gain",
            source.apply(|s| e::block_gained(1, 1, s, 0)),
        );
    }
    emit(
        "pool-capacity-consume",
        e::damage_unattributed(1, 67, 0, 67, 1, 0, 0),
    );
    reset();
    let base = Source::capture("BASE", 0);
    let modifier = Source::capture("MOD", 0);
    for _ in 0..18 {
        emit(
            "pending-capacity",
            modifier.apply(|s| e::block_modifier_contribution(1, s, 1, 0)),
        );
    }
    emit("pending-gain", base.apply(|s| e::block_gained(1, 20, s, 0)));
    emit(
        "pending-consume",
        e::damage_unattributed(1, 20, 0, 20, 1, 0, 0),
    );
    reset();
    emit(
        "unobserved-block",
        e::damage_unattributed(1, 9, 0, 9, 1, 0, 0),
    );
    reset();
    let a = Source::capture("A", 0);
    let b = Source::capture("B", 1);
    a.apply(|s| e::block_modifier_contribution(1, s, 3, 0));
    emit("zero-gain", b.apply(|s| e::block_gained(1, 0, s, 0)));
    emit("after-zero", b.apply(|s| e::block_gained(1, 7, s, 0)));
    emit(
        "consume-after-zero",
        e::damage_unattributed(1, 7, 0, 7, 1, 0, 0),
    );
    a.apply(|s| e::block_modifier_contribution(1, s, 3, 0));
    emit("clear-with-pending", e::block_pool_clear(1, 0));
    emit("after-clear", b.apply(|s| e::block_gained(1, 7, s, 0)));
    emit(
        "consume-after-clear",
        e::damage_unattributed(1, 7, 0, 7, 1, 0, 0),
    );
    reset();
    let a = Source::capture("A", 0);
    let b = Source::capture("B", 1);
    let mut seed = 0xf34e_8397_a68e_f099_u64;
    for _ in 0..2048 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let amount = ((seed >> 16) % 20 + 1) as i32;
        let slot = ((seed >> 24) % 4) as i32;
        let source = if seed & 1 == 0 { &a } else { &b };
        let status = match seed % 10 {
            0 => source.apply(|s| e::block_gained(1, amount, s, slot)),
            1 => e::damage_unattributed(1, amount, 0, amount, 1, slot, 0),
            2 => hit(source, Some(&b), amount, amount / 3, 0, 4),
            3 => source.apply(|s| e::buff_mitigation(1, s, amount)),
            4 => source.apply(|s| e::forge(1, s, amount)),
            5 => source.apply(|s| e::osty_summoned(1, s, amount, slot)),
            6 => e::damage_unattributed(1, amount, amount, 0, 4, slot, 0),
            7 => e::block_pool_clear(1, slot),
            8 => source.apply(|s| e::block_modifier_contribution(1, s, amount, slot)),
            _ => e::turn_started(1),
        };
        emit("seeded", status);
    }
    reset();
    for index in 0..520 {
        let source = Source::capture(&format!("ROW_{index}"), index % 5);
        emit("row-capacity", source.apply(|s| e::forge(1, s, 1)));
    }
    reset();
    let a = Source::capture("A", 0);
    let b = Source::capture("B", 1);
    emit(
        "power-attach",
        a.apply(|s| e::power_attached(1, 70, "STRENGTH_POWER", 80, 0, 0, 1, s)),
    );
    for amount in 1..=70 {
        let source = if amount % 2 == 0 { &a } else { &b };
        emit(
            "power-layer-capacity",
            source.apply(|s| {
                e::power_amount_changed(1, 70, "STRENGTH_POWER", 80, 0, 0, amount, amount + 1, s)
            }),
        );
        let captured = Source::retained(e::source_capture(1, 2, 70, "", 2, 0, 0));
        emit("power-weight-use", captured.apply(|s| e::forge(1, s, 11)));
    }
    for instance in 71..=335 {
        emit(
            "power-instance-capacity",
            a.apply(|s| e::power_attached(1, instance, "STRENGTH_POWER", 80, 0, 0, 1, s)),
        );
    }
    for instance in 70..=335 {
        emit("power-remove", e::power_removed(1, instance));
    }
    reset();
    let a = Source::capture("A", 0);
    for instance in 1..=70 {
        emit(
            "generation-capacity",
            a.apply(|s| e::card_generated(1, instance, s, 1)),
        );
    }
    let mut plays = Vec::new();
    for execution in 1..=35 {
        let play = a.apply(|s| e::card_play_started(1, execution, 999, "PLAY", 0, 0, 1, 0, s));
        emit("play-capacity", i32::from(play != 0));
        if play != 0 {
            plays.push(play);
        }
    }
    for play in plays {
        emit("play-finish", e::card_play_finished(play));
    }
    reset();
    let a = Source::capture("A", 0);
    let b = Source::capture("B", 1);
    emit("weak-attach", a.apply(|s| e::power_attached(1, 50, "WEAK_POWER", 99, 1, 4, 3, s)));
    emit("weak-stack", b.apply(|s| e::power_amount_changed(1, 50, "WEAK_POWER", 99, 1, 4, 3, 8, s)));
    emit("strength-reduce", a.apply(|s| e::power_attached(1, 51, "STRENGTH_POWER", 99, 1, 4, -3, s)));
    for index in 0..32 {
        let weak = Source::retained(e::source_capture(1, 5, 50, "", 2, 4, 0));
        let group = e::damage_calculation_begin(1, 0, 0, 1, 100);
        emit("enemy-hit", e::damage_calculation_enemy_hit(group, 99, 10, -3));
        emit("weak-source", weak.apply(|s| e::damage_calculation_weak_source(group, s)));
        emit("weak-result", e::damage_result_append(group, 6, 4, 2, 1, index % 4, 2));
        emit("weak-commit", e::damage_calculation_commit(group));
    }
    for id in ["POISON_POWER", "DOOM_POWER"] {
        emit("duration-attach", a.apply(|s| e::power_attached(1, 60, id, 99, 1, 4, 7, s)));
        emit("duration-stack", b.apply(|s| e::power_amount_changed(1, 60, id, 99, 1, 4, 7, 12, s)));
        for amount in (0..12).rev() {
            let source = Source::retained(e::source_capture(1, 2, 60, "", 2, 4, 0));
            emit("duration-hit", hit(&source, None, amount, 0, 0, 4));
            emit("duration-decay", e::power_amount_changed(1, 60, id, 99, 1, 4, amount + 1, amount, 0));
        }
        emit("duration-remove", e::power_removed(1, 60));
    }
    emit("doom-attach", a.apply(|s| e::power_attached(1, 61, "DOOM_POWER", 99, 1, 4, 3, s)));
    emit("doom-stack", b.apply(|s| e::power_amount_changed(1, 61, "DOOM_POWER", 99, 1, 4, 3, 8, s)));
    for hp in [2, 4, 11] {
        let batch = e::doom_batch_begin(1);
        emit("doom-target", e::doom_target_capture(batch, 99, 61, hp));
        emit("doom-complete", e::doom_kills_completed(batch));
    }
    for orb in 1..=260 {
        emit("orb-capacity", a.apply(|s| e::orb_channeled(1, orb, s)));
    }
    let orb = Source::retained(e::source_capture(1, 3, 1, "", 4, 0, 0));
    let play = b.apply(|s| e::card_play_started(1, 1, 100, "ORB_PLAY", 0, 0, 1, 0, s));
    for _ in 0..3 {
        emit("orb-context", e::orb_context_begin(1, 1, play, 0));
        emit("orb-hit", hit(&orb, None, 7, 2, 0, 4));
    }
    emit("orb-play-finish", e::card_play_finished(play));
    reset();
    let a = Source::capture("A", 0);
    let b = Source::capture("B", 1);
    a.apply(|s| e::block_gained(1, 10, s, 0));
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let row = &mut state.current.as_mut().expect("fixture combat").cards[1];
        row.damage_dealt = i64::MAX;
        row.dmg_direct = i64::MAX;
    });
    let group = b.apply(|s| e::damage_calculation_begin(1, s, 1, 0, 999));
    e::damage_result_append(group, 10, 0, 10, 1, 0, 0);
    e::damage_result_append(group, 1, 1, 0, 0, 4, 0);
    emit(
        "late-arithmetic-rollback",
        e::damage_calculation_commit(group),
    );
}
