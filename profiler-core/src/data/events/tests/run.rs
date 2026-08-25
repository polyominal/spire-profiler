//! Run-lifecycle tests: save+quit suspend and the resume rejoin
//! (fragments, seed, mid-combat discard), the defeat close-out, and the
//! `net_ids` roster parse.

use super::*;
use crate::data::records::CombatRec;
use crate::data::state::{self, RunOutcome, RunPlayer};
use crate::test_util::scratch_dir;

fn read_run(base: &Path) -> serde_json::Value {
    let runs = read_all_runs(base);
    assert_eq!(runs.len(), 1, "exactly one run record");
    serde_json::json!([runs.into_iter().next().expect("one run")])
}

#[test]
fn repeated_init_keeps_the_first_data_dir() {
    let first = scratch_dir("spire-profiler-test-init-first");
    let second = scratch_dir("spire-profiler-test-init-second");
    test_reset();
    init(&first);
    init(&second);

    assert!(
        STATE.with(|cell| cell.borrow().data_dir == first),
        "the second data dir must not replace the initialized one"
    );
    let log = read_test_file(&first, "profiler.log");
    assert_eq!(
        log.matches("profiler core initialized").count(),
        1,
        "a repeated init must not append another event-log line"
    );
    assert!(
        !second.join("profiler.log").exists(),
        "a repeated init must not write through the new argument"
    );
}

#[test]
fn resumed_run_records_the_seed() {
    let base = scratch_dir("spire-profiler-test-resume");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 7, "Standard", "SEED_123", 1, "", 0);
    combat_started("SELF_TEST", "test");
    combat_ended();
    run_ended(RunOutcome::Defeat);

    let runs = read_run(&base);
    assert_eq!(runs[0]["seed"], "SEED_123");
}

/// Clears `active` WITHOUT writing any record: the run is not over.
#[test]
fn suspend_clears_active_without_writing_a_run_record() {
    let base = scratch_dir("spire-profiler-test-suspend");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 0, "Standard", "SEED_SUSPEND", 0, "", 0);
    combat_started("SUSPEND_TEST", "test");
    combat_ended();
    run_suspended();

    assert!(!STATE.with(|s| s.borrow().run_ctx.active));
    assert!(
        !base.join("runs.jsonl").exists(),
        "a suspended run writes no record"
    );
}

/// A mid-combat suspend clears the filter too: with no current combat
/// there is no avatar row, so a selected player would strand the run tab.
#[test]
fn suspend_resets_the_player_filter() {
    let base = scratch_dir("spire-profiler-test-suspend-filter");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 0, "Standard", "SEED_SUSPEND_FILTER", 0, "", 0);
    combat_started("SUSPEND_FILTER", "test");
    panel_filter_toggle(0);
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        state::PlayerFilter::Player(0)
    );
    run_suspended();
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        state::PlayerFilter::All
    );
}

/// The screen-open flag must survive a no-op suspend.
#[test]
fn suspend_without_an_active_run_is_a_no_op() {
    let base = scratch_dir("spire-profiler-test-suspend-noop");
    test_reset();
    init(&base);
    run_history_select("SELF_TEST_SEED", 0, 1);
    run_suspended();
    assert!(
        crate::data::run_history::screen_open(),
        "a menu-level quit must not blank an open run-history screen"
    );
    run_started("IRONCLAD", 0, "Standard", "SEED_ENDED", 0, "", 0);
    combat_started("ENDED_TEST", "test");
    combat_ended();
    run_ended(RunOutcome::Defeat);
    run_suspended();
    assert!(!STATE.with(|s| s.borrow().run_ctx.active));
    let runs = read_run(&base);
    assert_eq!(runs[0]["outcome"], "defeat");
}

