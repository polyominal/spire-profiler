//! Self-test pipeline tests: the synthetic run's store output, the UI
//! snapshot rows/footers, the chart-row payload, and the filter toggle.

use super::*;
use crate::data::state::PlayerFilter;
use crate::source_kind::SourceKind;
use crate::test_util::scratch_dir;
use crate::ui::ui_model::{Section, Segment, UiRow, UiTab};

fn assert_self_test_combat(combat: &CombatRec, combat_json: &serde_json::Value) {
    assert_eq!(combat.encounter_id, "SELF_TEST");
    assert_eq!(combat.turns, 2);
    assert_eq!(current_play_counters().0, 6);
    assert_eq!(
        STATE.with(|cell| cell
            .borrow()
            .current
            .as_ref()
            .expect("combat exists")
            .potions_used),
        1
    );
    assert_eq!(combat.result, "completed");
    let run = combat.run.as_ref().expect("combat record carries its run");
    assert_eq!(run.seq, 1);
    assert_eq!(run.character, "SELF_TEST_CHAR");
    assert!(combat_json[0].get("profile").is_none());
    assert!(combat_json[0].get("build").is_none());
    let zap = card_row(combat, "ZAP");
    assert_eq!(
        (zap.kind, zap.plays, zap.damage_dealt),
        (SourceKind::Card, 1, 3)
    );
    let defend = card_row(combat, "DEFEND");
    assert_eq!(
        (defend.kind, defend.plays, defend.damage_dealt),
        (SourceKind::Card, 1, 0)
    );
    assert_eq!((defend.block_gained, defend.block_effective), (5, 5));
    let bash = card_row(combat, "BASH");
    assert_eq!(
        (bash.kind, bash.plays, bash.damage_dealt),
        (SourceKind::Card, 1, 6)
    );
    let cloak = card_row(combat, "CLOAK_AND_DAGGER");
    assert_eq!((cloak.kind, cloak.plays), (SourceKind::Card, 1));
    let dualcast = card_row(combat, "DUALCAST");
    assert_eq!(
        (dualcast.kind, dualcast.plays, dualcast.damage_dealt),
        (SourceKind::Card, 1, 8)
    );
    assert_eq!(card_json(combat_json, "DUALCAST")["dmg_direct"], 8);
    let cracked = card_row(combat, "CRACKED_CORE");
    assert_eq!(
        (cracked.kind, cracked.plays, cracked.damage_dealt),
        (SourceKind::Relic, 0, 8)
    );
    assert_eq!(card_json(combat_json, "CRACKED_CORE")["dmg_attributed"], 8);
    assert_eq!(combat.damage_received, 8);
    assert!(combat.cards.iter().all(|c| c.id != "FIRE_POTION"));
    assert!(combat.cards.iter().all(|c| c.id != "SHIV"));
    assert_no_key(combat_json, "origin");
    let furnace = card_row(combat, "FURNACE_POWER");
    assert_eq!((furnace.kind, furnace.plays), (SourceKind::Power, 0));
    assert_eq!(card_json(combat_json, "FURNACE_POWER")["forge"], 2);
}

#[test]
fn self_test_pipeline_writes_combat_and_run_files() {
    let base = scratch_dir("spire-profiler-test");
    test_reset();
    init(&base);
    self_test();

    let (combat_rec, combat_doc) = read_all_combats(&base)
        .into_iter()
        .next()
        .expect("one combat record");
    assert_self_test_combat(&combat_rec, &serde_json::json!([combat_doc]));

    let runs = read_all_runs(&base);
    let run_doc = &runs[0];
    assert_eq!(run_doc["outcome"], "victory");
    assert_eq!(run_doc["profile"], 1);
    assert_eq!(run_doc["character"], "SELF_TEST_CHAR");
    assert!(run_doc.get("combats").is_none());
    assert!(run_doc.get("build").is_none());

    let snap = read_test_file(&base, "runs/1/1.json");
    let snap_json: serde_json::Value = serde_json::from_str(&snap).expect("store file parses");
    assert_eq!(snap_json["combat_id"], 1);
    assert!(!std::path::Path::new(&format!("{base}/runs/profile-1")).exists());
}

