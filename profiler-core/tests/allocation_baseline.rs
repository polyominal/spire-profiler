#[path = "support/allocation.rs"]
mod allocation_support;

use profiler_core::data::state::RunOutcome;
use profiler_core::data::{events, persistence, records, run_history};
use profiler_core::test_util::{self, SourceFixture};
use profiler_core::ui::snapshot;
use profiler_core::ui::ui_model::{MAX_UI_ROWS, UiRow, UiTab};

fn measured<T>(name: &str, operation: impl FnOnce() -> T) -> T {
    let (result, counts) = allocation_support::measure(operation);
    eprintln!(
        "ACCOUNT {name}: alloc={} alloc_zeroed={} realloc={} dealloc={}",
        counts.alloc, counts.alloc_zeroed, counts.realloc, counts.dealloc
    );
    result
}

fn measured_drop<T>(name: &str, operation: impl FnOnce() -> T) {
    let counts = allocation_support::measure_and_drop(operation);
    eprintln!(
        "ACCOUNT {name}: alloc={} alloc_zeroed={} realloc={} dealloc={}",
        counts.alloc, counts.alloc_zeroed, counts.realloc, counts.dealloc
    );
}

fn start_combat(label: &str, seed: &str) -> u64 {
    let dir = test_util::unique_dir(label);
    events::test_reset();
    events::init(&dir);
    events::set_run_meta(1);
    events::run_started("IRONCLAD", 0, "Standard", seed, 0, "1", 1_234);
    events::combat_started(label, "Monster")
}

#[test]
fn allocation_accounting_calibrates_all_operations_and_scope_edges() {
    let calibration = allocation_support::calibration_counts();
    assert_eq!(
        calibration,
        allocation_support::AllocationCounts {
            alloc: 1,
            alloc_zeroed: 1,
            realloc: 1,
            dealloc: 2,
        },
        "direct allocator operations and both returned allocations must be observed"
    );
    assert!(!calibration.all_zero());

    let (inner, outer) = allocation_support::measure(allocation_support::calibration_counts);
    assert_eq!(inner, calibration);
    assert!(
        outer.alloc >= inner.alloc
            && outer.alloc_zeroed >= inner.alloc_zeroed
            && outer.realloc >= inner.realloc
            && outer.dealloc >= inner.dealloc,
        "outer snapshots must retain nested operation counts"
    );

    let (without_drop, without_drop_counts) =
        allocation_support::measure(|| allocation_support::CalibrationOnDrop);
    assert!(
        without_drop_counts.all_zero(),
        "measure must exclude destruction of its returned value"
    );
    drop(without_drop);
    let with_drop = allocation_support::measure_and_drop(|| allocation_support::CalibrationOnDrop);
    assert_eq!(with_drop, calibration);

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        allocation_support::measure_and_drop(|| {
            assert_eq!(allocation_support::calibration_counts(), calibration);
            panic!("calibration unwind");
        });
    }));
    assert!(unwind.is_err());
    let after_unwind = allocation_support::measure_and_drop(|| {
        assert_eq!(allocation_support::calibration_counts(), calibration);
    });
    assert_eq!(after_unwind, calibration);
}

#[test]
fn allocation_accounting_records_source_capture_and_attribution() {
    let epoch = start_combat("allocation-source", "SOURCE_ACCOUNTING");
    let transfer = measured(
        "source_capture first [pure attribution computation]",
        || events::source_capture(epoch, 1, 10, "STRIKE", 0, 0, 0),
    );
    assert_ne!(
        transfer, 0,
        "source capture must return a live transfer token"
    );
    let count = measured("source_count [pure attribution computation]", || {
        events::source_count(transfer)
    });
    assert!(count > 0, "captured transfer must contain a destination");
    let released = measured(
        "source_transfer_release [pure attribution computation]",
        || events::source_transfer_release(transfer),
    );
    assert_eq!(released, 1, "captured transfer must release exactly once");

    let before_stale = profiler_core::data::state::STATE.with(|cell| {
        let state = cell.borrow();
        (
            state.current.as_ref().map(|combat| combat.cards.len()),
            state.run_cards.len(),
        )
    });
    let stale_count = measured(
        "source_count stale token [pure expected failure computation]",
        || events::source_count(transfer),
    );
    assert_eq!(
        stale_count, -1,
        "released transfer must be rejected as stale"
    );
    let after_stale = profiler_core::data::state::STATE.with(|cell| {
        let state = cell.borrow();
        (
            state.current.as_ref().map(|combat| combat.cards.len()),
            state.run_cards.len(),
        )
    });
    assert_eq!(
        after_stale, before_stale,
        "stale rejection must not mutate state"
    );
    let following_transfer = measured(
        "source_capture after stale token [pure attribution computation]",
        || events::source_capture(epoch, 1, 11, "DEFEND", 0, 0, 0),
    );
    assert_ne!(
        following_transfer, 0,
        "a following source capture must succeed"
    );
    assert_eq!(events::source_transfer_release(following_transfer), 1);

    let source = SourceFixture::card("STRIKE", 0);
    measured_drop("source attribution first [pure event computation]", || {
        source.deal(6, 0)
    });
    measured_drop(
        "source attribution repeated [pure event computation]",
        || source.deal(6, 0),
    );
}

