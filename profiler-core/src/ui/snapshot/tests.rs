use super::*;
use crate::data::state::{State, StorePaths, TEAM_SLOT};
use crate::test_util::unique_dir;

fn reset_state() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

fn start_combat() {
    reset_state();
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.store_paths = Some(StorePaths::new(&unique_dir("ui-store")));
        st.current = Some(Combat {
            seq: 1,
            encounter_id: crate::test_util::text("BYGONE_EFFIGY"),
            encounter_type: crate::test_util::text("Elite"),
            ..Combat::default()
        })
        .into();
    });
}

fn push_card(id: &str, dmg: i64, blk: i64) {
    push_card_slot(id, dmg, blk, 0);
}

fn push_card_slot(id: &str, dmg: i64, blk: i64, player: u8) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let c = st.current.as_mut().expect("combat exists");
        c.cards.push(CardStat {
            player,
            id: crate::test_util::text(id),
            damage_dealt: dmg,
            dmg_direct: dmg,
            block_gained: blk,
            block_effective: blk,
            ..CardStat::default()
        });
    });
}

fn set_filter(filter: PlayerFilter) {
    STATE.with(|s| s.borrow_mut().player_filter = filter);
}

#[test]
fn snapshot_rows_orders_top_level_sources_and_self_rows() {
    start_combat();
    push_card("STRIKE", 9, 0);
    push_card("DEFEND", 0, 5);
    push_card("SHIV", 6, 0);
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(n, 3);
    assert_eq!(rows[0].name_str(), "STRIKE");
    assert_eq!(rows[0].flags, 0);
    assert_eq!(rows[0].value, 9);
    assert_eq!(rows[1].name_str(), "SHIV");
    assert_eq!(rows[1].flags, 0);
    assert_eq!(rows[2].name_str(), "DEFEND");
    assert_eq!(rows[2].section, Section::Defense);
}

#[test]
fn footer_text_includes_totals() {
    start_combat();
    push_card("STRIKE", 9, 0);
    let footer = ui_footer_text(UiTab::Combat);
    assert!(footer.contains("TOTAL 9 dmg"));
}

#[test]
fn rows_filter_by_player_filter_and_carry_the_player_slot() {
    start_combat();
    push_card_slot("STRIKE", 9, 0, 0);
    push_card_slot("STRIKE", 6, 0, 1);
    push_card_slot("MALEVOLENCE_POWER", 3, 0, TEAM_SLOT);
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];

    // Rows carry the owning slot, so same-id rows stay distinct.
    set_filter(PlayerFilter::All);
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(n, 3);
    let strikes: Vec<&UiRow> = rows[..n]
        .iter()
        .filter(|r| r.name_str() == "STRIKE")
        .collect();
    assert_eq!(strikes.len(), 2, "one row per player, never merged");
    assert!(strikes.iter().any(|r| r.player == 0));
    assert!(strikes.iter().any(|r| r.player == 1));
    assert!(rows[..n].iter().any(|r| r.player == TEAM_SLOT));

    set_filter(PlayerFilter::Player(0));
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(n, 1);
    assert_eq!((rows[0].player, rows[0].value), (0, 9));
    set_filter(PlayerFilter::Player(1));
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(n, 1);
    assert_eq!((rows[0].player, rows[0].value), (1, 6));
}

#[test]
fn run_tab_rows_filter_by_player_filter() {
    reset_state();
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.store_paths = Some(StorePaths::new(&unique_dir("ui-store")));
        // The run accumulator keys rows on (player, id, kind), so the
        // avatar toggle filters it the same way as the combat's cards.
        st.run_cards.push(CardStat {
            player: 0,
            id: crate::test_util::text("STRIKE"),
            plays: 2,
            damage_dealt: 9,
            dmg_direct: 9,
            ..CardStat::default()
        });
        st.run_cards.push(CardStat {
            player: 1,
            id: crate::test_util::text("STRIKE"),
            plays: 1,
            damage_dealt: 6,
            dmg_direct: 6,
            ..CardStat::default()
        });
    });
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    assert_eq!(
        ui_snapshot_rows(UiTab::Run, &mut rows),
        2,
        "All keeps both players' rows"
    );
    set_filter(PlayerFilter::Player(0));
    let n = ui_snapshot_rows(UiTab::Run, &mut rows);
    assert_eq!(n, 1);
    assert_eq!((rows[0].player, rows[0].value, rows[0].plays), (0, 9, 2));
    set_filter(PlayerFilter::Player(1));
    let n = ui_snapshot_rows(UiTab::Run, &mut rows);
    assert_eq!(n, 1);
    assert_eq!((rows[0].player, rows[0].value), (1, 6));
}

