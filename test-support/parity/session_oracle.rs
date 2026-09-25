use serde_json::{Value, json};

use crate::data::events;
use crate::data::persistence::{CardStatKey, card_stat_from_rec};
use crate::data::records::CardRec;
use crate::data::run_history::{self, RunSelection};
use crate::data::state::{CardStat, STATE, SourceKind};

fn row(id: &str, slot: u8, damage: i64) -> CardStat {
    CardStat {
        id: id.into(),
        player: slot,
        damage_dealt: damage,
        dmg_direct: damage,
        ..CardStat::default()
    }
}

fn rows_json(rows: &[CardStat]) -> Value {
    Value::Array(rows.iter().map(|r| json!({
        "id": r.id, "kind": r.kind as u8, "player": r.player, "plays": r.plays,
        "damage_dealt": r.damage_dealt, "damage_blocked": r.damage_blocked,
        "block_gained": r.block_gained, "block_effective": r.block_effective,
        "forge": r.forge, "dmg_direct": r.dmg_direct, "dmg_attributed": r.dmg_attributed,
        "dmg_modifier": r.dmg_modifier, "blk_modifier": r.blk_modifier,
        "mitigate_debuff": r.mitigate_debuff, "mitigate_buff": r.mitigate_buff,
        "mitigate_str": r.mitigate_str, "self_damage": r.self_damage,
    })).collect())
}

fn merge_case(name: &str, before: Vec<CardStat>, incoming: Vec<CardStat>, team: bool) -> Value {
    let mut after = before.clone();
    let accepted = CardStat::merge_rows(
        &mut after,
        &incoming,
        if team {
            CardStatKey::TeamMerged
        } else {
            CardStatKey::PerSource
        },
    );
    json!({ "name": name, "team": team, "before": rows_json(&before),
        "incoming": rows_json(&incoming), "accepted": accepted, "after": rows_json(&after) })
}

fn observe(damage: i64) {
    STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let combat = state.current.as_mut().expect("fixture combat exists");
        combat.cards = vec![row("STRIKE", 0, damage)];
    });
}

fn snapshot(name: &str) -> Value {
    STATE.with(|cell| {
        let state = cell.borrow();
        let active = state.run_ctx.is_some();
        json!({ "name": name, "in_run": active,
            "combat": state.current.as_ref().map(|c| json!({
                "cards": rows_json(&c.cards), "turns": c.turns, "result": c.result().map(|r| r.name()).unwrap_or("active")
            })),
            "run": active.then(|| json!({"cards": rows_json(&state.run_cards), "turns": state.run_turns, "combats": state.run_combats})) })
    })
}

fn history_json(seed: &str, started: i64) -> Value {
    match run_history::select_run(seed, started, 0) {
        RunSelection::Empty => Value::Null,
        RunSelection::Selected(view) => {
            json!({ "character": view.character, "ascension": view.ascension,
            "game_mode": view.game_mode, "outcome": view.outcome.map(|o| o.name()).unwrap_or(""),
            "seed": view.seed, "started_at": view.started_at,
            "combats": view.combats.len(), "turns": view.combats.iter().map(|c| c.turns).sum::<u32>(),
            "damage_received": view.combats.iter().map(|c| c.damage_taken).sum::<i64>(),
            "cards": rows_json(&view.rollup),
            "players": view.players.iter().map(|p| json!({"slot": p.slot, "character": p.character})).collect::<Vec<_>>(),
            "player_cards": view.player_rollups.iter().map(|p| (p.slot.to_string(), rows_json(&p.cards))).collect::<serde_json::Map<_, _>>() })
        }
    }
}

