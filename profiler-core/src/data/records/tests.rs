//! The record module's unit tests: each test exercises a record's JSON
//! contract: parse tolerance and the run record's byte-for-byte schema.

use super::*;
use crate::data::state::{RunContext, RunOutcome, RunPlayer};
use crate::source_kind::SourceKind;

fn run_context() -> RunContext {
    RunContext {
        active: true,
        seq: 9,
        character: "IRONCLAD".to_owned(),
        ascension: 3,
        game_mode: "standard".to_owned(),
        seed: "SEED123".to_owned(),
        started_at: 1_786_579_200, // 2026-08-13T00:00:00Z in epoch seconds
        ended_at: 1_786_597_200,   // 2026-08-13T00:30:00Z
        outcome: RunOutcome::Victory,
        players: vec![
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
        ],
    }
}

#[test]
fn parse_combat_doc_ignores_unknown_fields_and_fills_defaults() {
    // The pre-reduction schema's dropped fields are the fixture's unknown
    // fields: an older build's record still parses, everything purged
    // reading as absent.
    let doc = r#"{
            "combat_id":1,"encounter_id":"A","encounter_type":"Elite","result":"completed",
             "plays":2,"turns":1,"draws":3,"damage_received":4,"damage_received_unblocked":2,
             "block_total":6,"energy_spent":7,"stars_spent":8,"stars_gained":9,"heal_received":10,
             "potions_used":11,"ended_at":1786622496,
             "run":{"seq":5,"character":"X","ascension":12,"game_mode":"gm"},
             "players":[{"slot":0,"net_id":"1","character":"IRONCLAD"}],
             "profile":3,"build":"deadbeef","started_at":1786622400,
             "cards":[
                {"id":"C1","plays":1,"damage_dealt":5,"kind":2,"forge":1,
                 "energy_gained":3,"stars_gained":4,"self_damage":2,
                 "dmg_direct":5,"dmg_attributed":1,"blk_modifier":6,"mitigate_str":9,
                 "player":3},
                {"id":"C2"}
             ]
        }"#;
    let c = parse_combat_doc(doc).expect("parses");

    assert_eq!(c.combat_id, 1);
    assert_eq!(c.encounter_id, "A");
    assert_eq!(c.result, "completed");
    assert_eq!(c.turns, 1);
    assert_eq!(c.damage_received, 4);
    assert_eq!(c.started_at, 1_786_622_400);
    let run = c.run.as_ref().expect("run decoded");
    assert_eq!(run.seq, 5);
    assert_eq!(run.character, "X");
    assert_eq!(run.ascension, 12);
    assert_eq!(run.game_mode, "gm");
    assert_eq!(c.cards.len(), 2);
    assert_eq!(c.cards[0].id, "C1");
    assert_eq!(c.cards[0].kind, SourceKind::Power);
    assert_eq!(c.cards[0].plays, 1);
    assert_eq!(c.cards[0].damage_dealt, 5);
    assert_eq!(c.cards[0].forge, 1);
    assert_eq!(c.cards[0].self_damage, 2);
    assert_eq!(c.cards[0].dmg_direct, 5);
    assert_eq!(c.cards[0].dmg_attributed, 1);
    assert_eq!(c.cards[0].blk_modifier, 6);
    assert_eq!(c.cards[0].mitigate_str, 9);
    assert_eq!(c.cards[0].player, 3);
    // Missing fields fall back to defaults; unknown fields are skipped.
    // A card row without `player` reads as slot 0 (single-player), the
    // additive-schema rule.
    assert_eq!(
        c.cards[1],
        CardRec {
            id: "C2".to_owned(),
            ..CardRec::default()
        }
    );
    assert_eq!(c.cards[1].player, 0);
}

#[test]
fn parse_combat_doc_decodes_minimal_records() {
    let c = parse_combat_doc(r#"{"combat_id":2,"encounter_id":"B"}"#).expect("parses");
    assert_eq!(c.combat_id, 2);
    assert_eq!(c.encounter_id, "B");
    assert!(c.run.is_none());
    assert!(c.cards.is_empty());
}

#[test]
fn parse_combat_doc_run_defaults_ascension_to_minus_one() {
    let c = parse_combat_doc(r#"{"run":{"seq":3}}"#).expect("parses");
    let run = c.run.as_ref().expect("run present");
    assert_eq!(run.seq, 3);
    assert_eq!(run.ascension, -1);
    assert_eq!(run.character, "");
    assert_eq!(run.game_mode, "");
}

#[test]
fn parse_combat_doc_rejects_wrong_types_and_malformed_json() {
    assert!(parse_combat_doc(r#"{"combat_id":-1}"#).is_err()); // negative → u32
    assert!(parse_combat_doc(r#"{"cards":[{"plays":"x"}]}"#).is_err()); // string → u32
    assert!(parse_combat_doc(r#"{"combat_id":1"#).is_err()); // syntax error
}

// The runs.jsonl entry layout is pinned byte-for-byte: identity, header
// facts, outcome, timestamps, roster; no build tag, no combat roll-ups.
#[test]
fn build_run_json_matches_the_documented_schema() {
    insta::assert_snapshot!(build_run_json(&run_context(), 2));
}

#[test]
fn build_run_json_round_trips_through_the_parser() {
    let run = run_context();
    let json = build_run_json(&run, 7);
    let parsed: RunDocOwned = serde_json::from_str(&json).expect("parses back");
    assert_eq!(parsed.run_id, run.seq);
    assert_eq!(parsed.profile, 7);
    assert_eq!(parsed.character, run.character);
    assert_eq!(parsed.ascension, run.ascension);
    assert_eq!(parsed.game_mode, run.game_mode);
    assert_eq!(parsed.outcome, RunOutcome::Victory);
    assert_eq!(parsed.seed, run.seed);
    assert_eq!(parsed.started_at, run.started_at);
    assert_eq!(parsed.ended_at, run.ended_at);
    assert_eq!(parsed.players.len(), 2);
    assert_eq!(parsed.players[1].slot, 1);
    assert_eq!(parsed.players[1].character, "SILENT");
}

#[test]
fn build_run_json_omits_an_empty_roster() {
    let run = RunContext {
        players: Vec::new(),
        ..run_context()
    };
    let json = build_run_json(&run, 7);
    assert!(!json.contains(r#""players""#));
    let parsed: RunDocOwned = serde_json::from_str(&json).expect("parses back");
    assert!(parsed.players.is_empty());
}