/// A roll-up row with the chart fields the run-history view carries.
fn rollup_card(id: &str, plays: u32, dmg: i64) -> CardStat {
    CardStat {
        id: crate::test_util::text(id),
        plays,
        damage_dealt: dmg,
        dmg_direct: dmg,
        ..CardStat::default()
    }
}

// The run-history panel feeds its view's roll-up through the same row
// builder the in-game Run tab uses: the run tab renders both sections
// with the per-section |value| ranking, and the row cap per section
// still applies to the arbitrary dataset.
#[test]
fn rows_from_an_arbitrary_dataset_match_the_run_tab_shape() {
    let cards = vec![
        rollup_card("STRIKE", 4, 70),
        rollup_card("DEMON_FORM", 1, 35),
        CardStat {
            id: crate::test_util::text("DEFEND"),
            plays: 2,
            block_gained: 15,
            block_effective: 15,
            ..CardStat::default()
        },
    ];
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows_from(&cards, &mut rows);
    // Damage ranks by value: STRIKE (70) before DEMON_FORM (35); the
    // zero-damage cards never appear in the damage section.
    assert_eq!(rows[0].name_str(), "STRIKE");
    assert_eq!(rows[0].section, Section::Damage);
    assert_eq!(rows[0].value, 70);
    assert_eq!(rows[1].name_str(), "DEMON_FORM");
    // Defense follows with the only positive defense contributor.
    let sections: Vec<Section> = rows[..n].iter().map(|r| r.section).collect();
    assert!(sections.contains(&Section::Defense));

    // The combat tab over the same dataset builds the same two sections.
    let n_combat = ui_snapshot_rows_from(&cards, &mut rows);
    assert_eq!(n_combat, n);
}

#[test]
fn rows_from_cap_each_section_at_max_rows_per_section() {
    let cards: Vec<CardStat> = (0..(ui_model::MAX_ROWS_PER_SECTION + 40) as u32)
        .map(|i| rollup_card(&format!("CARD{i}"), 1, i as i64 + 1))
        .collect();
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows_from(&cards, &mut rows);
    let damage_rows = rows[..n]
        .iter()
        .filter(|r| r.section == Section::Damage)
        .count();
    assert_eq!(damage_rows, ui_model::MAX_ROWS_PER_SECTION);
    // The cap takes the first MAX_ROWS_PER_SECTION candidates in
    // dataset order, then ranks them (the same bound the live chart
    // applies); the strongest of those leads the section.
    assert_eq!(
        rows[0].name_str(),
        format!("CARD{}", ui_model::MAX_ROWS_PER_SECTION - 1)
    );
    // The buffer never overflows: total rows stay within MAX_UI_ROWS.
    assert!(n <= ui_model::MAX_UI_ROWS);
}

/// A self-only source keeps its name/plays, not a hanging label.
#[test]
fn self_only_defense_row_renders_as_standalone_self_damage() {
    let cards = vec![
        CardStat {
            id: crate::test_util::text("DEFEND"),
            plays: 2,
            block_gained: 15,
            block_effective: 15,
            ..CardStat::default()
        },
        CardStat {
            id: crate::test_util::text("BLOODLETTING"),
            plays: 3,
            self_damage: 9,
            ..CardStat::default()
        },
    ];
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows_from(&cards, &mut rows);
    let defense: Vec<&UiRow> = rows[..n]
        .iter()
        .filter(|r| r.section == Section::Defense)
        .collect();
    // Exactly two defense rows: DEFEND's positive row and the
    // standalone BLOODLETTING self row (no phantom positive row).
    assert_eq!(defense.len(), 2);
    let solo = defense
        .iter()
        .find(|r| r.name_str() == "BLOODLETTING")
        .expect("standalone self row");
    assert_eq!(solo.value, -9);
    assert_eq!(solo.plays, 3);
    assert_eq!(solo.share_x10, 0);
    assert_ne!(solo.flags & ui_model::ROW_FLAG_SELF, 0);
    assert_ne!(solo.flags & ui_model::ROW_FLAG_SELF_SOLO, 0);
    assert!(solo.seg_milli[Segment::SelfDamage.index()] > 0);
    assert!(solo.seg_milli[Segment::Direct.index()] == 0);
}

