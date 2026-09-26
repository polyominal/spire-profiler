use serde_json::json;

use super::*;

struct Fixture {
    events: Vec<Value>,
    grants: Vec<(u64, &'static str)>,
    credits: BTreeMap<&'static str, u64>,
    wrong_supplier: bool,
}

impl Fixture {
    fn new(wrong_supplier: bool) -> Self {
        let mut fixture = Self {
            events: Vec::new(),
            grants: Vec::new(),
            credits: BTreeMap::new(),
            wrong_supplier,
        };
        fixture.push(
            "combat_start",
            json!({"audit_version":2,"policy_version":3,"epoch":7}),
        );
        fixture
    }

    fn push(&mut self, event: &str, data: Value) -> u64 {
        let seq = self.events.len() as u64 + 1;
        self.events
            .push(json!({"seq":seq,"turn":1,"event":event,"data":data}));
        seq
    }

    fn creature(player: bool) -> Value {
        json!({"instance": if player {100} else {200}, "native_instance":if player {10} else {20},"player":if player {0}else{4}})
    }

    fn card(model: &str) -> Value {
        json!({"identity":if model=="ENVENOM" {1}else{2},"role":"card","model":model,"player":0,"origin":"ordinary"})
    }

    fn native(&self) -> Value {
        let claim = |id: &str| {
            if self.wrong_supplier {
                "IMAGINARY_SUPPLIER".to_owned()
            } else {
                id.to_owned()
            }
        };
        let mut source: BTreeMap<String, u64> = BTreeMap::new();
        let grants: Vec<_> = self.grants.iter().map(|(remaining, id)| {
            *source.entry(claim(id)).or_default() += remaining;
            json!({"remaining":remaining,"source":[{"id":claim(id),"kind":0,"player":0,"weight":1}]})
        }).collect();
        let amount: u64 = self.grants.iter().map(|(amount, _)| amount).sum();
        let mixture: Vec<_> = source
            .into_iter()
            .map(|(id, weight)| json!({"id":id,"kind":0,"player":0,"weight":weight}))
            .collect();
        let mut counters: BTreeMap<String, u64> = BTreeMap::new();
        for (id, amount) in &self.credits {
            *counters.entry(claim(id)).or_default() += amount;
        }
        let cards: Vec<_> = counters
            .into_iter()
            .map(|(id, damage)| json!({"id":id,"kind":0,"player":0,"dmg_attributed":damage}))
            .collect();
        let poison = if amount == 0 {
            Vec::new()
        } else {
            vec![
                json!({"instance":30,"owner":20,"amount":amount,"trusted":true,"grants":grants,"source":mixture}),
            ]
        };
        json!({"audit_version":1,"policy_version":3,"combat_id":7,"poison":poison,"cards":cards,"coverage":{"complete":true,"failures":0,"reasons":[]}})
    }

    fn card_frame(&mut self, card: &str) -> u64 {
        self.push("source_frame", json!({"source":Self::card(card),"power_identity":0,"target":Self::creature(true),"amount":0,"cause":0,"context":"command"}))
    }

    fn power_frame(&mut self, poison: bool, amount: u64) -> u64 {
        let id = if poison { 300 } else { 400 };
        self.push("source_frame", json!({"source":{"identity":id,"role":"power","model":if poison {"POISON_POWER"}else{"ENVENOM_POWER"},"origin":"observed","player":if poison {4}else{0}},"power_identity":id,"target":Self::creature(!poison),"amount":amount,"cause":0,"context":if poison {"damage"}else{"producer"}}))
    }

    fn grant(&mut self, card: &'static str, amount: u64) {
        let frame = self.card_frame(card);
        self.apply(true, amount, frame, Self::card(card), 0, card);
    }

    fn apply(
        &mut self,
        poison: bool,
        amount: u64,
        frame: u64,
        source: Value,
        cause: u64,
        card: &'static str,
    ) {
        let before: u64 = if poison {
            self.grants.iter().map(|(amount, _)| amount).sum()
        } else {
            0
        };
        let identity = if poison { 300 } else { 400 };
        let power = if poison {
            "POISON_POWER"
        } else {
            "ENVENOM_POWER"
        };
        let action = self.push("power_attempt", json!({"command":"Apply","power":power,"power_identity":identity,"target":Self::creature(!poison),"source":source,"source_frame":frame,"cause":cause,"amount":amount.to_string(),"parent_action":0}));
        if poison {
            self.grants.push((amount, card));
        }
        self.push("power_change", json!({"action":action,"source_frame":frame,"power":power,"power_identity":identity,"power_instance":if poison {30}else{40},"target":Self::creature(!poison),"before":before,"after":before+amount,"before_attached":before>0,"after_attached":true,"native":self.native()}));
        self.push("command_end", json!({"action":action,"command":"Apply","outcome":"completed","requested_power_identity":identity,"power_identity":identity,"target":Self::creature(!poison),"amount":before+amount,"attached":true}));
    }

