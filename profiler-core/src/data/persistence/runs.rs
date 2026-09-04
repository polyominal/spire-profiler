//! The run accumulator behind the Run Summary tab: the live per-combat
//! merge, the save+quit+resume rebuild, and the shared per-field fold.

use super::combat_doc::card_stat_from_rec;
use super::combats::{load_run_combat_docs, parse_combat_docs};
use crate::data::state::{self, CardStat, Combat, STATE};
use crate::fail;

/// The Run Summary tab survives a save+quit+resume.
pub fn rebuild_run_accumulator(seq: u32) -> (u32, u32) {
    let combats = parse_combat_docs(&load_run_combat_docs(seq));
    let mut cards: Vec<CardStat> = Vec::new();
    let mut turns = 0u32;
    let mut count = 0u32;
    for combat in combats {
        let Some(run) = &combat.run else { continue };
        if run.seq != seq {
            continue;
        }
        count += 1;
        turns += combat.turns;
        for rec in &combat.cards {
            upsert_card_stat(&mut cards, &card_stat_from_rec(rec), CardStatKey::PerSource);
        }
    }
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.run_turns += turns;
        state.run_combats += count;
        for card in &cards {
            upsert_card_stat(&mut state.run_cards, card, CardStatKey::PerSource);
        }
    });
    (count, turns)
}

/// Merges a finished combat's per-source stats into the run accumulator.
/// Only combats of the currently-active run merge.
pub fn merge_into_run(c: &Combat) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(run) = &c.run else { return };
        if !state.run_ctx.active || run.seq != state.run_ctx.seq {
            return;
        }
        state.run_turns += c.turns;
        state.run_combats += 1;
        for card in &c.cards {
            upsert_card_stat(&mut state.run_cards, card, CardStatKey::PerSource);
        }
    });
}

/// Which rows one upsert may merge into.
#[derive(Clone, Copy)]
pub(crate) enum CardStatKey {
    /// Per-source rows: the player slot is part of the key.
    PerSource,
    /// TEAM merge: every player's same-id rows fold into one row.
    TeamMerged,
}

/// Folds `card`'s stats into the first row matching the key, else appends
/// it as a new row; at [`state::caps::RUN_CARDS`] a new row is
/// fail-logged and dropped (existing rows still merge).
pub(crate) fn upsert_card_stat(cards: &mut Vec<CardStat>, card: &CardStat, key: CardStatKey) {
    let matches = |row: &CardStat| match key {
        CardStatKey::PerSource => {
            row.id == card.id && row.kind == card.kind && row.player == card.player
        }
        CardStatKey::TeamMerged => row.id == card.id && row.kind == card.kind,
    };
    if let Some(dst) = cards.iter_mut().find(|row| matches(row)) {
        merge_card_stat(dst, card);
        return;
    }
    if cards.len() >= state::caps::RUN_CARDS {
        fail!("card-stat table overflow; row '{}' dropped", card.id);
        return;
    }
    cards.push(card.clone());
}