#[test]
fn allocation_accounting_records_snapshot_tooltip_and_merge() {
    let epoch = start_combat("allocation-compute", "COMPUTE_ACCOUNTING");
    let source = SourceFixture::card("STRIKE", 0);
    source.deal(6, 0);

    let mut rows = [UiRow::default(); MAX_UI_ROWS];
    let row_count = measured("snapshot first [pure snapshot computation]", || {
        snapshot::ui_snapshot_rows(UiTab::Combat, &mut rows)
    });
    assert!(row_count > 0, "captured source must produce a snapshot row");
    measured("snapshot repeated [pure snapshot computation]", || {
        snapshot::ui_snapshot_rows(UiTab::Combat, &mut rows)
    });
    measured_drop("tooltip first [pure tooltip computation]", || {
        snapshot::ui_row_detail_from_rows(UiTab::Combat, &rows[..row_count], 0)
    });
    measured_drop("tooltip repeated [pure tooltip computation]", || {
        snapshot::ui_row_detail_from_rows(UiTab::Combat, &rows[..row_count], 0)
    });

    let staged = measured(
        "finished_combat staging [pure computation, no persistence]",
        || test_util::finish_combat_for_allocation_probe(epoch),
    );
    let staged = staged.expect("finished combat staging must return the captured record");
    measured_drop("merge_into_run [pure computation, no persistence]", || {
        persistence::merge_into_run(&staged)
    });
    let merged_rows = profiler_core::data::state::STATE.with(|cell| cell.borrow().run_cards.len());
    assert!(
        merged_rows > 0,
        "finished record must merge into the active run"
    );
    measured_drop(
        "finished_combat staged return drop [pure destruction]",
        || drop(staged),
    );
}

#[test]
fn allocation_accounting_records_lifecycle_and_history_boundaries() {
    let dir = test_util::unique_dir("allocation-lifecycle");
    events::test_reset();
    measured_drop("init [setup + filesystem boundary]", || events::init(&dir));
    events::set_run_meta(1);
    measured_drop("run_started first [mixed lifecycle + filesystem]", || {
        events::run_started(
            "IRONCLAD",
            0,
            "Standard",
            "LIFECYCLE_ACCOUNTING",
            0,
            "1",
            1_234,
        )
    });
    let epoch = measured(
        "combat_started first [mixed lifecycle + filesystem state]",
        || events::combat_started("LIFECYCLE_ACCOUNTING", "Monster"),
    );
    let source = SourceFixture::card("STRIKE", 0);
    source.deal(6, 0);

    let ended = measured("combat_ended [mixed computation + persistence I/O]", || {
        events::combat_ended(epoch)
    });
    assert_eq!(ended, 1, "active combat must persist successfully");
    measured_drop("run_ended [mixed lifecycle + persistence I/O]", || {
        events::run_ended(RunOutcome::Victory)
    });

    let selected = measured(
        "run_history select first [mixed history scan + parse]",
        || run_history::select("LIFECYCLE_ACCOUNTING", 1_234, 1),
    );
    assert!(selected, "history selection must find the recorded run");
    let view = measured("selected_view [pure view clone]", || {
        run_history::selected_view()
    });
    assert!(view.is_some(), "selected history view must be available");
    measured_drop("selected_view return drop [pure destruction]", || {
        drop(view)
    });
    let selected_again = measured("run_history select repeated [mixed cached history]", || {
        run_history::select("LIFECYCLE_ACCOUNTING", 1_234, 1)
    });
    assert!(
        selected_again,
        "repeated history selection must retain its match"
    );
    measured_drop(
        "run_started repeated [mixed lifecycle + filesystem]",
        || events::run_started("IRONCLAD", 0, "Standard", "SECOND", 0, "1", 1_235),
    );

    let malformed = measured("malformed JSON [codec boundary failure path]", || {
        records::parse_combat_doc("{")
    });
    assert!(
        malformed.is_err(),
        "malformed JSON must stay a rejected codec input"
    );
}