/// The resumed `run_started(continued=1)` finds no stale active run, so
/// the fragments rejoin under the original run id.
#[test]
fn suspend_then_continue_rejoins_without_a_spurious_defeat() {
    let base = scratch_dir("spire-profiler-test-suspend-resume");
    test_reset();
    init(&base);
    run_started("DEFECT", 1, "Standard", "SEED_SUSPEND_RESUME", 0, "", 0);
    combat_started("FRAG_ONE", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();
    run_suspended();
    assert!(!STATE.with(|s| s.borrow().run_ctx.active));

    run_started("DEFECT", 1, "Standard", "SEED_SUSPEND_RESUME", 1, "", 0);
    assert_eq!(STATE.with(|s| s.borrow().run_ctx.seq), 1);
    assert_eq!(STATE.with(|s| s.borrow().run_combats), 1);
    assert!(
        !base.join("runs.jsonl").exists(),
        "the continue must not close the suspended run as a defeat"
    );

    combat_started("FRAG_TWO", "test");
    card_play_started("DEFEND", 0, 1, 0, 0);
    block_gained(5, "DEFEND", 0, 0);
    card_play_finished(0);
    combat_ended();
    run_ended(RunOutcome::Abandoned); // the run really ends now
    let runs = read_run(&base);
    assert_eq!(runs[0]["outcome"], "abandoned");
    assert!(runs[0]["ended_at"].is_i64());
    assert_eq!(read_all_combats(&base).len(), 2, "both fragments persist");
}

/// Both sessions' combats land in one directory; close writes one record.
#[test]
fn resumed_run_rejoins_its_fragment_and_rebuilds_the_summary() {
    let base = scratch_dir("spire-profiler-test-resume-fragments");
    test_reset();
    init(&base);
    run_started("DEFECT", 1, "Standard", "SEED_FRAG", 0, "", 0);
    combat_started("FRAG_ONE", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.run_ctx = state::RunContext::default();
        st.current = None;
    });
    run_started("DEFECT", 1, "Standard", "SEED_FRAG", 1, "", 0);
    assert_eq!(STATE.with(|s| s.borrow().run_ctx.seq), 1);
    STATE.with(|s| {
        let st = s.borrow();
        assert_eq!(st.run_combats, 1);
        assert!(
            st.run_cards
                .iter()
                .any(|c| c.id == "STRIKE" && c.damage_dealt == 6)
        );
    });
    combat_started("FRAG_TWO", "test");
    card_play_started("DEFEND", 0, 1, 0, 0);
    block_gained(5, "DEFEND", 0, 0);
    card_play_finished(0);
    combat_ended();
    run_ended(RunOutcome::Abandoned); // abandoned after the resumed fragment
    let runs = read_run(&base);
    assert_eq!(runs[0]["outcome"], "abandoned");
    assert_eq!(read_all_combats(&base).len(), 2, "both fragments persist");
}

