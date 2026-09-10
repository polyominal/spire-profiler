//! The synthetic self-test pipeline (`--spire-profiler-self-test`).

use super::*;
use crate::data::state::RunOutcome;
use crate::ui::snapshot;

struct Script {
    epoch: u64,
    leases: Vec<u64>,
}

impl Script {
    fn source(&mut self, instance: u64, id: &str, kind: i32, generation: i32) -> u64 {
        if self.leases.len() >= crate::data::state::caps::SELF_TEST_LEASES {
            crate::fail!("self-test source lease capacity exhausted");
            return 0;
        }
        let capture = if kind == 0 { 1 } else { 4 };
        let transfer = source_capture(self.epoch, capture, instance, id, kind, 0, generation);
        self.leases.push(transfer);
        transfer
    }

    fn play(&self, instance: u64, id: &str, generation: i32, source: u64) -> u64 {
        card_play_started(
            self.epoch, instance, instance, id, 0, 0, 1, generation, source,
        )
    }

    fn hit(&self, source: u64, role: i32, segment: i32, total: i32, blocked: i32) {
        let calculation = damage_calculation_begin(self.epoch, source, role, segment, 999);
        if damage_result_append(calculation, total, total - blocked, blocked, 0, 4, 0) != 1
            || damage_calculation_commit(calculation) != 1
        {
            damage_calculation_abort(calculation);
            damage_unattributed(self.epoch, total, total - blocked, blocked, 0, 4, 0);
        }
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        for transfer in self.leases.drain(..) {
            source_transfer_release(transfer);
        }
        STATE.with(|cell| {
            cell.borrow_mut().self_test_leases = Some(std::mem::take(&mut self.leases))
        });
    }
}

#[allow(clippy::too_many_lines)]
pub fn self_test() {
    if !STATE.with(|cell| cell.borrow().ready()) {
        return;
    }
    let Some(leases) = STATE.with(|cell| cell.borrow_mut().self_test_leases.take()) else {
        crate::fail!("self-test lease storage is already in use");
        return;
    };
    let mut script = Script { epoch: 0, leases };
    set_run_meta(1);
    run_started(
        "SELF_TEST_CHAR",
        0,
        "Standard",
        "SELF_TEST_SEED",
        0,
        "",
        1_786_579_200,
    );
    let epoch = combat_started("SELF_TEST", "test");
    crate::ui::panel::enable_for_selftest();
    script.epoch = epoch;
    let cracked = script.source(0, "CRACKED_CORE", 1, 0);
    orb_channeled(epoch, 1002, cracked);
    turn_started(epoch);
    let zap = script.source(101, "ZAP", 0, 0);
    let play = script.play(101, "ZAP", 0, zap);
    orb_channeled(epoch, 1001, zap);
    card_play_finished(play);
    orb_context_begin(epoch, 1001, 0, 0);
    script.hit(zap, 5, 1, 3, 0);
    turn_started(epoch);
    let defend = script.source(102, "DEFEND", 0, 0);
    let play = script.play(102, "DEFEND", 0, defend);
    block_gained(epoch, 5, defend, 0);
    card_play_finished(play);
    damage_unattributed(epoch, 8, 3, 5, 1, 0, 0);
    potion_used(epoch);
    let bash = script.source(103, "BASH", 0, 0);
    let play = script.play(103, "BASH", 0, bash);
    script.hit(bash, 1, 0, 6, 3);
    card_play_finished(play);
    let cloak = script.source(104, "CLOAK_AND_DAGGER", 0, 0);
    let play = script.play(104, "CLOAK_AND_DAGGER", 0, cloak);
    card_generated(epoch, 5001, cloak, 1);
    card_play_finished(play);
    let dualcast = script.source(105, "DUALCAST", 0, 0);
    let play = script.play(105, "DUALCAST", 0, dualcast);
    orb_context_begin(epoch, 1002, play, 0);
    script.hit(cracked, 5, 1, 8, 8);
    orb_context_begin(epoch, 1002, play, 0);
    script.hit(dualcast, 5, 0, 8, 8);
    card_play_finished(play);
    let shiv = script.source(5001, "SHIV", 0, 1);
    let play = script.play(5001, "SHIV", 1, shiv);
    card_play_finished(play);
    let furnace = script.source(106, "FURNACE", 0, 0);
    forge(epoch, furnace, 2);
    drop(script);
    combat_ended(epoch);
    run_ended(RunOutcome::Victory);
    snapshot::chart_self_test();
}
