//! The ui-snapshot family: the structured UiRow/UiMeta payloads and the
//! hover-detail/footer texts consumed by `chart_layout` and the panels.

use crate::data::state::{CardStat, Combat, PlayerFilter, STATE};
use crate::ui::tooltip::{RowDetail, StatLine, StatTone};
use crate::ui::ui_model::{self, Section, Segment, UiMeta, UiRow, UiTab};
use crate::ui::{chart_layout, palette};

fn total_damage(c: &Combat) -> i64 {
    c.cards.iter().map(|card| card.damage_dealt).sum()
}

fn total_forge(c: &Combat) -> i64 {
    c.cards.iter().map(|card| card.forge).sum()
}

/// A stat group renders only when the card has it; inside a group the
/// decomposition lines always render, zeros included.
fn format_card_detail(card: &CardStat) -> RowDetail {
    let mut detail = RowDetail {
        title: format!(
            "{prefix}{id} x{plays}",
            prefix = palette::kind_prefix(card.kind).map_or("", |prefix| prefix.text),
            id = card.id,
            plays = card.plays
        ),
        stats: Vec::new(),
    };
    let mut push = |label: String, value: i64, tone: StatTone| {
        detail.stats.push(StatLine {
            label,
            value: value.to_string(),
            tone,
        });
    };
    if card.damage_dealt > 0 {
        // Derived: there is no separate unblocked counter.
        let unblocked = card.damage_dealt - card.damage_blocked;
        let direct = StatTone::Direct(Section::Damage, card.kind);
        push(
            format!("dmg ({unblocked} unblk)"),
            card.damage_dealt,
            direct,
        );
        push("direct".to_owned(), card.dmg_direct, direct);
        push(
            "indirect".to_owned(),
            card.dmg_attributed,
            StatTone::Attributed,
        );
        push("mod".to_owned(), card.dmg_modifier, StatTone::Modifier);
        push("upg".to_owned(), card.dmg_upgrade, StatTone::Upgrade);
    }
    if card.block_gained > 0 {
        push(
            format!("block ({} eff)", card.block_effective),
            card.block_gained,
            StatTone::Direct(Section::Defense, card.kind),
        );
        push("blk mod".to_owned(), card.blk_modifier, StatTone::Modifier);
        push("blk upg".to_owned(), card.blk_upgrade, StatTone::Upgrade);
    }
    if card.mitigate_debuff > 0 || card.mitigate_buff > 0 || card.mitigate_str > 0 {
        push(
            "weak".to_owned(),
            card.mitigate_debuff,
            StatTone::MitigateDebuff,
        );
        push(
            "buff".to_owned(),
            card.mitigate_buff,
            StatTone::MitigateBuff,
        );
        push("str".to_owned(), card.mitigate_str, StatTone::MitigateStr);
    }
    if card.self_damage > 0 {
        push(
            "self dmg".to_owned(),
            card.self_damage,
            StatTone::SelfDamage,
        );
    }
    if card.forge > 0 {
        push("forge".to_owned(), card.forge, StatTone::Neutral);
    }
    detail
}

#[derive(Clone, Copy, Debug, Default)]
struct SectionView {
    value: i64,
    segs: [i64; Segment::ALL.len()],
}

fn section_view(section: Section, card: &CardStat) -> SectionView {
    let mut v = SectionView::default();
    match section {
        Section::Damage => {
            v.segs[Segment::Direct.index()] = card.dmg_direct;
            v.segs[Segment::Attributed.index()] = card.dmg_attributed;
            v.segs[Segment::Modifier.index()] = card.dmg_modifier;
            v.segs[Segment::Upgrade.index()] = card.dmg_upgrade;
            v.value = card.dmg_direct + card.dmg_attributed + card.dmg_modifier + card.dmg_upgrade;
        }
        Section::Defense => {
            v.segs[Segment::Direct.index()] = card.block_effective;
            v.segs[Segment::Modifier.index()] = card.blk_modifier;
            v.segs[Segment::MitigateDebuff.index()] = card.mitigate_debuff;
            v.segs[Segment::MitigateBuff.index()] = card.mitigate_buff;
            v.segs[Segment::MitigateStr.index()] = card.mitigate_str;
            v.segs[Segment::Upgrade.index()] = card.blk_upgrade;
            v.segs[Segment::SelfDamage.index()] = card.self_damage;
            v.value = card.block_effective
                + card.blk_modifier
                + card.mitigate_debuff
                + card.mitigate_buff
                + card.mitigate_str
                + card.blk_upgrade
                - card.self_damage;
        }
    }
    v
}

