use super::*;
use crate::data::persistence::test_support::write_store_file;
use crate::data::state::RunOutcome;
use crate::test_util::{SourceFixture, combat_epoch};

#[test]
fn exhausted_combat_ids_preserve_records_and_clear_active_state() {
    for ending in ["empty", "interrupted", "completed"] {
        let written = ending != "empty";
        let base = unique_dir(&format!("combat-ids-{ending}"));
        write_store_file(&base, 0, u32::MAX - 1, "reserved filename, corrupt content");
        test_reset();
        init(&base);
        run_started("IRONCLAD", 0, "Standard", "LAST", 0, "", 1000);
        let epoch = combat_started("LAST", "test");
        assert_eq!(
            STATE.with(|s| s.borrow().current.as_ref().unwrap().seq),
            u32::MAX
        );
        let source = source_capture(epoch, 0, 0, "", 0, 4, 0);
        assert_eq!(source_count(source), 1);
        let calculation = damage_calculation_begin(epoch, source, 0, 1, 999);
        assert_eq!(damage_calculation_enemy_hit(calculation, 42, 6, 1), 1);
        let play = if written {
            let card = SourceFixture::card("STRIKE", 0);
            let play = card.play();
            card.block(5, 0);
            let channeled = card.with_transfer(|transfer| orb_channeled(epoch, 1, transfer));
            assert_eq!(channeled, 1);
            card.generate(2);
            play
        } else {
            0
        };
        if ending == "completed" {
            assert_eq!(card_play_finished(play), 1);
            combat_ended(epoch);
        }
        let path = base.join(format!("runs/1/{}.json", u32::MAX));
        let mut persisted = None;
        for attempt in 0..3 {
            combat_started("EXHAUSTED", "test");
            STATE.with(|cell| {
                let state = cell.borrow();
                assert_eq!(state.next_combat_id, u32::MAX);
                assert!(state.current.is_none());
                assert!(state.per_player.is_empty());
                assert_eq!(state.run_combats, u32::from(written));
            });
            assert_eq!(source_count(source), -1);
            assert_eq!(card_play_finished(play), 0);
            assert_eq!(damage_calculation_abort(calculation), 0);
            assert_eq!(damage_unattributed(epoch, 9, 9, 0, 0, 4, 0), 0);
            assert_eq!(block_gained(epoch, 9, 0, 0), 0);
            combat_ended(epoch);
            let bytes = std::fs::read(&path).ok();
            if attempt == 0 {
                persisted = bytes;
            } else {
                assert_eq!(
                    bytes, persisted,
                    "exhaustion cannot rewrite the last combat"
                );
            }
        }
        let mut expected = vec![(0, u32::MAX - 1)];
        if written {
            expected.push((1, u32::MAX));
            let doc: serde_json::Value =
                serde_json::from_slice(persisted.as_ref().unwrap()).unwrap();
            assert_eq!(doc["result"], ending);
        } else {
            assert!(persisted.is_none());
        }
        assert_eq!(combat_ids(&base.join("runs")), expected);
    }
}

#[test]
fn exhausted_combat_ids_from_store_preserve_the_existing_file() {
    let base = unique_dir("combat-ids-exhausted-boot");
    write_store_file(&base, 0, u32::MAX, "reserved filename, corrupt content");
    test_reset();
    init(&base);
    for _ in 0..3 {
        let epoch = combat_started("EXHAUSTED", "test");
        assert_eq!(epoch, 0);
        assert_eq!(combat_ended(epoch), 0);
        assert!(STATE.with(|s| s.borrow().current.is_none()));
    }
    assert_eq!(combat_ids(&base.join("runs")), vec![(0, u32::MAX)]);
    assert_eq!(
        read_test_file(&base, &format!("runs/0/{}.json", u32::MAX)),
        "reserved filename, corrupt content"
    );
    assert!(!read_test_file(&base, "profiler.log").contains("started: EXHAUSTED"));
}

