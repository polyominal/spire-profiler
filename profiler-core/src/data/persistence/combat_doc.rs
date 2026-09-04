//! The combat record's JSON serialization: [`build_combat_json`] and
//! [`card_stat_from_rec`], which converts a parsed card row back to the
//! in-memory `CardStat` the run-history roll-up merges.

use serde::Serialize;

use crate::data::records;
use crate::data::state::{CardStat, Combat, is_zero};
use crate::source_kind::SourceKind;
/// One combat record's card entry. serde serializes fields in declaration
/// order; the snapshot test pins that order byte for byte.
#[derive(Serialize)]
struct CardDoc<'a> {
    id: &'a str,
    kind: SourceKind,
    #[serde(skip_serializing_if = "is_zero")]
    plays: u32,
    #[serde(skip_serializing_if = "is_zero")]
    damage_dealt: i64,
    #[serde(skip_serializing_if = "is_zero")]
    damage_blocked: i64,
    #[serde(skip_serializing_if = "is_zero")]
    block_gained: i64,
    #[serde(skip_serializing_if = "is_zero")]
    block_effective: i64,
    #[serde(skip_serializing_if = "is_zero")]
    forge: i64,
    #[serde(skip_serializing_if = "is_zero")]
    dmg_direct: i64,
    #[serde(skip_serializing_if = "is_zero")]
    dmg_attributed: i64,
    #[serde(skip_serializing_if = "is_zero")]
    dmg_modifier: i64,
    #[serde(skip_serializing_if = "is_zero")]
    blk_modifier: i64,
    #[serde(skip_serializing_if = "is_zero")]
    mitigate_debuff: i64,
    #[serde(skip_serializing_if = "is_zero")]
    mitigate_buff: i64,
    #[serde(skip_serializing_if = "is_zero")]
    mitigate_str: i64,
    #[serde(skip_serializing_if = "is_zero")]
    self_damage: i64,
    /// The owning player's slot (u8, TEAM = 4): two players' same-id cards
    /// stay separate rows in the combat record ([`crate::data::state`]).
    player: u8,
}

#[derive(Serialize)]
struct RunDoc<'a> {
    seq: u32,
    character: &'a str,
    ascension: i32,
    game_mode: &'a str,
    seed: &'a str,
}

/// One combat record's JSON shape; `run` is omitted when `run_seq == 0`.
#[derive(Serialize)]
struct CombatDoc<'a> {
    combat_id: u32,
    started_at: i64,
    encounter_id: &'a str,
    result: &'a str,
    #[serde(skip_serializing_if = "is_zero")]
    turns: u32,
    #[serde(skip_serializing_if = "is_zero")]
    damage_received: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<RunDoc<'a>>,
    cards: Vec<CardDoc<'a>>,
}

/// The per-card row serialization, split out so the field list stays out
/// of `build_combat_json`.
fn card_doc(card: &crate::data::state::CardStat) -> CardDoc<'_> {
    CardDoc {
        id: &card.id,
        kind: card.kind,
        plays: card.plays,
        damage_dealt: card.damage_dealt,
        damage_blocked: card.damage_blocked,
        block_gained: card.block_gained,
        block_effective: card.block_effective,
        forge: card.forge,
        dmg_direct: card.dmg_direct,
        dmg_attributed: card.dmg_attributed,
        dmg_modifier: card.dmg_modifier,
        blk_modifier: card.blk_modifier,
        mitigate_debuff: card.mitigate_debuff,
        mitigate_buff: card.mitigate_buff,
        mitigate_str: card.mitigate_str,
        self_damage: card.self_damage,
        player: card.player,
    }
}

/// Serializes one combat record.
pub fn build_combat_json(c: &Combat) -> String {
    let run = c.run.as_ref().map(|run| RunDoc {
        seq: run.seq,
        character: &run.character,
        ascension: run.ascension,
        game_mode: &run.game_mode,
        seed: &run.seed,
    });
    let cards = c.cards.iter().map(card_doc).collect();
    let doc = CombatDoc {
        combat_id: c.seq,
        started_at: c.started_at,
        encounter_id: &c.encounter_id,
        result: &c.result,
        turns: c.turns,
        damage_received: c.damage_received,
        run,
        cards,
    };
    serde_json::to_string(&doc).expect("combat document cannot fail to serialize")
}