    fn envenom(&mut self, amount: u64) {
        let frame = self.card_frame("ENVENOM");
        self.apply(false, amount, frame, Self::card("ENVENOM"), 0, "ENVENOM");
        let begin = self.push(
            "damage_begin",
            json!({"dealer":Self::creature(true),"target":Self::creature(false),"props":8}),
        );
        self.push("damage_result", json!({"tick_seq":begin,"result_identity":500,"target":Self::creature(false),"unblocked":1,"blocked":0}));
        let frame = self.power_frame(false, amount);
        let cause = self.push("envenom_trigger", json!({"power_identity":400,"source_frame":frame,"owner":Self::creature(true),"dealer":Self::creature(true),"target":Self::creature(false),"result_identity":500,"unblocked":1,"props":8,"amount":amount,"eligible":true}));
        self.apply(true, amount, frame, json!({"role":"power","identity":400,"model":"ENVENOM_POWER","player":0,"origin":"observed"}), cause, "ENVENOM");
        self.push(
            "envenom_end",
            json!({"trigger":cause,"outcome":"completed"}),
        );
    }

    fn tick(&mut self, expected: &[(&'static str, u64)], frame: Option<u64>) {
        let amount: u64 = self.grants.iter().map(|(amount, _)| amount).sum();
        let frame = frame.unwrap_or_else(|| self.power_frame(true, amount));
        let tick = self.push("poison_tick", json!({"power_identity":300,"power_instance":30,"source_frame":frame,"target":Self::creature(false),"poison_before":amount,"requested":amount.to_string(),"hp_before":1000,"native":self.native()}));
        for (id, amount) in expected {
            *self.credits.entry(id).or_default() += amount;
        }
        self.push("poison_damage", json!({"tick_seq":tick,"target":Self::creature(false),"hp_after":1000-expected.iter().map(|(_,amount)|amount).sum::<u64>(),"unblocked":expected.iter().map(|(_,amount)|amount).sum::<u64>(),"blocked":0,"overkill":0,"group_count":1,"group_index":0,"receiver_side":"enemy","receiver_kind":"monster","native":self.native()}));
    }

    fn decay(&mut self) {
        let before: u64 = self.grants.iter().map(|(amount, _)| amount).sum();
        self.grants[0].0 -= 1;
        self.grants.retain(|(amount, _)| *amount != 0);
        self.push("power_change", json!({"action":0,"source_frame":0,"power":"POISON_POWER","power_identity":300,"power_instance":30,"target":Self::creature(false),"before":before,"after":before-1,"before_attached":true,"after_attached":before>1,"native":self.native()}));
    }

    fn finish(&mut self) -> Reconstruction {
        self.push(
            "combat_end",
            json!({"complete":true,"native":self.native()}),
        );
        let mut input = self
            .events
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        input.push('\n');
        Reconstruction::run(&Trace::read(input.as_bytes()).expect("synthetic trace framing"))
    }
}

#[test]
fn raw_sources_expose_internally_consistent_wrong_native_supplier() {
    for wrong in [false, true] {
        let mut fixture = Fixture::new(wrong);
        fixture.grant("SNAKEBITE", 7);
        fixture.tick(&[("SNAKEBITE", 7)], None);
        let result = fixture.finish();
        let discrepancies: Vec<_> = result
            .checks
            .values()
            .flatten()
            .filter(|check| matches!(check, Check::Discrepancy(_)))
            .collect();
        assert_eq!(discrepancies.is_empty(), !wrong);
        assert_eq!(
            result.credits[&Source {
                id: "SNAKEBITE".into(),
                kind: 0,
                player: 0
            }],
            7
        );
        assert!(result.complete);
    }
}

#[test]
fn envenom_causal_chain_and_fifo_independently_produce_six_and_nine() {
    let mut fixture = Fixture::new(false);
    fixture.envenom(3);
    fixture.grant("SNAKEBITE", 2);
    for expected in [
        vec![("ENVENOM", 3), ("SNAKEBITE", 2)],
        vec![("ENVENOM", 2), ("SNAKEBITE", 2)],
        vec![("ENVENOM", 1), ("SNAKEBITE", 2)],
        vec![("SNAKEBITE", 2)],
        vec![("SNAKEBITE", 1)],
    ] {
        fixture.tick(&expected, None);
        fixture.decay();
    }
    let result = fixture.finish();
    assert!(result.complete);
    assert!(
        result
            .checks
            .values()
            .flatten()
            .all(|check| matches!(check, Check::Match(_))),
        "{:?}",
        result
            .checks
            .values()
            .flatten()
            .map(Check::text)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        result.credits[&Source {
            id: "ENVENOM".into(),
            kind: 0,
            player: 0
        }],
        6
    );
    assert_eq!(
        result.credits[&Source {
            id: "SNAKEBITE".into(),
            kind: 0,
            player: 0
        }],
        9
    );
}

#[test]
fn damage_uses_frozen_raw_source_before_later_mutations() {
    let mut fixture = Fixture::new(false);
    fixture.grant("ENVENOM", 2);
    let frozen = fixture.power_frame(true, 2);
    fixture.grant("SNAKEBITE", 3);
    fixture.tick(&[("ENVENOM", 2)], Some(frozen));
    let result = fixture.finish();
    assert_eq!(
        result.credits[&Source {
            id: "ENVENOM".into(),
            kind: 0,
            player: 0
        }],
        2
    );
    assert!(
        result
            .checks
            .values()
            .flatten()
            .all(|check| matches!(check, Check::Match(_)))
    );
}

#[test]
fn unsupported_origin_missing_outcome_and_raw_gaps_never_seed_native_suppliers() {
    for failure in 0..3 {
        let mut fixture = Fixture::new(false);
        fixture.grant("SNAKEBITE", 7);
        match failure {
            0 => fixture.events[1]["data"]["source"]["origin"] = json!("unknown"),
            1 => fixture.events[4]["event"] = json!("missing_command_end"),
            _ => fixture.events[3]["seq"] = json!(900),
        }
        fixture.tick(&[("SNAKEBITE", 7)], None);
        let result = fixture.finish();
        assert!(!result.complete);
        assert!(result.credits.is_empty());
        assert!(
            result
                .checks
                .values()
                .flatten()
                .any(|check| matches!(check, Check::Unverified(_)))
        );
    }
}

#[test]
fn envenom_wrong_result_or_dealer_taints_dependent_poison() {
    for field in ["result_identity", "dealer"] {
        let mut fixture = Fixture::new(false);
        fixture.envenom(3);
        let trigger = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == "envenom_trigger")
            .expect("trigger fixture");
        trigger["data"][field] = if field == "dealer" {
            Fixture::creature(false)
        } else {
            json!(9999)
        };
        fixture.tick(&[("ENVENOM", 3)], None);
        let result = fixture.finish();
        assert!(!result.complete);
        assert!(result.credits.is_empty());
    }
}

#[test]
fn completed_envenom_without_canonical_child_is_explicitly_unverifiable() {
    let mut fixture = Fixture::new(false);
    fixture.envenom(3);
    for event in &mut fixture.events {
        if event["event"] == "power_attempt"
            && event["data"]["cause"]
                .as_u64()
                .is_some_and(|cause| cause != 0)
        {
            event["data"]["cause"] = json!(0);
        }
    }
    let result = fixture.finish();
    assert!(!result.complete);
    assert!(result.checks.values().flatten().any(|check| {
        check
            .text()
            .1
            .contains("no observable canonical application child")
    }));
}

#[test]
fn wrong_raw_tick_receiver_cannot_accumulate_independent_credit() {
    for changed in ["poison_tick", "poison_damage"] {
        let mut fixture = Fixture::new(false);
        fixture.grant("SNAKEBITE", 7);
        fixture.tick(&[("SNAKEBITE", 7)], None);
        let event = fixture
            .events
            .iter_mut()
            .find(|event| event["event"] == changed)
            .expect("tick fixture");
        event["data"]["target"]["instance"] = json!(999);
        let result = fixture.finish();
        assert!(!result.complete);
        assert!(result.credits.is_empty());
    }
}

#[test]
fn consistent_fiction_about_envenom_amount_or_completion_is_not_provenance() {
    for duplicate in [false, true] {
        let mut fixture = Fixture::new(false);
        fixture.envenom(3);
        if duplicate {
            let end = fixture
                .events
                .iter()
                .find(|event| event["event"] == "envenom_end")
                .expect("callback fixture")["data"]
                .clone();
            fixture.push("envenom_end", end);
        } else {
            for event in &mut fixture.events {
                if event["event"] == "envenom_trigger" {
                    event["data"]["amount"] = json!(99);
                }
                if event["event"] == "power_attempt" && event["data"]["power"] == "POISON_POWER" {
                    event["data"]["amount"] = json!("99");
                }
            }
        }
        fixture.tick(&[("ENVENOM", 3)], None);
        let result = fixture.finish();
        assert!(!result.complete);
        assert!(result.credits.is_empty());
    }
}

#[test]
fn independently_reconstructed_nine_turn_subtotals_are_150_and_477() {
    let mut fixture = Fixture::new(false);
    for turn in 0..9 {
        let (envenom, snakebite) = if turn == 8 { (45, 22) } else { (54, 16) };
        fixture.grant("ENVENOM", envenom);
        fixture.grant("SNAKEBITE", snakebite);
        fixture.tick(&[("ENVENOM", envenom), ("SNAKEBITE", snakebite)], None);
        fixture.grants.clear();
        fixture.push("power_change",json!({"action":0,"source_frame":0,"power":"POISON_POWER","power_identity":300,"power_instance":30,"target":Fixture::creature(false),"before":envenom+snakebite,"after":0,"before_attached":true,"after_attached":false,"native":fixture.native()}));
    }
    let result = fixture.finish();
    assert!(result.complete);
    assert_eq!(
        result.credits[&Source {
            id: "ENVENOM".into(),
            kind: 0,
            player: 0
        }],
        477
    );
    assert_eq!(
        result.credits[&Source {
            id: "SNAKEBITE".into(),
            kind: 0,
            player: 0
        }],
        150
    );
    assert!(
        result
            .checks
            .values()
            .flatten()
            .all(|check| matches!(check, Check::Match(_)))
    );
}

#[test]
fn stale_producer_frame_cannot_replace_poison_command_entry_snapshot() {
    let mut fixture = Fixture::new(false);
    fixture.grant("SNAKEBITE", 7);
    let frame = fixture.power_frame(true, 7);
    fixture.events[frame as usize - 1]["data"]["context"] = json!("producer");
    fixture.tick(&[("SNAKEBITE", 7)], Some(frame));
    let result = fixture.finish();
    assert!(!result.complete);
    assert!(result.credits.is_empty());
}

#[test]
fn ineligible_envenom_without_child_is_explained_from_raw_attack_facts() {
    let mut fixture = Fixture::new(false);
    fixture.envenom(3);
    for event in &mut fixture.events {
        match event["event"].as_str() {
            Some("damage_result" | "envenom_trigger") => event["data"]["unblocked"] = json!(0),
            Some("power_attempt") if event["data"]["power"] == "POISON_POWER" => {
                event["data"]["cause"] = json!(0)
            }
            _ => {}
        }
    }
    let result = fixture.finish();
    assert!(
        result
            .checks
            .values()
            .flatten()
            .any(|check| matches!(check,Check::Match(text) if text.contains("predicate is false")))
    );
    assert!(result.credits.is_empty());
}

#[test]
fn lookahead_cannot_cross_capture_failure() {
    for (command, gap) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut fixture = Fixture::new(false);
        fixture.grant("SNAKEBITE", 7);
        if !command {
            fixture.tick(&[("SNAKEBITE", 7)], None);
        }
        let event = if command {
            "command_end"
        } else {
            "poison_damage"
        };
        let completion = fixture
            .events
            .iter_mut()
            .find(|entry| entry["event"] == event)
            .expect("completion fixture");
        let data = completion["data"].clone();
        completion["event"] = json!("diagnostic");
        completion["data"] = json!({"reason":"missed raw observation"});
        if gap {
            completion["event"] = json!("missing_event");
            completion["seq"] = json!(900);
        }
        fixture.push(event, data);
        if command {
            fixture.tick(&[("SNAKEBITE", 7)], None);
        }
        let result = fixture.finish();
        assert!(!result.complete);
        assert!(result.credits.is_empty());
        assert!(result.checks.values().flatten().all(|check| !matches!(check,Check::Match(text) if text.starts_with("Independently reconstructed Poison damage"))));
    }
}