#[test]
fn exhausted_run_ids_close_previous_run_and_discard_stale_data() {
    for source in ["record", "directory"] {
        let base = unique_dir(&format!("run-ids-{source}"));
        test_reset();
        init(&base);
        run_started("IRONCLAD", 0, "Standard", "PREVIOUS", 0, "", 1000);
        combat_started("SAVED", "test");
        turn_started(combat_epoch());
        let card = SourceFixture::card("STRIKE", 0);
        card.finish(card.play());
        combat_ended(combat_epoch());
        let saved = read_test_file(&base, "runs/1/1.json");
        let stale_epoch = combat_started("STALE", "test");
        let stale_play = SourceFixture::card("STALE", 0).play();
        let stale_source = source_capture(stale_epoch, 0, 0, "", 0, 4, 0);
        assert_eq!(source_count(stale_source), 1);
        panel_filter_toggle(0);
        let sentinel = format!(r#"{{"run_id":{},"seed":"RESERVED"}}"#, u32::MAX);
        let directory = base.join(format!("runs/{}", u32::MAX));
        if source == "record" {
            std::fs::write(base.join("runs.jsonl"), &sentinel).unwrap();
        } else {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("reserved"), &sentinel).unwrap();
        }
        for _ in 0..3 {
            run_started("DEFECT", 0, "Standard", "EXHAUSTED", 0, "", 2000);
            STATE.with(|cell| {
                let state = cell.borrow();
                assert!(state.run_ctx.is_none());
                assert!(state.current.is_none());
                assert!(state.run_cards.is_empty());
                assert_eq!((state.run_combats, state.run_turns), (0, 0));
                assert_eq!(state.player_filter, PlayerFilter::All);
            });
            assert_eq!(source_count(stale_source), -1);
            assert_eq!(card_play_finished(stale_play), 0);
            assert_eq!(damage_unattributed(stale_epoch, 9, 9, 0, 0, 4, 0), 0);
            combat_ended(stale_epoch);
            run_ended(RunOutcome::Victory);
        }
        assert_eq!(read_test_file(&base, "runs/1/1.json"), saved);
        assert_eq!(combat_ids(&base.join("runs")), vec![(1, 1)]);
        let runs = read_all_runs(&base);
        assert_eq!(runs.len(), if source == "record" { 2 } else { 1 });
        assert_eq!(runs.last().unwrap()["run_id"], 1);
        assert_eq!(runs.last().unwrap()["outcome"], "defeat");
        if source == "record" {
            assert_eq!(
                read_test_file(&base, "runs.jsonl").lines().next().unwrap(),
                sentinel
            );
        } else {
            assert_eq!(
                std::fs::read_to_string(directory.join("reserved")).unwrap(),
                sentinel
            );
        }
        assert!(!read_test_file(&base, "profiler.log").contains("started: DEFECT"));
        combat_started("OUTSIDE_RUN", "test");
        combat_ended(combat_epoch());
        let outside: serde_json::Value =
            serde_json::from_str(&read_test_file(&base, "runs/0/3.json")).unwrap();
        assert!(outside.get("run").is_none());
    }
}

#[test]
fn final_run_id_resumes_after_fresh_allocation_exhaustion() {
    let base = unique_dir("run-id-last-resume");
    std::fs::create_dir_all(base.join(format!("runs/{}", u32::MAX - 1))).unwrap();
    test_reset();
    init(&base);
    set_run_meta(2);
    run_started("IRONCLAD", 0, "Standard", "LAST", 0, "", 1000);
    assert_eq!(
        STATE.with(|s| s.borrow().run_ctx.as_ref().unwrap().run.seq),
        u32::MAX
    );
    combat_started("LAST", "test");
    turn_started(combat_epoch());
    let card = SourceFixture::card("STRIKE", 0);
    let play = card.play();
    card.block(5, 0);
    card.finish(play);
    combat_ended(combat_epoch());
    run_suspended();
    let saved = read_test_file(&base, &format!("runs/{}/1.json", u32::MAX));
    test_reset();
    init(&base);
    set_run_meta(2);
    for continued in [0, 1] {
        run_started("DEFECT", 0, "Standard", "DIFFERENT", continued, "", 1000);
        assert!(STATE.with(|s| s.borrow().run_ctx.is_none()));
    }
    combat_started("OUTSIDE_RUN", "test");
    SourceFixture::relic("OUTER", 0).block(99, 0);
    combat_ended(combat_epoch());
    run_started("IRONCLAD", 0, "Standard", "LAST", 1, "", 1000);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.run_ctx.as_ref().unwrap().run.seq, u32::MAX);
        assert_eq!((state.run_combats, state.run_turns), (1, 1));
        assert_eq!(state.run_cards.len(), 1);
        assert_eq!(state.run_cards[0].block_gained, 5);
    });
    combat_started("RESUMED", "test");
    combat_ended(combat_epoch());
    run_ended(RunOutcome::Victory);
    assert_eq!(
        read_test_file(&base, &format!("runs/{}/1.json", u32::MAX)),
        saved
    );
    let runs = read_all_runs(&base);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], u32::MAX);
    assert_eq!(
        combat_ids(&base.join("runs")),
        vec![(u32::MAX, 1), (0, 2), (u32::MAX, 3)]
    );
    let outside: serde_json::Value =
        serde_json::from_str(&read_test_file(&base, "runs/0/2.json")).unwrap();
    assert_eq!(card_json(&outside, "OUTER")["block_gained"], 99);
}