/// Converts a parsed card row to the in-memory `CardStat`.
pub(crate) fn card_stat_from_rec(rec: &records::CardRec) -> CardStat {
    CardStat {
        id: rec.id.clone(),
        kind: rec.kind,
        player: rec.player,
        plays: rec.plays,
        damage_dealt: rec.damage_dealt,
        damage_blocked: rec.damage_blocked,
        block_gained: rec.block_gained,
        block_effective: rec.block_effective,
        forge: rec.forge,
        dmg_direct: rec.dmg_direct,
        dmg_attributed: rec.dmg_attributed,
        dmg_modifier: rec.dmg_modifier,
        blk_modifier: rec.blk_modifier,
        mitigate_debuff: rec.mitigate_debuff,
        mitigate_buff: rec.mitigate_buff,
        mitigate_str: rec.mitigate_str,
        self_damage: rec.self_damage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::persistence::test_support::*;

    #[test]
    fn build_combat_json_matches_the_documented_schema() {
        insta::assert_snapshot!(build_combat_json(&synthetic_combat()));
    }

    #[test]
    fn build_combat_json_omits_run_when_absent() {
        let mut c = synthetic_combat();
        c.run = None;
        c.players.clear();
        let json = build_combat_json(&c);
        assert!(!json.contains(r#""run":"#));
        assert!(!json.contains(r#""origin""#));
        assert!(!json.contains(r#""players""#));
        assert!(!json.contains(r#""profile""#));
        assert!(!json.contains(r#""build""#));
    }

    /// An all-zero card row serializes to its identity fields only and
    /// parses back to the same row.
    #[test]
    fn all_zero_card_rows_round_trip_as_identity_only() {
        let mut c = synthetic_combat();
        c.cards = vec![CardStat {
            id: "ZERO_ROW".to_owned(),
            kind: SourceKind::Potion,
            player: 2,
            ..CardStat::default()
        }];
        let json = build_combat_json(&c);
        assert!(json.contains(r#""id":"ZERO_ROW","kind":3,"player":2}"#));
        let doc: serde_json::Value = serde_json::from_str(&json).expect("combat JSON parses");
        let row = doc["cards"][0]
            .as_object()
            .expect("the zero row is a JSON object");
        for name in row.keys() {
            assert!(
                matches!(name.as_str(), "id" | "kind" | "player"),
                "the all-zero row must carry identity fields only, got '{name}'"
            );
        }
        let combat = records::parse_combat_doc(&json).expect("record parses");
        assert_eq!(
            combat.cards[0],
            records::CardRec {
                id: "ZERO_ROW".to_owned(),
                kind: SourceKind::Potion,
                player: 2,
                ..records::CardRec::default()
            }
        );
    }

    #[test]
    fn combat_file_round_trips_through_parse_combat_doc() {
        let dir = unique_dir("roundtrip");
        let data = dir.join("data");
        init_state(&data);
        let c1 = synthetic_combat(); // run_seq 42
        let entry1 = build_combat_json(&c1);
        write_store_file(&data, 42, 7, &entry1);
        let mut c2 = synthetic_combat();
        c2.seq = 8;
        c2.encounter_id = "FROZEN_COUNCIL".to_owned();
        let entry2 = build_combat_json(&c2);
        write_store_file(&data, 42, 8, &entry2);

        let combats: Vec<records::CombatRec> = super::super::combats::load_all_combat_docs()
            .iter()
            .map(|doc| records::parse_combat_doc(doc).expect("store doc parses"))
            .collect();
        assert_eq!(combats.len(), 2);

        let c = &combats[0];
        assert_eq!(c.combat_id, 7);
        assert_eq!(c.encounter_id, "BYGONE_EFFIGY");
        assert_eq!(c.result, "completed");
        assert_eq!(c.turns, 5);
        assert_eq!(c.damage_received, 33);
        let run = c.run.as_ref().expect("run present");
        assert_eq!(run.seq, 42);
        assert_eq!(run.character, "SHROUD");
        assert_eq!(run.ascension, 5);
        assert_eq!(run.game_mode, "standard");
        assert_eq!(c.cards.len(), 2);
        assert_eq!(c.cards[0].id, "OMNI_CARD");
        assert_eq!(c.cards[0].player, 0, "single-player rows read as slot 0");
        assert_eq!(c.cards[0].plays, 4);
        assert_eq!(c.cards[0].damage_dealt, 21);
        assert_eq!(c.cards[1].id, "ANCHOR");
        assert_eq!(c.cards[1].block_gained, 10);
        assert_eq!(combats[1].combat_id, 8);
        assert_eq!(combats[1].encounter_id, "FROZEN_COUNCIL");
    }
}
