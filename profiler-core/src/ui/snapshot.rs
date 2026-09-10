//! The ui-snapshot family: the structured UiRow/UiMeta payloads and the
//! hover-detail/footer texts consumed by [`chart_layout`] and the panels.

use crate::data::state::{CardStat, Combat, PlayerFilter, STATE, State};
use crate::marker;
use crate::ui::tooltip::{RowDetail, StatLine, StatTone};
use crate::ui::ui_model::{self, SEG_COUNT, Section, Segment, UiMeta, UiRow, UiTab};
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
    }
    if card.block_gained > 0 {
        push(
            format!("block ({} eff)", card.block_effective),
            card.block_gained,
            StatTone::Direct(Section::Defense, card.kind),
        );
        push("blk mod".to_owned(), card.blk_modifier, StatTone::Modifier);
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

#[derive(Clone, Copy, Default)]
struct SectionView {
    value: i64,
    segs: [i64; SEG_COUNT],
}

fn section_view(section: Section, card: &CardStat) -> SectionView {
    let mut v = SectionView::default();
    match section {
        Section::Damage => {
            v.segs[Segment::Direct.index()] = card.dmg_direct;
            v.segs[Segment::Attributed.index()] = card.dmg_attributed;
            v.segs[Segment::Modifier.index()] = card.dmg_modifier;
            v.value = card.dmg_direct + card.dmg_attributed + card.dmg_modifier;
        }
        Section::Defense => {
            v.segs[Segment::Direct.index()] = card.block_effective;
            v.segs[Segment::Modifier.index()] = card.blk_modifier;
            v.segs[Segment::MitigateDebuff.index()] = card.mitigate_debuff;
            v.segs[Segment::MitigateBuff.index()] = card.mitigate_buff;
            v.segs[Segment::MitigateStr.index()] = card.mitigate_str;
            v.segs[Segment::SelfDamage.index()] = card.self_damage;
            v.value = card.block_effective
                + card.blk_modifier
                + card.mitigate_debuff
                + card.mitigate_buff
                + card.mitigate_str
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
}

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

fn cards_for_tab(st: &State, tab: UiTab) -> &[CardStat] {
    if tab == UiTab::Run {
        &st.run_cards
    } else {
        st.current.as_ref().map_or(&[], |c| &c.cards)
    }
}

/// The avatar row filters both tabs: the combat's cards and the run
/// accumulator carry per-player rows, and headline totals stay team-wide.
fn chart_dataset(tab: UiTab) -> Vec<CardStat> {
    STATE.with(|s| {
        let st = s.borrow();
        let filter = st.player_filter;
        cards_for_tab(&st, tab)
            .iter()
            .filter(|card| player_filter_keeps(filter, card))
            .cloned()
            .collect()
    })
}

/// Defense sorts standalone self-damage below every positive contributor:
/// what protected the player first, then what it cost.
pub fn ui_snapshot_rows(tab: UiTab, out: &mut [UiRow]) -> usize {
    if !STATE.with(|s| s.borrow().initialized) {
        return 0;
    }
    let cards = chart_dataset(tab);
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
        for i in Segment::ALL.iter() {
            let i = i.index();
            let seg = view.segs[i];
            if seg <= 0 {
                continue;
            }
            row.seg_milli[i] = (seg * 1000 / max_val).min(1000) as u16;
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
            0
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
            0
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
        ui_row_detail_from_cards(rows, flat_index, cards_for_tab(&st, tab))
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
        marker!(
            "chart self-test ({}): {} rows -> {} cmds, {} hit rows, height {}",
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
mod tests;
