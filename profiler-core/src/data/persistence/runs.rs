//! The run accumulator behind the Run Summary tab: the live per-combat
//! merge, the save+quit+resume rebuild, and the shared per-field fold.

use super::combat_doc::card_stat_from_rec;
use super::combats::{load_run_combat_docs, parse_combat_docs};
use crate::data::state::{self, CardStat, Combat, STATE};
use crate::fail;

/// The Run Summary tab survives a save+quit+resume.
pub fn rebuild_run_accumulator(seq: u32) -> (u32, u32) {
    let Some(active) = STATE.with(|s| {
        s.borrow()
            .run_ctx
            .as_ref()
            .map(|context| context.run.clone())
    }) else {
        return (0, 0);
    };
    if active.seq != seq {
        return (0, 0);
    }
    let combats = parse_combat_docs(load_run_combat_docs(seq));
    let mut cards = STATE.with(|s| s.borrow().run_cards.clone());
    let mut turns = 0u32;
    let mut count = 0u32;
    for combat in combats {
        if combat.run.as_ref().is_none_or(|run| {
            run.seq != seq || !run.matches_identity(&active.seed, active.started_at, active.profile)
        }) {
            fail!(
                "combat {} has a different identity from run {seq}",
                combat.combat_id
            );
            continue;
        }
        if CardStat::merge_rows(
            &mut cards,
            combat.cards.iter().map(card_stat_from_rec),
            CardStatKey::PerSource,
        ) {
            count += 1;
            turns += combat.turns;
        }
    }
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.run_turns += turns;
        state.run_combats += count;
        state.run_cards = cards;
    });
    (count, turns)
}