#[test]
fn export_reference() {
    let mut cases = vec![
        merge_case(
            "players_stay_separate",
            vec![],
            vec![row("A", 0, 2), row("A", 1, 3)],
            false,
        ),
        merge_case(
            "team_merges_sources",
            vec![],
            vec![row("A", 0, 2), row("A", 1, 3)],
            true,
        ),
        merge_case(
            "first_seen_order",
            vec![row("B", 1, 2)],
            vec![row("A", 0, 3), row("B", 1, 5)],
            false,
        ),
    ];
    let mut first = row("A", 0, 0);
    first.plays = u32::MAX;
    let mut second = first.clone();
    second.plays = 1;
    cases.push(merge_case(
        "plays_overflow_uses_unknown",
        vec![first],
        vec![second],
        false,
    ));
    let full: Vec<_> = (0..1019)
        .map(|i| row(&format!("ROW_{i}"), (i % 4) as u8, 1))
        .collect();
    cases.push(merge_case(
        "capacity_preserves_five_creditors",
        full.clone(),
        (0..5)
            .map(|slot| row("OVERFLOW", slot, i64::from(slot) + 1))
            .collect(),
        false,
    ));
    cases.push(merge_case(
        "capacity_existing_still_merges",
        full,
        vec![
            row("ROW_0", 0, 10),
            row("EXCESS", 0, 7),
            row("EXCESS", 0, 8),
        ],
        false,
    ));
    cases.push(merge_case(
        "late_overflow_rolls_back_combat",
        vec![row("A", 0, i64::MAX)],
        vec![row("B", 1, 0), row("C", 1, 1)],
        false,
    ));
    let unknown = CardStat {
        id: "UNATTRIBUTED".into(),
        kind: SourceKind::Unknown,
        ..row("", 0, 2)
    };
    cases.push(merge_case(
        "unknown_stays_per_creditor",
        vec![unknown.clone()],
        vec![CardStat {
            player: 1,
            ..unknown
        }],
        true,
    ));
    let negative = CardStat {
        block_effective: -i64::MAX,
        ..row("NEGATIVE", 0, 0)
    };
    let positive = CardStat {
        block_effective: i64::MAX,
        ..row("POSITIVE", 0, 0)
    };
    let excess = CardStat {
        block_effective: 1,
        ..row("EXCESS", 0, 0)
    };
    cases.push(merge_case(
        "positive_and_negative_subtotals_are_separate",
        vec![negative, positive],
        vec![excess],
        false,
    ));
    for slot in 0..5 {
        let rec: CardRec = serde_json::from_value(
            json!({"id":"WIRE", "kind":255, "player":slot + 4, "damage_dealt":7, "dmg_direct":7}),
        )
        .expect("legacy fixture parses");
        let parsed = card_stat_from_rec(&rec);
        cases.push(merge_case(
            &format!("legacy_wire_{slot}"),
            vec![],
            vec![parsed],
            false,
        ));
    }

    let dir = crate::test_util::unique_dir("session-parity");
    events::test_reset();
    events::init(&dir);
    events::set_run_meta(0);
    events::run_started("IRONCLAD", 3, "Standard", "SESSION", 0, "1", 300);
    let mut lifecycle = vec![snapshot("run_started")];
    let first = events::combat_started("ONE", "Normal");
    observe(9);
    lifecycle.push(snapshot("active_first"));
    events::combat_ended(first);
    lifecycle.push(snapshot("completed_first"));
    events::combat_started("DISCARDED", "Normal");
    observe(50);
    events::run_suspended();
    lifecycle.push(snapshot("suspended"));
    let unfinished = history_json("SESSION", 300);
    events::run_started("IRONCLAD", 3, "Standard", "SESSION", 1, "1", 300);
    lifecycle.push(snapshot("resumed"));
    let second = events::combat_started("TWO", "Normal");
    observe(4);
    lifecycle.push(snapshot("active_second"));
    events::combat_ended(second);
    lifecycle.push(snapshot("completed_second"));
    events::run_ended(crate::data::state::RunOutcome::Victory);
    lifecycle.push(snapshot("ended"));
    let ended = history_json("SESSION", 300);

    events::run_started("IRONCLAD", 3, "Standard", "OLD", 0, "1", 400);
    events::combat_started("ABORTED", "Normal");
    observe(5);
    events::run_started("DEFECT", 1, "Standard", "NEW", 0, "1", 500);
    lifecycle.push(snapshot("replaced_run_before_combat"));
    events::combat_started("NEW_COMBAT", "Normal");
    lifecycle.push(snapshot("replaced_run_after_combat"));
    let replaced = history_json("OLD", 400);
    events::run_ended(crate::data::state::RunOutcome::Defeat);
    let active_end = history_json("NEW", 500);

    events::run_suspended();
    events::run_started("IRONCLAD", 0, "Standard", "BLANK", 0, "1", 600);
    events::run_ended(crate::data::state::RunOutcome::Victory);
    let blank = history_json("BLANK", 600);

    events::run_started("DEFECT", 1, "Standard", "SESSION", 1, "1", 300);
    let repeated = events::combat_started("THIRD", "Normal");
    observe(2);
    events::combat_ended(repeated);
    events::run_ended(crate::data::state::RunOutcome::Defeat);
    let first_header = history_json("SESSION", 300);

    events::run_started("IRONCLAD", 3, "Standard", "REPEATED", 0, "1", 700);
    events::combat_started("OLD_REPEAT", "Normal");
    observe(11);
    events::run_started("IRONCLAD", 3, "Standard", "REPEATED", 0, "1", 700);
    events::combat_started("NEW_REPEAT", "Normal");
    let same_identity = snapshot("same_identity_interrupted");
    events::run_suspended();
    let same_history = history_json("REPEATED", 700);

    let output = json!({ "baseline":"c928c477852e75ffcc35f8cd16c7ed03caad7c7f", "merge_cases":cases,
        "lifecycle":lifecycle, "unfinished_history":unfinished, "ended_history":ended,
        "replaced_history":replaced, "active_end_history":active_end,
        "blank_history":blank, "first_header_history":first_header, "same_identity":same_identity, "same_history":same_history });
    let path = std::env::var("PARITY_SESSION_OUTPUT").expect("session parity output path supplied");
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&output).expect("fixture serializes"),
    )
    .expect("session fixture writes");
    events::test_reset();
}