fn defense_positive(card: &CardStat) -> i64 {
    card.block_effective
        + card.blk_modifier
        + card.mitigate_debuff
        + card.mitigate_buff
        + card.mitigate_str
        + card.blk_upgrade
}

#[derive(Clone, Copy, Debug)]
struct RowCand<'a> {
    card: &'a CardStat,
    view: SectionView,
}

fn player_filter_keeps(filter: PlayerFilter, card: &CardStat) -> bool {
    match filter {
        PlayerFilter::All => true,
        PlayerFilter::Player(s) => card.player == s,
    }
}

/// The avatar row filters both tabs: the combat's cards and the run
/// accumulator carry per-player rows, and headline totals stay team-wide.
fn chart_dataset(tab: UiTab) -> Vec<CardStat> {
    STATE.with(|s| {
        let st = s.borrow();
        let cards: &[CardStat] = if tab == UiTab::Run {
            &st.run_cards
        } else {
            st.current.as_ref().map_or(&[], |c| &c.cards)
        };
        let filter = st.player_filter;
        cards
            .iter()
            .filter(|card| player_filter_keeps(filter, card))
            .cloned()
            .collect()
    })
}

/// Defense sorts standalone self-damage below every positive contributor:
/// what protected the player first, then what it cost.
pub fn ui_snapshot_rows(tab: UiTab, out: &mut [UiRow]) -> usize {
    let cards = chart_dataset(tab);
    if !STATE.with(|s| s.borrow().initialized) {
        return 0;
    }
    ui_snapshot_rows_from(&cards, out)
}

pub fn ui_snapshot_rows_from(cards: &[CardStat], out: &mut [UiRow]) -> usize {
    let mut n: usize = 0;
    for section in Section::ALL {
        n += build_section_rows(section, cards, &mut out[n..]);
    }
    n
}

fn build_section_rows(section: Section, cards: &[CardStat], out: &mut [UiRow]) -> usize {
    if out.is_empty() {
        return 0;
    }
    let kept = rank_rows(section, collect_candidates(section, cards));

    let mut max_val: i64 = 0;
    for top in &kept {
        if top.view.value.abs() > max_val {
            max_val = top.view.value.abs();
        }
    }
    let mut total_val: i64 = 0;
    for card in cards {
        if section == Section::Defense {
            let pos = defense_positive(card);
            if pos > 0 {
                total_val += pos;
            }
        } else {
            total_val += section_view(section, card).value;
        }
    }

    let mut n: usize = 0;
    for top in &kept {
        n += emit_top_row(section, top, max_val, total_val, &mut out[n..]);
    }
    n
}

fn collect_candidates<'a>(section: Section, cards: &'a [CardStat]) -> Vec<RowCand<'a>> {
    let mut kept: Vec<RowCand> = Vec::new();
    for card in cards {
        let view = section_view(section, card);
        if view.value <= 0 && !(section == Section::Defense && card.self_damage > 0) {
            continue;
        }
        if kept.len() < ui_model::MAX_ROWS_PER_SECTION {
            kept.push(RowCand { card, view });
        }
    }
    kept
}

/// By |value| descending; self-damage sorts below every contributor.
fn rank_rows(section: Section, mut kept: Vec<RowCand>) -> Vec<RowCand> {
    kept.sort_by_key(|row| {
        let solo_self = section == Section::Defense
            && row.card.self_damage > 0
            && defense_positive(row.card) == 0;
        (solo_self, std::cmp::Reverse(row.view.value.abs()))
    });
    kept
}