/// A big HP price never tops the Defense chart; solo-row hover still
/// shows the full card detail (the source's upside matters).
#[test]
fn defense_section_ranks_self_costs_below_contributors() {
    let card = |id: &str, plays: u32, block: i64, self_damage: i64| CardStat {
        id: crate::test_util::text(id),
        plays,
        block_gained: block,
        block_effective: block,
        self_damage,
        ..CardStat::default()
    };
    let cards = vec![
        card("DEFEND", 2, 15, 0),
        card("OFFERING", 1, 0, 30),
        card("CRIMSON_MANTLE", 1, 10, 3),
        card("BLOODLETTING", 3, 0, 9),
    ];
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows_from(&cards, &mut rows);
    let defense: Vec<(usize, &UiRow)> = rows[..n]
        .iter()
        .enumerate()
        .filter(|(_, r)| r.section == Section::Defense)
        .collect();
    // OFFERING's 30 must not outrank DEFEND despite being the
    // section's largest |value|.
    let names: Vec<&str> = defense.iter().map(|(_, r)| r.name_str()).collect();
    assert_eq!(
        names,
        [
            "DEFEND",
            "CRIMSON_MANTLE",
            "CRIMSON_MANTLE",
            "OFFERING",
            "BLOODLETTING"
        ]
    );
    assert_eq!(defense[0].1.flags, 0);
    assert_eq!(defense[1].1.flags, 0);
    assert_eq!(
        defense[2].1.flags,
        ui_model::ROW_FLAG_SELF,
        "the hanging row is SELF only (no SOLO)"
    );
    assert_eq!(
        defense[3].1.flags,
        ui_model::ROW_FLAG_SELF | ui_model::ROW_FLAG_SELF_SOLO
    );
    assert_eq!(defense[3].1.value, -30);
    assert_eq!(defense[4].1.value, -9);
    let solo_detail = ui_row_detail_from_cards(&rows[..n], defense[3].0, &cards);
    assert!(
        solo_detail
            .stats
            .iter()
            .any(|s| s.label == "self dmg" && s.value == "30"),
        "solo hover shows the full detail: {solo_detail:?}"
    );
    let hanging_detail = ui_row_detail_from_cards(&rows[..n], defense[2].0, &cards);
    assert_eq!(hanging_detail.title, "CRIMSON_MANTLE");
    assert_eq!(
        hanging_detail.stats,
        vec![StatLine {
            label: "self dmg".to_owned(),
            value: "3".to_owned(),
            tone: StatTone::SelfDamage,
        }]
    );
}

#[test]
fn run_meta_from_rollup_and_combat_totals() {
    let cards = vec![rollup_card("STRIKE", 4, 70), rollup_card("DEFEND", 2, 0)];
    let m = ui_snapshot_meta_from_run(&cards, 6, 2, 15);
    assert_eq!(m.turns, 6);
    assert_eq!(m.plays, 6);
    assert_eq!(m.combats, 2);
    assert_eq!(m.total_damage, 70);
    assert_eq!(m.damage_taken, 15);
    assert_eq!(m.dps_x10, 116); // 70 * 10 / 6, truncating
    let no_dps = ui_snapshot_meta_from_run(&cards, 0, 0, 0);
    assert_eq!(no_dps.dps_x10, 0);
    assert_eq!(no_dps.plays, 6);
}

#[test]
fn row_detail_matches_the_rows_player_slot() {
    start_combat();
    push_card_slot("STRIKE", 6, 0, 0);
    push_card_slot("STRIKE", 9, 0, 1);
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];

    set_filter(PlayerFilter::Player(1));
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    let idx = (0..n)
        .position(|i| rows[i].name_str() == "STRIKE")
        .expect("STRIKE row under the filter");
    let detail = ui_row_detail_from_rows(UiTab::Combat, &rows[..n], idx);
    assert!(
        detail.stats.iter().any(|s| s.value == "9"),
        "detail must show P1's 9: {detail:?}"
    );

    set_filter(PlayerFilter::All);
    let n = ui_snapshot_rows(UiTab::Combat, &mut rows);
    assert_eq!(rows[0].player, 1);
    assert!(
        ui_row_detail_from_rows(UiTab::Combat, &rows[..n], 0)
            .stats
            .iter()
            .any(|s| s.value == "9")
    );
    assert!(
        ui_row_detail_from_rows(UiTab::Combat, &rows[..n], 1)
            .stats
            .iter()
            .any(|s| s.value == "6")
    );
}

/// Resolves against the caller's cards, never the live state.
#[test]
fn row_detail_from_cards_uses_the_given_dataset() {
    let cards = vec![CardStat {
        id: crate::test_util::text("STRIKE"),
        plays: 3,
        damage_dealt: 42,
        dmg_direct: 42,
        ..CardStat::default()
    }];
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    let n = ui_snapshot_rows_from(&cards, &mut rows);
    assert!(n > 0);
    let detail = ui_row_detail_from_cards(&rows[..n], 0, &cards);
    assert!(
        detail.title.contains("STRIKE"),
        "the row's own card: {detail:?}"
    );
    assert!(
        detail.stats.iter().any(|s| s.value == "42"),
        "the given dataset's numbers: {detail:?}"
    );
    assert!(ui_row_detail_from_cards(&rows[..n], 99, &cards).is_empty());
    assert!(ui_row_detail_from_cards(&rows[..n], 0, &[]).is_empty());
}