#[test]
fn allocation_accounting_records_interrupted_and_prior_run_close() {
    let dir = test_util::unique_dir("allocation-interruption");
    events::test_reset();
    measured_drop("init [setup + filesystem boundary]", || events::init(&dir));
    events::set_run_meta(2);
    measured_drop("run_started initial [mixed lifecycle + filesystem]", || {
        events::run_started("IRONCLAD", 0, "Standard", "INTERRUPTION", 0, "1", 4_321)
    });
    let first_epoch = measured("combat_started initial [mixed lifecycle state]", || {
        events::combat_started("POPULATED", "Monster")
    });
    SourceFixture::card("STRIKE", 0).deal(6, 0);

    let interrupted_epoch = measured(
        "combat_started interruption [mixed computation + persistence I/O]",
        || events::combat_started("NEXT", "Monster"),
    );
    assert!(
        interrupted_epoch > first_epoch,
        "interruption must advance combat ID"
    );
    let interrupted_doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("runs/1/1.json"))
            .expect("interrupted combat record must be written"),
    )
    .expect("interrupted combat record must parse");
    assert_eq!(interrupted_doc["result"], "interrupted");
    assert_eq!(interrupted_doc["cards"][0]["damage_dealt"], 6);

    SourceFixture::card("DEFEND", 0).deal(4, 0);
    let ended = measured(
        "combat_ended after interruption [mixed computation + persistence I/O]",
        || events::combat_ended(interrupted_epoch),
    );
    assert_eq!(ended, 1, "the following combat must persist successfully");
    measured_drop(
        "run_started prior-run close [mixed lifecycle + persistence I/O]",
        || events::run_started("IRONCLAD", 0, "Standard", "AFTER_CLOSE", 0, "1", 4_322),
    );
    let run_doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("runs.jsonl")).expect("prior run record must be written"),
    )
    .expect("prior run record must parse");
    assert_eq!(run_doc["seed"], "INTERRUPTION");
    assert_eq!(run_doc["profile"], 2);
    assert_eq!(run_doc["outcome"], "defeat");
    assert_eq!(
        profiler_core::data::state::STATE
            .with(|cell| { cell.borrow().run_ctx.as_ref().map(|run| run.run.seq) }),
        Some(2)
    );
}

#[test]
fn allocation_accounting_records_suspend_and_resume_rebuild() {
    let dir = test_util::unique_dir("allocation-resume");
    events::test_reset();
    measured_drop("init [setup + filesystem boundary]", || events::init(&dir));
    events::set_run_meta(3);
    let seed = "RESUME_ACCOUNTING";
    let start_time = 5_432;
    measured_drop("run_started initial [mixed lifecycle + filesystem]", || {
        events::run_started("DEFECT", 1, "Standard", seed, 0, "1", start_time)
    });
    let epoch = measured(
        "combat_started before suspend [mixed lifecycle state]",
        || events::combat_started("SAVED", "Monster"),
    );
    SourceFixture::card("STRIKE", 0).deal(9, 0);
    assert_eq!(
        measured(
            "turn_started before suspend [pure lifecycle computation]",
            || { events::turn_started(epoch) }
        ),
        1
    );
    let ended = measured(
        "combat_ended before suspend [mixed computation + persistence I/O]",
        || events::combat_ended(epoch),
    );
    assert_eq!(ended, 1);
    measured_drop(
        "run_suspended [mixed logical reset + event log]",
        events::run_suspended,
    );
    assert!(
        !dir.join("runs.jsonl").exists() && dir.join("runs/1/1.json").exists(),
        "suspend must omit the run record while retaining the combat record"
    );

    measured_drop(
        "run_started continued [mixed resume scan + rebuild]",
        || events::run_started("DEFECT", 1, "Standard", seed, 1, "1", start_time),
    );
    assert!(
        profiler_core::data::state::STATE.with(|cell| {
            let state = cell.borrow();
            state.run_ctx.as_ref().is_some_and(|run| {
                run.run.seq == 1
                    && run.run.seed == seed
                    && run.run.profile == 3
                    && run.run.started_at == start_time
            }) && state.run_combats == 1
                && state.run_turns == 1
                && state
                    .run_cards
                    .iter()
                    .any(|row| row.id == "STRIKE" && row.damage_dealt == 9)
        }),
        "resume must rebuild its identity and totals"
    );
    measured_drop(
        "run_ended after resume [mixed lifecycle + persistence I/O]",
        || events::run_ended(RunOutcome::Victory),
    );
    let run_doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("runs.jsonl")).expect("resumed run record must exist"),
    )
    .expect("resumed run record must parse");
    assert_eq!(run_doc["seed"], seed);
    assert_eq!(run_doc["profile"], 3);
    assert_eq!(run_doc["started_at"], start_time);
}