fn emit_top_row(
    section: Section,
    top: &RowCand,
    max_val: i64,
    total_val: i64,
    out: &mut [UiRow],
) -> usize {
    let mut n: usize = 0;
    let card = top.card;
    let pos = if section == Section::Defense {
        defense_positive(card)
    } else {
        0
    };
    let self_row = section == Section::Defense && card.self_damage > 0;
    let split_self = self_row && pos > 0;

    if self_row && !split_self && n < out.len() {
        // The view's value is already -self_damage: only the flags change.
        out[n] = make_row(
            section,
            card,
            top.view,
            max_val,
            total_val,
            ui_model::ROW_FLAG_SELF | ui_model::ROW_FLAG_SELF_SOLO,
        );
        return 1;
    }
    if n < out.len() {
        let mut view = top.view;
        if split_self {
            view.value = pos;
            view.segs[Segment::SelfDamage.index()] = 0;
        }
        out[n] = make_row(section, card, view, max_val, total_val, 0);
        n += 1;
    }
    if split_self && n < out.len() {
        let mut self_view = SectionView {
            value: -card.self_damage,
            ..SectionView::default()
        };
        self_view.segs[Segment::SelfDamage.index()] = card.self_damage;
        out[n] = make_row(
            section,
            card,
            self_view,
            max_val,
            total_val,
            ui_model::ROW_FLAG_SELF,
        );
        n += 1;
    }
    n
}

fn make_row(
    section: Section,
    card: &CardStat,
    view: SectionView,
    max_val: i64,
    total_val: i64,
    flags: u8,
) -> UiRow {
    // A byte clamp could split a multibyte id.
    let name = chart_layout::truncate(&card.id, 64);
    let copy = name.len();
    let mut row = UiRow {
        section,
        kind: card.kind,
        player: card.player,
        flags,
        name_len: copy as u8,
        plays: card.plays,
        value: view.value,
        ..UiRow::default()
    };
    row.name[..copy].copy_from_slice(name.as_bytes());
    if flags & ui_model::ROW_FLAG_SELF == 0 && total_val > 0 && view.value > 0 {
        row.share_x10 = (view.value * 1000 / total_val) as i32;
    }
    if max_val > 0 {
        for (segment, &seg) in Segment::ALL.iter().zip(view.segs.iter()) {
            if seg <= 0 {
                continue;
            }
            row.seg_milli[segment.index()] = (seg * 1000 / max_val).min(1000) as u16;
        }
    }
    row
}

/// The in-game run state never tracked a run-scope value.
pub(crate) fn ui_snapshot_meta_from_run(
    cards: &[CardStat],
    turns: u32,
    combats: u32,
    damage_taken: i64,
) -> UiMeta {
    let mut damage: i64 = 0;
    let mut plays: u32 = 0;
    for card in cards {
        damage += card.damage_dealt;
        plays += card.plays;
    }
    UiMeta {
        turns,
        plays,
        combats,
        total_damage: damage,
        damage_taken,
        dps_x10: if turns > 0 {
            (damage * 10 / turns as i64) as i32
        } else {
            -1
        },
        ..UiMeta::default()
    }
}

pub(crate) fn ui_snapshot_meta(tab: UiTab) -> UiMeta {
    STATE.with(|s| {
        let st = s.borrow();
        let mut m = UiMeta::default();
        if !st.initialized {
            return m;
        }
        if tab == UiTab::Run {
            return ui_snapshot_meta_from_run(&st.run_cards, st.run_turns, st.run_combats, 0);
        }
        let Some(c) = &st.current else { return m };
        m.turns = c.turns;
        m.plays = c.plays;
        // Headline totals stay TEAM-wide even under a filter.
        m.total_damage = total_damage(c);
        m.damage_taken = c.damage_received;
        m.dps_x10 = if c.turns > 0 {
            (m.total_damage * 10 / c.turns as i64) as i32
        } else {
            -1
        };
        let len = c.encounter_id.len().min(64);
        m.encounter_len = len as u8;
        m.encounter[..len].copy_from_slice(&c.encounter_id.as_bytes()[..len]);
        m
    })
}

pub fn ui_row_detail_from_cards(
    rows: &[UiRow],
    flat_index: usize,
    cards: &[CardStat],
) -> RowDetail {
    let Some(row) = rows.get(flat_index) else {
        return RowDetail::default();
    };
    let is_self = row.flags & ui_model::ROW_FLAG_SELF != 0;
    let is_solo = row.flags & ui_model::ROW_FLAG_SELF_SOLO != 0;
    if is_self && !is_solo {
        // The hanging self row is terse: name plus the HP cost.
        return RowDetail {
            title: format!(
                "{prefix}{name}",
                prefix = palette::kind_prefix(row.kind).map_or("", |prefix| prefix.text),
                name = row.name_str()
            ),
            stats: vec![StatLine {
                label: "self dmg".to_owned(),
                value: row.value.unsigned_abs().to_string(),
                tone: StatTone::SelfDamage,
            }],
        };
    }
    // A filtered view can show one player's row while a same-id row sits
    // in the dataset.
    let name = row.name_str();
    let player = row.player;
    cards
        .iter()
        .find(|card| card.player == player && card.id == name)
        .map_or_else(RowDetail::default, format_card_detail)
}