#[test]
fn combat_tab_rows_and_footer_render_generator_and_forge() {
    let base = scratch_dir("spire-profiler-test-ui");
    test_reset();
    init(&base);
    combat_started("UI_TEST", "test");
    card_play_started("INFERNAL_BLADE", 0, 1, 0, 0);
    card_generated(9001, "", 0, 0);
    card_play_finished(0);
    card_play_started("PILLAGE", 0, 1, 9001, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    forge("FURNACE_POWER", 2, 2, 0);

    let mut rows = [UiRow::default(); crate::ui::ui_model::MAX_UI_ROWS];
    let count = crate::ui::snapshot::ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(count, 1);
    assert_eq!(rows[0].kind, SourceKind::Card);
    assert_eq!(rows[0].name_str(), "INFERNAL_BLADE");
    let footer = crate::ui::snapshot::ui_footer_text(UiTab::Combat);
    assert!(footer.contains("forge 2"));
    combat_ended();
}

#[test]
fn chart_rows_payload_sections_ordering_self_rows() {
    let base = scratch_dir("spire-profiler-test-rows");
    test_reset();
    init(&base);
    seed_rows_combat_one();
    assert_combat_tab_rows();
    seed_rows_combat_two();
    assert_run_tab_rows();
    assert_rows_meta_and_footers();
}

fn seed_rows_combat_one() {
    run_started("IRONCLAD", 0, "Standard", "SEED_ROWS", 0, "", 0);
    combat_started("ROWS_TEST", "test");
    turn_started();

    card_play_started("ANGER", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_generated(9001, "", 0, 0);
    card_play_finished(0);
    card_play_started("ANGER", 1, 2, 0, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("ANGER", 0, 1, 9001, 0);
    damage_dealt(DamageDealt {
        total: 6,
        unblocked: 6,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("INFLAME", 0, 1, 0, 0);
    power_applied("STRENGTH_POWER", 2, 0, 1, 0);
    card_play_finished(0);
    card_play_started("BASH", 0, 1, 0, 0);
    damage_modifier_contribution("STRENGTH_POWER", 2, 2, 0);
    damage_dealt(DamageDealt {
        total: 10,
        unblocked: 10,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    card_play_started("CRIMSON_MANTLE", 0, 1, 0, 0);
    block_gained(10, "CRIMSON_MANTLE", 0, 0);
    damage_dealt(DamageDealt {
        total: 3,
        unblocked: 3,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 999,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    damage_dealt(DamageDealt {
        total: 10,
        blocked: 10,
        to_player: 1,
        receiver_hash: 999,
        dealer_hash: 42,
        ..DamageDealt::default()
    });
    card_generated(7001, "GHOST_RELIC", 1, 0);
    card_play_started("SHIV", 0, 1, 7001, 0);
    damage_dealt(DamageDealt {
        total: 2,
        unblocked: 2,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();
}

fn assert_combat_tab_rows() {
    let mut rows = [UiRow::default(); crate::ui::ui_model::MAX_UI_ROWS];
    let count = crate::ui::snapshot::ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert!(count > 0);
    assert_eq!(rows[0].section, Section::Damage);
    assert_eq!(rows[0].name_str(), "ANGER");
    assert_eq!(rows[0].value, 18);
    assert_eq!(rows[0].plays, 2);
    assert_eq!(rows[0].share_x10, 600); // 18/30
    assert_eq!(rows[0].seg_milli[Segment::Direct.index()], 1000);
    assert_eq!(rows[0].flags, 0);
    assert_eq!(rows[1].name_str(), "BASH");
    assert_eq!(rows[2].name_str(), "INFLAME");
    assert!(rows[2].seg_milli[Segment::Modifier.index()] > 0);
    assert_eq!(rows[2].seg_milli[Segment::Direct.index()], 0);
    let ghost_row = rows[..count]
        .iter()
        .find(|row| row.name_str() == "GHOST_RELIC");
    let ghost_row = ghost_row.expect("relic generator row");
    assert_eq!(ghost_row.section, Section::Damage);
    assert_eq!(ghost_row.kind, SourceKind::Relic);
    assert_eq!(ghost_row.flags, 0);
    assert_eq!(ghost_row.value, 2);
    assert!(!rows[..count].iter().any(|row| row.name_str() == "SHIV"));
    let di = rows[..count]
        .iter()
        .position(|row| {
            row.section == Section::Defense && row.flags & crate::ui::ui_model::ROW_FLAG_SELF == 0
        })
        .expect("positive defense row");
    assert_eq!(rows[di].name_str(), "CRIMSON_MANTLE");
    assert_eq!(rows[di].value, 10);
    assert_eq!(rows[di].seg_milli[Segment::Direct.index()], 1000);
    assert_ne!(rows[di + 1].flags & crate::ui::ui_model::ROW_FLAG_SELF, 0);
    assert_eq!(rows[di + 1].value, -3);
    assert_eq!(rows[di + 1].seg_milli[Segment::SelfDamage.index()], 428); // 3/7 (net value)
    assert_eq!(rows[di + 1].share_x10, 0);
}

fn seed_rows_combat_two() {
    combat_started("ROWS_TEST_2", "test");
    turn_started();
    turn_started();
    card_play_started("ANGER", 0, 1, 0, 0);
    damage_dealt(DamageDealt {
        total: 10,
        unblocked: 10,
        ..DamageDealt::default()
    });
    card_play_finished(0);
    combat_ended();
}

fn assert_run_tab_rows() {
    let mut run_rows = [UiRow::default(); crate::ui::ui_model::MAX_UI_ROWS];
    let rn = crate::ui::snapshot::ui_snapshot_rows(UiTab::Run, &mut run_rows);
    assert!(rn > 0);
    assert_eq!(run_rows[0].name_str(), "ANGER");
    assert_eq!(run_rows[0].value, 28);
}

fn assert_rows_meta_and_footers() {
    let meta = crate::ui::snapshot::ui_snapshot_meta(UiTab::Combat);
    assert_eq!(meta.turns, 2);
    assert_eq!(meta.total_damage, 10);
    assert_eq!(meta.dps_x10, 50);
    assert_eq!(meta.encounter_str(), "ROWS_TEST_2");
    let meta = crate::ui::snapshot::ui_snapshot_meta(UiTab::Run);
    assert_eq!(meta.turns, 3);
    assert_eq!(meta.combats, 2);
    assert_eq!(meta.total_damage, 40);
    assert_eq!(meta.dps_x10, 133); // 40/3

    let run_footer = crate::ui::snapshot::ui_footer_text(UiTab::Run);
    assert!(run_footer.contains("RUN TOTAL 40 dmg"));
    let combat_footer = crate::ui::snapshot::ui_footer_text(UiTab::Combat);
    assert!(combat_footer.contains("TOTAL 10 dmg"));
}

/// The avatar toggle: pressing a slot selects that player, pressing the
/// active slot again returns to All. A no-op outside a combat; a run
/// start resets a stale filter.
#[test]
fn panel_filter_toggle_selects_and_deselects_players() {
    let base = scratch_dir("spire-profiler-test-mp2-filter");
    test_reset();
    init(&base);

    assert_eq!(STATE.with(|s| s.borrow().player_filter), PlayerFilter::All);
    panel_filter_toggle(0);
    assert_eq!(STATE.with(|s| s.borrow().player_filter), PlayerFilter::All);

    run_started(
        "IRONCLAD,SILENT",
        0,
        "Standard",
        "SEED_FILTER",
        0,
        "111,222",
        0,
    );
    combat_started("MP2_FILTER_TEST", "test");
    panel_filter_toggle(0);
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        PlayerFilter::Player(0)
    );
    panel_filter_toggle(0);
    assert_eq!(STATE.with(|s| s.borrow().player_filter), PlayerFilter::All);
    panel_filter_toggle(1);
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        PlayerFilter::Player(1)
    );
    // A press on another avatar switches, it never stacks.
    panel_filter_toggle(0);
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        PlayerFilter::Player(0)
    );
    panel_filter_toggle(0);
    assert_eq!(STATE.with(|s| s.borrow().player_filter), PlayerFilter::All);
    combat_ended();
    run_ended(0);

    // A stale slot from a previous run never survives a run start.
    STATE.with(|s| s.borrow_mut().player_filter = PlayerFilter::Player(1));
    run_started("IRONCLAD", 0, "Standard", "SEED_FILTER_SOLO", 0, "", 0);
    combat_started("MP2_FILTER_SOLO", "test");
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        PlayerFilter::All,
        "run start must reset the filter"
    );
    panel_filter_toggle(0);
    assert_eq!(
        STATE.with(|s| s.borrow().player_filter),
        PlayerFilter::Player(0)
    );
    panel_filter_toggle(0);
    assert_eq!(STATE.with(|s| s.borrow().player_filter), PlayerFilter::All);
    combat_ended();
    run_ended(0);
}