#[test]
fn resumed_run_log_lines_keep_their_legacy_order() {
    let base = scratch_dir("spire-profiler-test-resume-log-order");
    test_reset();
    init(&base);
    run_started("DEFECT", 1, "Standard", "SEED_LOG_ORDER", 0, "", 0);
    combat_started("FRAG_ONE", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    card_play_finished(0);
    combat_ended();

    run_started(
        "DEFECT,DEFECT",
        1,
        "Standard",
        "SEED_LOG_ORDER",
        1,
        "1,2",
        0,
    );
    let log = read_test_file(&base, "profiler.log");
    let ended = log
        .find("run 1 ended: DEFECT (Standard), defeat\n")
        .expect("previous run ends");
    let resumed = log
        .find("run 1 resumed: 1 combats (0 turns)")
        .expect("resume merge renders");
    let started = log
        .find("run 1 started: DEFECT,DEFECT (ascension 1, Standard, continued)\n")
        .expect("started run renders");
    let roster = log
        .find("  roster: 2 players (slot 0 = 1 (DEFECT), slot 1 = 2 (DEFECT))\n")
        .expect("multiplayer roster renders");
    assert!(
        ended < resumed && resumed < started && started < roster,
        "run lifecycle log order changed:\n{log}"
    );
}

/// The game restarts the combat after loading, so it is never persisted.
#[test]
fn resumed_run_discards_the_unfinished_combat() {
    let base = scratch_dir("spire-profiler-test-resume-midcombat");
    test_reset();
    init(&base);
    run_started("DEFECT", 1, "Standard", "SEED_MID", 0, "", 0);
    combat_started("MID_COMBAT", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.run_ctx = state::RunContext::default();
        st.current = None;
    });
    run_started("DEFECT", 1, "Standard", "SEED_MID", 1, "", 0);
    let combats: Vec<CombatRec> = read_all_combats(&base)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert!(
        combats.iter().all(|c| c.encounter_id != "MID_COMBAT"),
        "the unfinished combat must not be persisted"
    );
    assert_eq!(STATE.with(|s| s.borrow().run_combats), 0);
    combat_started("MID_COMBAT", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 9,
        unblocked: 9,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();
    run_ended(RunOutcome::Defeat);
    read_run(&base);
    let combats: Vec<CombatRec> = read_all_combats(&base)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    assert_eq!(combats.len(), 1);
    assert_eq!(combats[0].cards[0].damage_dealt, 9);
}

/// `player_died` double-fires idempotently; the run closes as a loss.
#[test]
fn player_death_marks_the_combat_and_run_as_defeat() {
    let base = scratch_dir("spire-profiler-test-defeat");
    test_reset();
    init(&base);
    run_started("IRONCLAD", 0, "Standard", "SEED_DEFEAT", 0, "", 0);
    combat_started("DEFEAT_TEST", "test");
    card_play_started("STRIKE", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    damage_dealt(DamageDealt {
        total: 12,
        unblocked: 12,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });
    player_died(0);
    combat_ended();
    run_ended(RunOutcome::Defeat);

    let (combat, _) = read_combat(&base);
    assert_eq!(combat.result, "defeat");
    assert_eq!(combat.damage_received, 12);
    let runs = read_run(&base);
    assert_eq!(runs[0]["outcome"], "defeat");
}

/// `net_ids` pairs positionally with `character_ids`; mismatches truncate.
#[test]
fn roster_parses_from_net_ids_and_truncates() {
    let base = scratch_dir("spire-profiler-test-mp2-roster");
    test_reset();
    init(&base);
    run_started(
        "IRONCLAD,SILENT",
        0,
        "Standard",
        "SEED_ROSTER",
        0,
        "111,222",
        0,
    );
    let players = STATE.with(|s| s.borrow().run_ctx.players.clone());
    assert_eq!(
        players,
        vec![
            RunPlayer {
                slot: 0,
                net_id: "111".to_owned(),
                character: "IRONCLAD".to_owned(),
            },
            RunPlayer {
                slot: 1,
                net_id: "222".to_owned(),
                character: "SILENT".to_owned(),
            },
        ]
    );
    combat_started("ROSTER_TEST", "test");
    let stamped = STATE.with(|s| {
        s.borrow()
            .current
            .as_ref()
            .expect("combat exists")
            .players
            .clone()
    });
    assert_eq!(stamped.len(), 2);
    assert_eq!(stamped[1].slot, 1);
    combat_ended();
    run_ended(RunOutcome::Victory);
    run_started("A,B,C", 0, "Standard", "SEED_TRUNC", 0, "1,2", 0);
    let players = STATE.with(|s| s.borrow().run_ctx.players.clone());
    assert_eq!(players.len(), 2);
    assert_eq!(players[0].character, "A");
    assert_eq!(players[1].character, "B");
    run_started("IRONCLAD", 0, "Standard", "SEED_SOLO", 0, "", 0);
    let players = STATE.with(|s| s.borrow().run_ctx.players.clone());
    assert_eq!(
        players,
        vec![RunPlayer {
            slot: 0,
            net_id: String::new(),
            character: "IRONCLAD".to_owned(),
        }]
    );
}

/// The forwarded StartTime IS the run's identity; a 0 from the shim falls
/// back to the session clock.
#[test]
fn run_started_stamps_the_forwarded_start_time() {
    let base = scratch_dir("spire-profiler-test-start-time");
    test_reset();
    init(&base);
    run_started(
        "IRONCLAD",
        0,
        "Standard",
        "SEED_START_TIME",
        0,
        "",
        1_786_579_200,
    );
    assert_eq!(
        STATE.with(|s| s.borrow().run_ctx.started_at),
        1_786_579_200,
        "the forwarded StartTime is stamped verbatim"
    );
    run_started(
        "IRONCLAD",
        0,
        "Standard",
        "SEED_START_TIME",
        1,
        "",
        1_786_579_200,
    );
    assert_eq!(STATE.with(|s| s.borrow().run_ctx.started_at), 1_786_579_200);
    run_started("IRONCLAD", 0, "Standard", "SEED_START_TIME", 0, "", 0);
    let fallback = STATE.with(|s| s.borrow().run_ctx.started_at);
    assert!(
        fallback > 1_700_000_000,
        "the 0 fallback stamps the session clock ({fallback})"
    );
}