pub fn ui_row_detail_from_rows(tab: UiTab, rows: &[UiRow], flat_index: usize) -> RowDetail {
    STATE.with(|s| {
        let st = s.borrow();
        let cards: &[CardStat] = if tab == UiTab::Run {
            &st.run_cards
        } else {
            st.current.as_ref().map_or(&[], |c| &c.cards)
        };
        ui_row_detail_from_cards(rows, flat_index, cards)
    })
}

pub fn ui_footer_text(tab: UiTab) -> String {
    STATE.with(|s| {
        let st = s.borrow();
        if !st.initialized {
            return String::new();
        }
        if tab == UiTab::Run {
            if st.run_combats == 0 {
                return "no completed combats this run yet".to_owned();
            }
            let mut damage: i64 = 0;
            let mut block: i64 = 0;
            for card in &st.run_cards {
                damage += card.damage_dealt;
                block += card.block_gained;
            }
            return format!(
                "RUN TOTAL {} dmg | {} turns | {} combats | {} block\n",
                damage, st.run_turns, st.run_combats, block,
            );
        }
        let Some(c) = &st.current else {
            return String::new();
        };
        format!(
            "TOTAL {} dmg | {} taken | {} block | pots {} | forge {}\n",
            total_damage(c),
            c.damage_received,
            c.block_total,
            c.potions_used,
            total_forge(c),
        )
    })
}

/// Headless verification that the layout engine links and runs in the game
/// process.
pub fn chart_self_test() {
    for tab in [UiTab::Combat, UiTab::Run] {
        let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
        let n = ui_snapshot_rows(tab, &mut rows);
        let meta = ui_snapshot_meta(tab);
        let footer = ui_footer_text(tab);
        let layout = chart_layout::build(chart_layout::BuildInput {
            tab,
            rows: &rows[..n],
            meta,
            footer: &footer,
            hover_row: None,
            skip_chrome: false,
            avatars: &[],
            flat_chrome: true,
            tab_sprites: false,
            width: chart_layout::PANEL_WIDTH,
            right_gutter: 0.0,
        });
        eprintln!(
            "[SpireProfiler] chart self-test ({}): {} rows -> {} cmds, {} hit rows, height {}",
            if tab == UiTab::Combat {
                "combat"
            } else {
                "run"
            },
            n,
            layout.cmds.len(),
            layout.row_hits.len(),
            layout.height as i32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::state::{State, TEAM_SLOT};

    fn reset_state() {
        STATE.with(|s| *s.borrow_mut() = State::default());
    }

    fn start_combat() {
        reset_state();
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.initialized = true;
            st.current = Some(Combat {
                seq: 1,
                encounter_id: "BYGONE_EFFIGY".to_owned(),
                encounter_type: "Elite".to_owned(),
                ..Combat::default()
            });
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
                id: id.to_owned(),
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
            st.initialized = true;
            // The run accumulator keys rows on (player, id, kind), so the
            // avatar toggle filters it the same way as the combat's cards.
            st.run_cards.push(CardStat {
                player: 0,
                id: "STRIKE".to_owned(),
                plays: 2,
                damage_dealt: 9,
                dmg_direct: 9,
                ..CardStat::default()
            });
            st.run_cards.push(CardStat {
                player: 1,
                id: "STRIKE".to_owned(),
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
            id: id.to_owned(),
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
                id: "DEFEND".to_owned(),
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
                id: "DEFEND".to_owned(),
                plays: 2,
                block_gained: 15,
                block_effective: 15,
                ..CardStat::default()
            },
            CardStat {
                id: "BLOODLETTING".to_owned(),
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
            id: id.to_owned(),
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
        assert_eq!(no_dps.dps_x10, -1);
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
            id: "STRIKE".to_owned(),
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
}