/// Merges a finished combat's per-source stats into the run accumulator.
/// Only combats of the currently-active run merge.
pub fn merge_into_run(c: &Combat) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(run) = &c.run else { return };
        if !state.run_ctx.as_ref().is_some_and(|context| {
            context.run.seq != 0
                && context.run.seq == run.seq
                && context.run.seed == run.seed
                && context.run.profile == run.profile
                && context.run.started_at == run.started_at
        }) {
            return;
        }
        if CardStat::merge_rows(&mut state.run_cards, &c.cards, CardStatKey::PerSource) {
            state.run_turns += c.turns;
            state.run_combats += 1;
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

thread_local! {
    static ROLLUP_FAILURE_LOGGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn upsert_card_stat(cards: &mut Vec<CardStat>, card: &CardStat, key: CardStatKey) -> bool {
    let matches = |row: &CardStat| {
        row.id == card.id
            && row.kind == card.kind
            && (row.player == card.player
                || matches!(key, CardStatKey::TeamMerged)
                    && card.kind != state::SourceKind::Unknown)
    };
    if let Some(index) = cards.iter().position(matches) {
        if cards[index].merge_checked(card).is_some() {
            return true;
        }
    } else if (card.kind == state::SourceKind::Unknown
        || cards
            .iter()
            .filter(|row| row.kind != state::SourceKind::Unknown)
            .count()
            < state::caps::RUN_CARDS - state::caps::UNKNOWN_ROWS)
        && cards.len() < state::caps::RUN_CARDS
    {
        cards.push(card.clone());
        return true;
    }
    crate::fail_once(
        &ROLLUP_FAILURE_LOGGED,
        format_args!("run row capacity/arithmetic loss; using credited-slot Unknown"),
    );
    let slot = state::clamp_source_slot(i32::from(card.player));
    if let Some(index) = cards
        .iter()
        .position(|row| row.player == slot && row.kind == state::SourceKind::Unknown)
    {
        if cards[index].merge_checked(card).is_some() {
            return true;
        }
    } else if cards.len() < state::caps::RUN_CARDS {
        let mut unknown = card.clone();
        unknown.id = "UNATTRIBUTED".into();
        unknown.kind = state::SourceKind::Unknown;
        unknown.player = slot;
        cards.push(unknown);
        return true;
    }
    crate::fail!("run row update exceeds capacity or representable arithmetic");
    false
}

impl CardStat {
    // Each combat is one transaction, including capacity fallback. A late
    // rejection must not publish a prefix or increment its run counters.
    pub(crate) fn merge_rows<T: std::borrow::Borrow<Self>>(
        cards: &mut Vec<Self>,
        incoming: impl IntoIterator<Item = T>,
        key: CardStatKey,
    ) -> bool {
        let mut staged = cards.clone();
        for card in incoming {
            if !upsert_card_stat(&mut staged, card.borrow(), key) {
                return false;
            }
        }
        if !Self::arithmetic_representable(&staged) {
            crate::fail!("run row update exceeds representable ledger totals");
            return false;
        }
        *cards = staged;
        true
    }

    fn merge_checked(&mut self, source: &CardStat) -> Option<()> {
        let plays = self.plays.checked_add(source.plays)?;
        let damage_dealt = self.damage_dealt.checked_add(source.damage_dealt)?;
        let damage_blocked = self.damage_blocked.checked_add(source.damage_blocked)?;
        let block_gained = self.block_gained.checked_add(source.block_gained)?;
        let block_effective = self.block_effective.checked_add(source.block_effective)?;
        let forge = self.forge.checked_add(source.forge)?;
        let dmg_direct = self.dmg_direct.checked_add(source.dmg_direct)?;
        let dmg_attributed = self.dmg_attributed.checked_add(source.dmg_attributed)?;
        let dmg_modifier = self.dmg_modifier.checked_add(source.dmg_modifier)?;
        let blk_modifier = self.blk_modifier.checked_add(source.blk_modifier)?;
        let mitigate_debuff = self.mitigate_debuff.checked_add(source.mitigate_debuff)?;
        let mitigate_buff = self.mitigate_buff.checked_add(source.mitigate_buff)?;
        let mitigate_str = self.mitigate_str.checked_add(source.mitigate_str)?;
        let self_damage = self.self_damage.checked_add(source.self_damage)?;

        self.plays = plays;
        self.damage_dealt = damage_dealt;
        self.damage_blocked = damage_blocked;
        self.block_gained = block_gained;
        self.block_effective = block_effective;
        self.forge = forge;
        self.dmg_direct = dmg_direct;
        self.dmg_attributed = dmg_attributed;
        self.dmg_modifier = dmg_modifier;
        self.blk_modifier = blk_modifier;
        self.mitigate_debuff = mitigate_debuff;
        self.mitigate_buff = mitigate_buff;
        self.mitigate_str = mitigate_str;
        self.self_damage = self_damage;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::build_combat_json;
    use crate::data::persistence::test_support::*;
    use crate::data::records;
    use crate::data::state::{RunContext, RunPlayer, caps};
    use crate::source_kind::SourceKind;

    fn set_active_run(seq: u32) {
        STATE.with(|s| {
            s.borrow_mut().run_ctx = Some(RunContext {
                run: synthetic_run(seq),
                ..RunContext::default()
            });
        });
    }

    #[test]
    fn two_players_same_id_cards_round_trip_and_stay_separate() {
        let mut c = synthetic_combat();
        c.players = Box::new([
            RunPlayer {
                slot: 0,
                net_id: "1".into(),
                character: "IRONCLAD".into(),
            },
            RunPlayer {
                slot: 1,
                net_id: "2".into(),
                character: "SILENT".into(),
            },
        ]);
        c.cards = vec![
            CardStat {
                id: "STRIKE".into(),
                kind: SourceKind::Card,
                player: 0,
                plays: 2,
                damage_dealt: 20,
                ..CardStat::default()
            },
            CardStat {
                id: "STRIKE".into(),
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
        assert_eq!((rows[0].id.as_ref(), rows[0].player), ("STRIKE", 0));
        assert_eq!((rows[1].id.as_ref(), rows[1].player), ("STRIKE", 1));
        assert_eq!((rows[0].plays, rows[1].plays), (2, 1));

        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
        });
        set_active_run(42);
        merge_into_run(&c);
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_cards.len(), 2);
            assert!(st.run_cards.iter().any(|r| r.player == 0 && r.plays == 2));
            assert!(st.run_cards.iter().any(|r| r.player == 1 && r.plays == 1));
        });
    }

    #[test]
    fn missing_run_identity_parses_but_cannot_rebuild() {
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
        set_active_run(7);
        let (combats, turns) = rebuild_run_accumulator(7);
        assert_eq!((combats, turns), (0, 0));
        STATE.with(|s| {
            let st = s.borrow();
            assert!(st.run_cards.is_empty());
        });
    }

    #[test]
    fn merge_into_run_accumulates_for_active_run() {
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_turns = 1;
            st.run_combats = 1;
        });
        set_active_run(42);
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
            assert_eq!(omni.id.as_ref(), "OMNI_CARD");
            assert_eq!(omni.kind, SourceKind::Card);
            assert_eq!(omni.plays, 8); // 4 + 4 across two combats
            assert_eq!(omni.damage_dealt, 42); // 21 + 21
            let anchor = &st.run_cards[1];
            assert_eq!(anchor.id.as_ref(), "ANCHOR");
            assert_eq!(anchor.kind, SourceKind::Relic);
            assert_eq!(anchor.block_gained, 20); // 10 + 10
        });
    }

    #[test]
    fn merge_into_run_ignores_inactive_or_foreign_combats() {
        set_active_run(42);
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
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S","profile":2,"started_at":1000},
                "cards":[
                  {"id":"STRIKE","kind":0,"plays":2,"damage_dealt":12,"block_gained":0,"block_effective":0,"heal":0},
                  {"id":"DEFEND","kind":0,"plays":1,"damage_dealt":0,"block_gained":5,"block_effective":5,"heal":0}
                ]},
               {"combat_id":2,"encounter_id":"B","result":"completed","turns":3,"damage_received":8,
                "block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S","profile":2,"started_at":1000},
                "cards":[
                  {"id":"STRIKE","kind":0,"plays":1,"damage_dealt":7,"block_gained":0,"block_effective":0,"heal":0}
                ]},
               {"combat_id":3,"encounter_id":"C","result":"completed","turns":1,"damage_received":1,
                "block_total":0,
                "run":{"seq":6,"character":"IRONCLAD","ascension":0,"game_mode":"Standard","seed":"T"},
                "cards":[{"id":"BASH","kind":0,"plays":9,"damage_dealt":99,"block_gained":0,"block_effective":0,"heal":0}]}
              ]"#,
        );
        for (id, profile, start, seed) in [(4, 3, 1000, "S"), (5, 2, 1001, "S"), (6, 2, 1000, "T")]
        {
            let mut foreign = synthetic_combat();
            foreign.seq = id;
            let mut run = synthetic_run(5);
            run.profile = profile;
            run.started_at = start;
            run.seed = seed.into();
            foreign.run = Some(run);
            write_store_file(&data, 5, id, &build_combat_json(&foreign));
        }
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        set_active_run(5);
        let (combats, turns) = rebuild_run_accumulator(5);
        assert_eq!((combats, turns), (2, 7));
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.run_combats, 2);
            assert_eq!(st.run_turns, 7);
            let strike = st
                .run_cards
                .iter()
                .find(|c| c.id.as_ref() == "STRIKE")
                .expect("STRIKE merged");
            assert_eq!(strike.plays, 3);
            assert_eq!(strike.damage_dealt, 19);
            let defend = st
                .run_cards
                .iter()
                .find(|c| c.id.as_ref() == "DEFEND")
                .expect("DEFEND merged");
            assert_eq!(defend.block_gained, 5);
            assert!(!st.run_cards.iter().any(|c| c.id.as_ref() == "BASH"));
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
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S","profile":2,"started_at":1000},
                "cards":[{"id":"STRIKE","kind":0,"plays":2,"damage_dealt":12,
                          "block_gained":0,"block_effective":0,"heal":0}]}"#,
        );
        write_store_file(
            &data,
            6,
            4,
            r#"{"combat_id":4,"encounter_id":"D","result":"completed","turns":1,
                "damage_received":1,"block_total":0,
                "run":{"seq":5,"character":"DEFECT","ascension":1,"game_mode":"Standard","seed":"S","profile":2,"started_at":1000},
                "cards":[{"id":"SNEAKY","kind":0,"plays":100,"damage_dealt":1,
                          "block_gained":0,"block_effective":0,"heal":0}]}"#,
        );
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards.clear();
            st.run_turns = 0;
            st.run_combats = 0;
        });
        set_active_run(5);
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
                !st.run_cards.iter().any(|c| c.id.as_ref() == "SNEAKY"),
                "a same-seq combat in a foreign run's directory must not fold"
            );
        });
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
            st.run_turns = 0;
            st.run_combats = 0;
            st.run_cards.clear();
        });
        set_active_run(42);
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
            assert_eq!(st.run_cards, live_cards);
        });
    }

    #[test]
    fn upsert_key_chooses_whether_player_splits_rows() {
        let row = |player: u8| CardStat {
            id: "STRIKE".into(),
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
    fn late_field_overflow_preserves_rows_before_unknown_fallback() {
        let mut rows = vec![
            CardStat {
                id: "STRIKE".into(),
                player: 2,
                plays: 7,
                damage_dealt: 11,
                self_damage: i64::MAX,
                ..CardStat::default()
            },
            CardStat {
                id: "UNATTRIBUTED".into(),
                kind: SourceKind::Unknown,
                player: 2,
                plays: 1,
                damage_dealt: 5,
                ..CardStat::default()
            },
        ];
        let original = rows[0].clone();
        let mut incoming = CardStat {
            id: "STRIKE".into(),
            player: 2,
            plays: 2,
            damage_dealt: 3,
            self_damage: 1,
            ..CardStat::default()
        };
        assert!(upsert_card_stat(
            &mut rows,
            &incoming,
            CardStatKey::PerSource
        ));
        assert_eq!(rows[0], original);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (
                rows[1].player,
                rows[1].plays,
                rows[1].damage_dealt,
                rows[1].self_damage
            ),
            (2, 3, 8, 1)
        );

        let before = rows.clone();
        incoming.self_damage = i64::MAX;
        assert!(!upsert_card_stat(
            &mut rows,
            &incoming,
            CardStatKey::PerSource
        ));
        assert_eq!(rows, before);
    }

    #[test]
    fn defense_boundary_rejects_the_whole_run_update() {
        for (id, kind, player) in [
            ("A", SourceKind::Card, 0),
            ("B", SourceKind::Card, 1),
            ("UNATTRIBUTED", SourceKind::Unknown, state::TEAM_SLOT),
        ] {
            for negative in [0, -2] {
                STATE.with(|s| *s.borrow_mut() = state::State::default());
                set_active_run(42);
                let mut combat = synthetic_combat();
                combat.cards = vec![
                    CardStat {
                        id: "A".into(),
                        mitigate_buff: i64::MAX - 1,
                        ..CardStat::default()
                    },
                    CardStat {
                        id: "NEGATIVE".into(),
                        player: 2,
                        block_effective: negative,
                        ..CardStat::default()
                    },
                ];
                merge_into_run(&combat);
                let before = STATE.with(|s| s.borrow().run_cards.clone());
                combat.cards = vec![
                    CardStat {
                        id: "EARLY".into(),
                        damage_dealt: 3,
                        dmg_direct: 3,
                        ..CardStat::default()
                    },
                    CardStat {
                        id: id.into(),
                        kind,
                        player,
                        mitigate_debuff: 2,
                        ..CardStat::default()
                    },
                ];
                merge_into_run(&combat);
                STATE.with(|s| {
                    let st = s.borrow();
                    assert_eq!(st.run_cards, before);
                    assert_eq!((st.run_combats, st.run_turns), (1, combat.turns));
                });
                combat.cards.remove(0);
                combat.cards[0].mitigate_debuff = 1;
                merge_into_run(&combat);
                STATE.with(|s| {
                    let st = s.borrow();
                    let positive_defense: i128 = st
                        .run_cards
                        .iter()
                        .map(|row| {
                            (i128::from(row.block_effective)
                                + i128::from(row.blk_modifier)
                                + i128::from(row.mitigate_buff)
                                + i128::from(row.mitigate_debuff)
                                + i128::from(row.mitigate_str))
                            .max(0)
                        })
                        .sum();
                    assert_eq!(positive_defense, i128::from(i64::MAX));
                    assert_eq!((st.run_combats, st.run_turns), (2, combat.turns * 2));
                });
            }
        }
    }

    #[test]
    fn defense_overflow_cannot_spill_into_a_reserved_unknown_row() {
        let mut rows: Vec<_> = (0..caps::RUN_CARDS - caps::UNKNOWN_ROWS)
            .map(|index| CardStat {
                id: format!("C{index}").into(),
                ..CardStat::default()
            })
            .collect();
        rows[0].mitigate_buff = i64::MAX - 1;
        let before = rows.clone();
        let incoming = [
            CardStat {
                id: "EARLY".into(),
                player: 3,
                damage_dealt: 2,
                dmg_direct: 2,
                ..CardStat::default()
            },
            CardStat {
                id: "LATE".into(),
                player: 3,
                mitigate_debuff: 2,
                ..CardStat::default()
            },
        ];
        assert!(!CardStat::merge_rows(
            &mut rows,
            incoming.clone(),
            CardStatKey::PerSource
        ));
        assert_eq!(rows, before);
        let mut accepted = incoming;
        accepted[1].mitigate_debuff = 1;
        assert!(CardStat::merge_rows(
            &mut rows,
            accepted,
            CardStatKey::PerSource
        ));
        let unknown = rows
            .last()
            .expect("capacity fallback appends its reserved row");
        assert_eq!(unknown.kind, SourceKind::Unknown);
        assert_eq!(
            (
                unknown.player,
                unknown.damage_dealt,
                unknown.mitigate_debuff
            ),
            (3, 2, 1)
        );
    }

    #[test]
    fn team_merge_rejects_combined_defense_and_late_rows_transactionally() {
        for id in ["A", "B"] {
            let mut rows = vec![CardStat {
                id: "A".into(),
                mitigate_buff: i64::MAX - 1,
                ..CardStat::default()
            }];
            let before = rows.clone();
            let incoming = [
                CardStat {
                    id: "EARLY".into(),
                    damage_dealt: 1,
                    ..CardStat::default()
                },
                CardStat {
                    id: id.into(),
                    player: 1,
                    mitigate_debuff: 2,
                    ..CardStat::default()
                },
            ];
            assert!(!CardStat::merge_rows(
                &mut rows,
                incoming,
                CardStatKey::TeamMerged
            ));
            assert_eq!(rows, before);
        }
    }

    #[test]
    fn rebuild_rejects_whole_overflowing_combats_and_keeps_existing_rows() {
        let dir = unique_dir("rebuild-defense-boundary");
        let data = dir.join("data");
        init_state(&data);
        set_active_run(42);
        STATE.with(|s| {
            let mut st = s.borrow_mut();
            st.run_cards = vec![CardStat {
                id: "A".into(),
                mitigate_buff: i64::MAX - 1,
                ..CardStat::default()
            }];
            st.run_turns = 3;
            st.run_combats = 1;
        });
        let mut combat = synthetic_combat();
        combat.seq = 1;
        combat.cards = vec![
            CardStat {
                id: "EARLY".into(),
                damage_dealt: 1,
                dmg_direct: 1,
                ..CardStat::default()
            },
            CardStat {
                id: "B".into(),
                player: 1,
                mitigate_debuff: 2,
                ..CardStat::default()
            },
        ];
        write_store_file(&data, 42, 1, &build_combat_json(&combat));
        combat.seq = 2;
        combat.cards.remove(0);
        combat.cards[0].mitigate_debuff = 1;
        write_store_file(&data, 42, 2, &build_combat_json(&combat));
        assert_eq!(rebuild_run_accumulator(42), (1, combat.turns));
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!((st.run_combats, st.run_turns), (2, 3 + combat.turns));
            assert_eq!(st.run_cards.len(), 2);
            assert_eq!(st.run_cards[0].mitigate_buff, i64::MAX - 1);
            assert_eq!(
                (st.run_cards[1].id.as_ref(), st.run_cards[1].mitigate_debuff),
                ("B", 1)
            );
        });
    }

    #[test]
    fn run_cap_preserves_each_creditor_in_reserved_unknown_rows() {
        let row = |id: String, player| CardStat {
            id: id.into(),
            kind: SourceKind::Card,
            player,
            plays: 1,
            damage_dealt: 2,
            dmg_direct: 2,
            damage_blocked: 1,
            ..CardStat::default()
        };
        let mut rows = Vec::new();
        for index in 0..caps::RUN_CARDS - caps::UNKNOWN_ROWS {
            upsert_card_stat(
                &mut rows,
                &row(format!("C{index}"), 0),
                CardStatKey::PerSource,
            );
        }
        for player in 0..=state::TEAM_SLOT {
            upsert_card_stat(
                &mut rows,
                &row(format!("EXTRA_{player}"), player),
                CardStatKey::PerSource,
            );
        }
        assert_eq!(rows.len(), caps::RUN_CARDS);
        for player in 0..=state::TEAM_SLOT {
            let unknown = rows
                .iter()
                .find(|row| row.player == player && row.kind == SourceKind::Unknown)
                .expect("each creditor keeps its reserved overflow row");
            assert_eq!(
                (
                    unknown.id.as_ref(),
                    unknown.plays,
                    unknown.damage_dealt,
                    unknown.damage_blocked
                ),
                ("UNATTRIBUTED", 1, 2, 1)
            );
        }
        assert_eq!(
            rows.iter().map(|row| row.damage_dealt).sum::<i64>(),
            2 * caps::RUN_CARDS as i64
        );
        let mut existing = row("C0".to_owned(), 0);
        existing.plays = 7;
        upsert_card_stat(&mut rows, &existing, CardStatKey::PerSource);
        assert_eq!(rows[0].plays, 8);
        assert_eq!(rows.len(), caps::RUN_CARDS);
    }
}
