use super::*;
use crate::data::persistence::test_support::write_store_file;
use crate::data::state::RunOutcome;

#[test]
fn exhausted_combat_ids_preserve_records_and_clear_active_state() {
    for ending in ["empty", "interrupted", "completed"] {
        let written = ending != "empty";
        let base = wiped_dir(&format!("combat-ids-{ending}"));
        write_store_file(&base, 0, u32::MAX - 1, "reserved filename, corrupt content");
        test_reset();
        init(&base);
        run_started("IRONCLAD", 0, "Standard", "LAST", 0, "", 1000);
        context_begin("OUTER", 1, 0);
        combat_started("LAST", "test");
        assert_eq!(
            STATE.with(|s| s.borrow().current.as_ref().unwrap().seq),
            u32::MAX
        );
        if written {
            card_play_started("STRIKE", 0, 1, 0, 0);
            block_gained(5, "STRIKE", 0, 0);
            orb_channeled(1, 0);
            card_generated(2, "STRIKE", 0, 0);
            enemy_hit_context(6, 1);
        }
        if ending == "completed" {
            card_play_finished(0);
            combat_ended();
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
                assert!(state.last_source.is_none());
                assert!(state.orb_sources.is_empty());
                assert!(state.generated_instances.is_empty());
                assert!(state.enemy_hit.is_none());
                assert_eq!(state.context_stack.active().unwrap().id, "OUTER");
                assert_eq!(state.run_combats, u32::from(written));
            });
            card_play_finished(0);
            damage_dealt(DamageDealt {
                total: 9,
                unblocked: 9,
                ..DamageDealt::default()
            });
            block_gained(9, "STALE", 0, 0);
            combat_ended();
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
        context_end();
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
    let base = wiped_dir("combat-ids-exhausted-boot");
    write_store_file(&base, 0, u32::MAX, "reserved filename, corrupt content");
    test_reset();
    init(&base);
    for _ in 0..3 {
        combat_started("EXHAUSTED", "test");
        combat_ended();
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
        let base = wiped_dir(&format!("run-ids-{source}"));
        test_reset();
        init(&base);
        run_started("IRONCLAD", 0, "Standard", "PREVIOUS", 0, "", 1000);
        combat_started("SAVED", "test");
        turn_started();
        card_play_started("STRIKE", 0, 1, 0, 0);
        card_play_finished(0);
        combat_ended();
        let saved = read_test_file(&base, "runs/1/1.json");
        combat_started("STALE", "test");
        card_play_started("STALE", 0, 1, 0, 0);
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
            card_play_finished(0);
            damage_dealt(DamageDealt {
                total: 9,
                unblocked: 9,
                ..DamageDealt::default()
            });
            combat_ended();
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
        combat_ended();
        let outside: serde_json::Value =
            serde_json::from_str(&read_test_file(&base, "runs/0/3.json")).unwrap();
        assert!(outside.get("run").is_none());
    }
}

#[test]
fn final_run_id_resumes_after_fresh_allocation_exhaustion() {
    let base = wiped_dir("run-id-last-resume");
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
    turn_started();
    card_play_started("STRIKE", 0, 1, 0, 0);
    block_gained(5, "STRIKE", 0, 0);
    card_play_finished(0);
    combat_ended();
    run_suspended();
    let saved = read_test_file(&base, &format!("runs/{}/1.json", u32::MAX));
    test_reset();
    init(&base);
    set_run_meta(2);
    context_begin("OUTER", 1, 0);
    for continued in [0, 1] {
        run_started("DEFECT", 0, "Standard", "DIFFERENT", continued, "", 1000);
        assert!(STATE.with(|s| s.borrow().run_ctx.is_none()));
    }
    combat_started("OUTSIDE_RUN", "test");
    block_gained(99, "", 0, 0);
    combat_ended();
    run_started("IRONCLAD", 0, "Standard", "LAST", 1, "", 1000);
    STATE.with(|cell| {
        let state = cell.borrow();
        assert_eq!(state.run_ctx.as_ref().unwrap().run.seq, u32::MAX);
        assert_eq!((state.run_combats, state.run_turns), (1, 1));
        assert_eq!(state.run_cards.len(), 1);
        assert_eq!(state.run_cards[0].block_gained, 5);
    });
    context_end();
    combat_started("RESUMED", "test");
    combat_ended();
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