/// Adds every numeric field of `src` into `dst`.
fn merge_card_stat(dst: &mut CardStat, src: &CardStat) {
    dst.plays += src.plays;
    dst.damage_dealt += src.damage_dealt;
    dst.damage_blocked += src.damage_blocked;
    dst.block_gained += src.block_gained;
    dst.block_effective += src.block_effective;
    dst.forge += src.forge;
    dst.dmg_direct += src.dmg_direct;
    dst.dmg_attributed += src.dmg_attributed;
    dst.dmg_modifier += src.dmg_modifier;
    dst.blk_modifier += src.blk_modifier;
    dst.mitigate_debuff += src.mitigate_debuff;
    dst.mitigate_buff += src.mitigate_buff;
    dst.mitigate_str += src.mitigate_str;
    dst.self_damage += src.self_damage;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::build_combat_json;
    use crate::data::persistence::test_support::*;
    use crate::data::records;
    use crate::data::state::{RunPlayer, caps};
    use crate::source_kind::SourceKind;

    #[test]
    fn two_players_same_id_cards_round_trip_and_stay_separate() {
        let mut c = synthetic_combat();
        c.players = vec![
            RunPlayer {
                slot: 0,
                net_id: "1".to_owned(),
                character: "IRONCLAD".to_owned(),
            },
            RunPlayer {
                slot: 1,
                net_id: "2".to_owned(),
                character: "SILENT".to_owned(),
            },
        ];
        c.cards = vec![
            CardStat {
                id: "STRIKE".to_owned(),
                kind: SourceKind::Card,
                player: 0,
                plays: 2,
                damage_dealt: 20,
                ..CardStat::default()
            },
            CardStat {
                id: "STRIKE".to_owned(),
                kind: SourceKind::Card,
                player: 1,
                plays: 1,
                damage_dealt: 9,
                ..CardStat::default()
            },
        ];

        let json = build_combat_json(&c);
        assert!(!json.contains(r#""players""#));
        let combat = records::parse_combat_doc(&json).expect("record parses");
        let rows = &combat.cards;
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].id.as_str(), rows[0].player), ("STRIKE", 0));
        assert_eq!((rows[1].id.as_str(), rows[1].player), ("STRIKE", 1));
        assert_eq!((rows[0].plays, rows[1].plays), (2, 1));

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.active = true;
            st.run_ctx.seq = 42;
            st.run_cards.clear();
        });
        merge_into_run(&c);
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_cards.len(), 2);
            assert!(st.run_cards.iter().any(|r| r.player == 0 && r.plays == 2));
            assert!(st.run_cards.iter().any(|r| r.player == 1 && r.plays == 1));
        });
    }

    #[test]
    fn old_combat_records_parse_as_slot_zero() {
        let old = r#"[{"combat_id":1,"encounter_id":"A","result":"completed","turns":2,
            "damage_received":5,"block_total":0,
            "run":{"seq":7,"character":"DEFECT","ascension":1,"game_mode":"Standard"},
            "cards":[{"id":"STRIKE","kind":0,"plays":1,"damage_dealt":6}]}]"#;
        let record = old
            .trim()
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .expect("single-record fixture");
        let combat = records::parse_combat_doc(record).expect("old record parses");
        assert_eq!(combat.cards[0].player, 0, "old rows read as slot 0");

        let dir = unique_dir("old-record-rebuild");
        let data = dir.join("data");
        init_state(&data);
        write_store_file(&data, 7, 1, record);
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        let (combats, turns) = rebuild_run_accumulator(7);
        assert_eq!((combats, turns), (1, 2));
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_cards.len(), 1);
            assert_eq!(st.run_cards[0].player, 0);
            assert_eq!(st.run_cards[0].plays, 1);
        });
    }

    #[test]
    fn merge_into_run_accumulates_for_active_run() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.active = true;
            st.run_ctx.seq = 42;
            st.run_turns = 1;
            st.run_combats = 1;
        });
        let c = synthetic_combat(); // run_seq 42, turns 5
        merge_into_run(&c);
        let mut c2 = synthetic_combat();
        c2.seq = 8;
        c2.turns = 3;
        merge_into_run(&c2);
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_turns, 9);
            assert_eq!(st.run_combats, 3);
            assert_eq!(st.run_cards.len(), 2);
            let omni = &st.run_cards[0];
            assert_eq!(omni.id, "OMNI_CARD");
            assert_eq!(omni.kind, SourceKind::Card);
            assert_eq!(omni.plays, 8); // 4 + 4 across two combats
            assert_eq!(omni.damage_dealt, 42); // 21 + 21
            let anchor = &st.run_cards[1];
            assert_eq!(anchor.id, "ANCHOR");
            assert_eq!(anchor.kind, SourceKind::Relic);
            assert_eq!(anchor.block_gained, 20); // 10 + 10
        });
    }

    #[test]
    fn merge_into_run_ignores_inactive_or_foreign_combats() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.active = true;
            st.run_ctx.seq = 42;
        });
        let mut c = synthetic_combat();
        c.run = None; // outside any run
        merge_into_run(&c);
        let mut foreign = synthetic_combat();
        foreign.run = Some(synthetic_run(99)); // another run
        merge_into_run(&foreign);
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_combats, 0);
            assert!(st.run_cards.is_empty());
        });
    }
    /// Writes a combat-array fixture as one store file per record, under
    /// the record's own run directory.
    fn write_store_fixture(data: &std::path::Path, array_text: &str) {
        let combats: serde_json::Value = serde_json::from_str(array_text).expect("fixture parses");
        for combat in combats.as_array().expect("fixture is an array") {
            let id = combat["combat_id"].as_u64().expect("combat_id");
            let run_seq = combat["run"]["seq"].as_u64().expect("run.seq");
            write_store_file(data, run_seq as u32, id as u32, &combat.to_string());
        }
    }

    #[test]
    fn rebuild_run_accumulator_folds_prior_fragments() {
        let dir = unique_dir("rebuild-accumulator");
        let data = dir.join("data");
        init_state(&data);
        write_store_fixture(
            &data,
            r#"[{"combat_id":1,"encounter_id":"A","result":"completed","turns":4,"damage_received":12,
                "block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S"},
                "cards":[
                  {"id":"STRIKE","kind":0,"plays":2,"damage_dealt":12,"block_gained":0,"block_effective":0,"heal":0},
                  {"id":"DEFEND","kind":0,"plays":1,"damage_dealt":0,"block_gained":5,"block_effective":5,"heal":0}
                ]},
               {"combat_id":2,"encounter_id":"B","result":"completed","turns":3,"damage_received":8,
                "block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S"},
                "cards":[
                  {"id":"STRIKE","kind":0,"plays":1,"damage_dealt":7,"block_gained":0,"block_effective":0,"heal":0}
                ]},
               {"combat_id":3,"encounter_id":"C","result":"completed","turns":1,"damage_received":1,
                "block_total":0,
                "run":{"seq":6,"character":"IRONCLAD","ascension":0,"game_mode":"Standard","seed":"T"},
                "cards":[{"id":"BASH","kind":0,"plays":9,"damage_dealt":99,"block_gained":0,"block_effective":0,"heal":0}]}
              ]"#,
        );
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        let (combats, turns) = rebuild_run_accumulator(5);
        assert_eq!((combats, turns), (2, 7));
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_combats, 2);
            assert_eq!(st.run_turns, 7);
            let strike = st
                .run_cards
                .iter()
                .find(|c| c.id == "STRIKE")
                .expect("STRIKE merged");
            assert_eq!(strike.plays, 3);
            assert_eq!(strike.damage_dealt, 19);
            let defend = st
                .run_cards
                .iter()
                .find(|c| c.id == "DEFEND")
                .expect("DEFEND merged");
            assert_eq!(defend.block_gained, 5);
            assert!(!st.run_cards.iter().any(|c| c.id == "BASH"));
        });
    }

    #[test]
    fn rebuild_run_accumulator_ignores_foreign_run_directories() {
        // A combat filed under runs/6/ but stamped run.seq 5 would pass
        // the seq filter, so its absence pins that runs/6/ was not read.
        let dir = unique_dir("rebuild-accumulator-foreign");
        let data = dir.join("data");
        init_state(&data);
        write_store_file(
            &data,
            5,
            1,
            r#"{"combat_id":1,"encounter_id":"A","result":"completed","turns":4,"damage_received":12,
                "block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S"},
                "cards":[{"id":"STRIKE","kind":0,"plays":2,"damage_dealt":12,
                          "block_gained":0,"block_effective":0,"heal":0}]}"#,
        );
        write_store_file(
            &data,
            6,
            4,
            r#"{"combat_id":4,"encounter_id":"D","result":"completed","turns":1,
                "damage_received":1,"block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S"},
                "cards":[{"id":"SNEAKY","kind":0,"plays":100,"damage_dealt":1,
                          "block_gained":0,"block_effective":0,"heal":0}]}"#,
        );
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        let (combats, turns) = rebuild_run_accumulator(5);
        assert_eq!(
            (combats, turns),
            (1, 4),
            "the foreign run's directory was not read by the rebuild"
        );
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_combats, 1);
            assert!(
                !st.run_cards.iter().any(|c| c.id == "SNEAKY"),
                "a same-seq combat in a foreign run's directory must not fold"
            );
        });
    }

    /// Compares every field explicitly, so a field added to `CardStat`
    /// must be added to the merge and to this assertion.
    fn assert_card_stat_eq(a: &CardStat, b: &CardStat) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.player, b.player);
        assert_eq!(a.plays, b.plays);
        assert_eq!(a.damage_dealt, b.damage_dealt);
        assert_eq!(a.damage_blocked, b.damage_blocked);
        assert_eq!(a.block_gained, b.block_gained);
        assert_eq!(a.block_effective, b.block_effective);
        assert_eq!(a.forge, b.forge);
        assert_eq!(a.dmg_direct, b.dmg_direct);
        assert_eq!(a.dmg_attributed, b.dmg_attributed);
        assert_eq!(a.dmg_modifier, b.dmg_modifier);
        assert_eq!(a.blk_modifier, b.blk_modifier);
        assert_eq!(a.mitigate_debuff, b.mitigate_debuff);
        assert_eq!(a.mitigate_buff, b.mitigate_buff);
        assert_eq!(a.mitigate_str, b.mitigate_str);
        assert_eq!(a.self_damage, b.self_damage);
    }

    #[test]
    fn rebuild_run_accumulator_matches_live_merge_field_by_field() {
        let dir = unique_dir("rebuild-accumulator-parity");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        init_state(&data);
        let c1 = synthetic_combat(); // run_seq 42
        let mut c2 = synthetic_combat();
        c2.seq = 8;
        c2.turns = 3;
        c2.cards[0].plays = 5; // non-trivial sums across the two combats
        c2.cards[1].block_gained = 3;

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_ctx.active = true;
            st.run_ctx.seq = 42;
            st.run_turns = 0;
            st.run_combats = 0;
            st.run_cards.clear();
        });
        merge_into_run(&c1);
        merge_into_run(&c2);
        let (live_cards, live_turns, live_combats) = STATE.with(|s| {
            let st = s.borrow();
            (st.run_cards.clone(), st.run_turns, st.run_combats)
        });

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        write_store_file(&data, 42, c1.seq, &build_combat_json(&c1));
        write_store_file(&data, 42, c2.seq, &build_combat_json(&c2));
        let (combats, turns) = rebuild_run_accumulator(42);
        assert_eq!((combats, turns), (live_combats, live_turns));
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_turns, live_turns);
            assert_eq!(st.run_combats, live_combats);
            assert_eq!(st.run_cards.len(), live_cards.len());
            for (rebuilt, expected) in st.run_cards.iter().zip(&live_cards) {
                assert_card_stat_eq(rebuilt, expected);
            }
        });
    }

    #[test]
    fn upsert_key_chooses_whether_player_splits_rows() {
        let row = |player: u8| CardStat {
            id: "STRIKE".to_owned(),
            kind: SourceKind::Card,
            player,
            plays: 1,
            damage_dealt: 5,
            ..CardStat::default()
        };

        let mut rows: Vec<CardStat> = Vec::new();
        upsert_card_stat(&mut rows, &row(0), CardStatKey::PerSource);
        upsert_card_stat(&mut rows, &row(1), CardStatKey::PerSource);
        assert_eq!(rows.len(), 2, "PerSource keeps each player's row");
        assert_eq!((rows[0].plays, rows[1].plays), (1, 1));

        let mut team: Vec<CardStat> = Vec::new();
        upsert_card_stat(&mut team, &row(0), CardStatKey::TeamMerged);
        upsert_card_stat(&mut team, &row(1), CardStatKey::TeamMerged);
        assert_eq!(
            team.len(),
            1,
            "TeamMerged folds same-id rows across players"
        );
        assert_eq!(team[0].player, 0, "the row keeps the first-seen player");
        assert_eq!((team[0].plays, team[0].damage_dealt), (2, 10));
    }

    #[test]
    fn upsert_drops_new_rows_at_the_run_cap_but_still_merges() {
        let row = |id: &str| CardStat {
            id: id.to_owned(),
            kind: SourceKind::Card,
            player: 0,
            ..CardStat::default()
        };
        let mut rows: Vec<CardStat> = Vec::new();
        for i in 0..caps::RUN_CARDS {
            upsert_card_stat(&mut rows, &row(&format!("C{i}")), CardStatKey::PerSource);
        }
        assert_eq!(rows.len(), caps::RUN_CARDS);
        upsert_card_stat(&mut rows, &row("EXTRA"), CardStatKey::PerSource);
        assert_eq!(rows.len(), caps::RUN_CARDS, "the cap must not grow");
        assert!(!rows.iter().any(|r| r.id == "EXTRA"));
        // A full table still folds stats into existing rows.
        upsert_card_stat(
            &mut rows,
            &CardStat {
                id: "C0".to_owned(),
                kind: SourceKind::Card,
                player: 0,
                plays: 7,
                ..CardStat::default()
            },
            CardStatKey::PerSource,
        );
        assert_eq!(rows[0].plays, 7);
    }
}
