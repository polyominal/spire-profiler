//! Deterministic randomized simulation of the profiler core, inspired by
//! TigerBeetle's VOPR:
//! https://github.com/tigerbeetle/tigerbeetle/blob/97c7a8ef385270ebe0e1b75959d3d21d134629df/docs/internals/vopr.md
//! A seeded PRNG feeds every scenario; `SIM_SEED` overrides the default so a
//! failing run reproduces byte-for-byte. The lifecycle walk runs 20
//! scenarios x 40 weighted events in a wiped dir, re-checking ledger
//! invariants (segment sums, sign constraints, combat totals, queue bounds)
//! after every event, then parses the JSON back; the block-pool test does
//! the same against an independent naive FIFO model.
//!
//! The walk models the shim, not an adversary: a queued damage-modifier
//! contribution forces the next event to be an enemy hit covering the
//! queued share, leaving its attribution route free (orb fallback or power
//! context), so `ledger::apply_pending_contribs_in`'s carve stays exact on
//! both routes.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use profiler_core::data::state::{self, PendingContrib, RunOutcome, STATE, SourceKind};
use profiler_core::data::{events, ledger, records};
use profiler_core::test_util::wiped_dir;

const DEFAULT_SEED: u64 = 0x5EED_5EED_5EED_5EED;
const SCENARIOS: u32 = 20;
const EVENTS_PER_SCENARIO: u32 = 40;

const CARD_POOL: [&str; 8] = [
    "STRIKE",
    "DEFEND",
    "BASH",
    "ZAP",
    "DUALCAST",
    "BODYGUARD",
    "INFLAME",
    "NOT_YET",
];
const RELIC_POOL: [&str; 4] = [
    "CRACKED_CORE",
    "MERCURY_HOURGLASS",
    "VAJRA",
    "BOUND_PHYLACTERY",
];
const POWER_POOL: [&str; 6] = [
    "STRENGTH_POWER",
    "DEXTERITY_POWER",
    "POISON_POWER",
    "VULNERABLE_POWER",
    "WEAK_POWER",
    "DOOM_POWER",
];
const POTION_POOL: [&str; 3] = ["FIRE_POTION", "STRENGTH_POTION", "BLOCK_POTION"];

/// splitmix64: determinism is the only quality that matters here.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        self.range(i64::from(lo), i64::from(hi)) as i32
    }

    fn pick<'a>(&mut self, pool: &[&'a str]) -> &'a str {
        pool[self.below(pool.len() as u64) as usize]
    }
}

fn sim_seed() -> u64 {
    std::env::var("SIM_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SEED)
}

fn check_invariants(step: u32) {
    STATE.with(|cell| {
        let st = cell.borrow();
        if let Some(combat) = &st.current {
            check_combat_totals(combat, step);
            check_card_invariants(combat, step);
        }
        // Play source and card id are set/cleared in lockstep per slot;
        // otherwise the shim's explicit-self-id damage/block mis-resolves.
        for (slot, player) in st.per_player.iter().enumerate() {
            assert_eq!(
                player.active_play_source.is_some(),
                player.active_play_card_id.is_some(),
                "step {step}: slot {slot}: active_play_source and active_play_card_id must be set/cleared together"
            );
            if player.active_play_source.is_some() {
                assert!(
                    player.active_play_source_slot <= state::TEAM_SLOT,
                    "step {step}: slot {slot}: active_play_source_slot must stay in the source-slot vocabulary"
                );
            }
        }
        check_queue_bounds(&st, step);
    });
}

fn check_combat_totals(combat: &state::Combat, step: u32) {
    // Only a source's own triggers increment a row; this identity catches
    // leaked or double-counted plays.
    let card_plays: u64 = combat.cards.iter().map(|card| u64::from(card.plays)).sum();
    assert_eq!(
        u64::from(combat.plays) + u64::from(combat.generation_triggers),
        card_plays + u64::from(combat.generated_plays),
        "step {step}: plays + generation triggers must equal per-card plays + generated plays"
    );
    assert!(
        combat.block_total >= 0,
        "step {step}: block_total must never go negative"
    );
    assert!(
        combat.damage_received >= 0,
        "step {step}: damage_received must never go negative"
    );
}

fn check_card_invariants(combat: &state::Combat, step: u32) {
    let mut seen: std::collections::HashSet<(u8, &str, SourceKind)> =
        std::collections::HashSet::new();
    for card in &combat.cards {
        // Rows key on (player, id, kind); the key must be unique.
        assert!(
            card.player <= state::TEAM_SLOT,
            "step {step}: card {id}: row slot must stay in the source-slot vocabulary",
            id = card.id
        );
        assert!(
            seen.insert((card.player, &card.id, card.kind)),
            "step {step}: card {id}: row key (player, id, kind) must be unique",
            id = card.id
        );
        assert_eq!(
            card.damage_dealt,
            card.dmg_direct + card.dmg_attributed + card.dmg_modifier,
            "step {step}: card {id}: segment sum must equal damage_dealt",
            id = card.id
        );
        // `dealt - blocked` may legitimately dip negative when a queued
        // modifier share overlaps blocked damage.
        assert!(
            card.damage_dealt >= 0
                && card.damage_blocked >= 0
                && card.dmg_direct >= 0
                && card.dmg_attributed >= 0
                && card.dmg_modifier >= 0,
            "step {step}: card {id}: damage fields must be non-negative",
            id = card.id
        );
        assert!(
            card.block_gained >= 0,
            "step {step}: card {id}: block_gained must never go negative",
            id = card.id
        );
        // osty_killed removes the summon's unabsorbed HP from the killer's
        // credit.
        assert!(
            card.blk_modifier >= 0,
            "step {step}: card {id}: block modifier credits must be non-negative",
            id = card.id
        );
        assert!(
            card.mitigate_debuff >= 0 && card.mitigate_buff >= 0 && card.mitigate_str >= 0,
            "step {step}: card {id}: mitigation credits must be non-negative",
            id = card.id
        );
        assert!(
            card.self_damage >= 0 && card.forge >= 0,
            "step {step}: card {id}: self_damage and forge must be non-negative",
            id = card.id
        );
    }
}

fn check_queue_bounds(st: &state::State, step: u32) {
    assert!(
        st.context_stack.len() <= state::caps::CONTEXT_STACK,
        "step {step}: context stack overflow"
    );
    assert!(
        st.orb_sources.len() <= state::caps::ORB_SOURCES,
        "step {step}: orb source table overflow"
    );
    for (slot, player) in st.per_player.iter().enumerate() {
        assert!(
            player.block_pool.len() <= state::caps::BLOCK_POOL,
            "step {step}: slot {slot}: block pool overflow"
        );
        assert!(
            player.pending_block_contribs.len() <= state::caps::PENDING_BLOCK_CONTRIBS,
            "step {step}: slot {slot}: pending block contrib queue overflow"
        );
        assert!(
            player.pending_contribs.len() <= state::caps::PENDING_CONTRIBS,
            "step {step}: slot {slot}: pending contrib queue overflow"
        );
        assert!(
            player.osty_stack.len() <= state::caps::OSTY_STACK,
            "step {step}: slot {slot}: osty stack overflow"
        );
    }
    assert!(
        st.power_sources.len() <= state::caps::POWER_SOURCES,
        "step {step}: power source table overflow"
    );
    assert!(
        st.generated_instances.len() <= state::caps::GENERATED_INSTANCES,
        "step {step}: generated instance table overflow"
    );
    assert!(
        st.doom_layers.len() <= state::caps::DOOM_LAYERS,
        "step {step}: doom layer table overflow"
    );
    assert!(
        st.doom_targets.len() <= state::caps::DOOM_TARGETS,
        "step {step}: doom target table overflow"
    );
    assert!(
        st.str_reductions.len() <= state::caps::STR_REDUCTIONS,
        "step {step}: str reduction table overflow"
    );
    assert!(
        st.debuff_layers.len() <= state::caps::DEBUFF_LAYERS,
        "step {step}: debuff layer table overflow"
    );
    for entry in st.per_player.iter().flat_map(|slot| &slot.block_pool) {
        assert!(
            entry.mods.len() <= state::BlockEntry::MAX_MODS,
            "step {step}: block chunk modifier breakdown overflow"
        );
    }
}

/// A queued modifier contribution forces the next event to be an enemy
/// hit covering the share.
fn drive_one_event(rng: &mut Rng, follow_up: &mut bool) {
    let roll = rng.below(99);
    if roll <= 47 {
        drive_common_event(rng, follow_up, roll);
    } else {
        drive_rare_event(rng, roll);
    }
}

fn drive_common_event(rng: &mut Rng, follow_up: &mut bool, roll: u64) {
    match roll {
        0..=15 => drive_card_play(rng, follow_up),
        16..=27 => drive_damage(rng, follow_up),
        28..=33 => drive_block_gained(rng),
        34..=37 => {
            events::damage_modifier_contribution(
                rng.pick(&POWER_POOL),
                rng.range_i32(0, 2),
                rng.range_i32(1, 8),
                0,
            );
            *follow_up = true;
        }
        38..=41 => events::block_modifier_contribution(
            rng.pick(&POWER_POOL),
            rng.range_i32(0, 2),
            rng.range_i32(1, 8),
            0,
        ),
        42..=47 => drive_power_applied(rng),
        _ => unreachable!("rolls 48+ are dispatched elsewhere"),
    }
}

// A single linear dispatch; splitting it would bury the roll mapping.
#[allow(clippy::too_many_lines)]
fn drive_rare_event(rng: &mut Rng, roll: u64) {
    match roll {
        48..=50 => {
            let power = if rng.below(2) == 0 {
                "STRENGTH_POWER"
            } else {
                rng.pick(&POWER_POOL)
            };
            events::power_decreased(
                power,
                rng.range_i32(1, 5),
                rng.next_u64(),
                rng.range_i32(0, 1),
                rng.range_i32(0, 3),
            );
        }
        51..=52 => {
            // The shim's OnUseWrapper prefix precedes the effects and the
            // PotionUsed postfix follows them; the walk drives both.
            let potion = rng.pick(&POTION_POOL);
            events::potion_context_begin(potion, 0);
            events::potion_used(potion, 0);
        }
        53..=55 => events::orb_channeled(rng.range_i32(1, 999), 0),
        56..=58 => events::orb_context_begin(rng.range_i32(1, 999), 0),
        59..=61 => events::turn_started(),
        62..=63 => events::context_begin(
            rng.pick(&RELIC_POOL),
            rng.range_i32(0, 5),
            rng.range_i32(0, 4),
        ),
        64..=65 => events::context_end(),
        66..=67 => events::doom_target_capture(rng.next_u64() as i32, rng.range_i32(1, 20)),
        68 => events::doom_kills_completed(),
        69..=70 => {
            let source = if rng.below(3) == 0 {
                rng.pick(&CARD_POOL)
            } else {
                ""
            };
            events::osty_summoned(source, rng.range_i32(0, 5), rng.range_i32(1, 10), 0);
        }
        71..=72 => {
            // Absorb is signaled as damage_dealt with OstyFlagAbsorbed.
            let absorbed = rng.range_i32(1, 10);
            events::damage_dealt(events::DamageDealt {
                total: absorbed,
                unblocked: absorbed,
                osty_flag: 2,
                ..events::DamageDealt::default()
            });
        }
        73 => events::osty_killed(0),
        74..=75 => events::forge(
            if rng.below(2) == 0 {
                rng.pick(&RELIC_POOL)
            } else {
                ""
            },
            rng.range_i32(0, 5),
            rng.range_i32(1, 5),
            rng.range_i32(0, 3),
        ),
        76..=77 => drive_card_generated(rng),
        78..=79 => events::weak_mitigation(rng.range_i32(1, 8), rng.next_u64()),
        80..=81 => events::buff_mitigation(rng.pick(&POWER_POOL), rng.range_i32(1, 8)),
        82..=83 => events::enemy_hit_context(rng.range_i32(1, 20), rng.range_i32(-8, 8)),
        _ => events::block_pool_clear(0),
    }
}

fn drive_card_play(rng: &mut Rng, follow_up: &mut bool) {
    let card_id = rng.pick(&CARD_POOL);
    let card_hash = if rng.below(3) == 0 {
        rng.range_i32(1, 60_000)
    } else {
        0
    };
    events::card_play_started(
        card_id,
        rng.range_i32(0, 2),
        rng.range_i32(1, 3),
        card_hash,
        0,
    );
    for _ in 0..rng.below(4) {
        drive_in_play_event(rng, follow_up);
    }
    events::card_play_finished(0);
}

fn drive_in_play_event(rng: &mut Rng, follow_up: &mut bool) {
    match rng.below(5) {
        0 | 1 => drive_damage(rng, follow_up),
        2 => drive_block_gained(rng),
        3 => drive_power_applied(rng),
        _ => drive_card_generated(rng),
    }
}

/// A follow-up stays an enemy hit covering the queued share, steered onto
/// an attributed route half the time.
#[allow(clippy::too_many_lines)]
fn drive_damage(rng: &mut Rng, follow_up: &mut bool) {
    let queued: i64 = STATE.with(|cell| {
        cell.borrow()
            .per_player
            .iter()
            .flat_map(|slot| &slot.pending_contribs)
            .map(|p| p.amount)
            .sum::<i64>()
    });
    let total = if *follow_up {
        rng.range(1, 30).max(queued)
    } else {
        rng.range(1, 30)
    };
    let blocked = rng.range(0, total);
    let unblocked = total - blocked;
    let to_player = if *follow_up { 0 } else { rng.range_i32(0, 2) };
    let osty_flag = if *follow_up {
        0
    } else {
        match rng.below(10) {
            0..=1 => 1,
            2..=3 => 2,
            _ => 0,
        }
    };
    let card_source = if *follow_up {
        // Half the follow-ups drop the explicit source and prepare a live
        // orb fallback, so the hit resolves attributed and the segment
        // carve in `apply_pending_contribs_in` must hold there.
        if rng.below(2) == 0 {
            events::context_begin(
                rng.pick(&RELIC_POOL),
                rng.range_i32(0, 2),
                rng.range_i32(0, 4),
            );
            let hash = rng.range_i32(1, 999);
            events::orb_channeled(hash, 0);
            events::orb_context_begin(hash, 0);
            events::context_end();
            ""
        } else {
            rng.pick(&CARD_POOL)
        }
    } else if rng.below(2) == 0 {
        rng.pick(&CARD_POOL)
    } else {
        ""
    };
    *follow_up = false;
    let card_source_slot = rng.range_i32(0, 4);
    // A rare player kill drives the defeat record.
    let player_killed = to_player != 0 && rng.below(8) == 0;
    let receiver = if rng.below(4) == 0 { 0 } else { rng.next_u64() };
    let dealer = if rng.below(2) == 0 { 0 } else { rng.next_u64() };
    events::damage_dealt(events::DamageDealt {
        total: total as i32,
        unblocked: unblocked as i32,
        blocked: blocked as i32,
        card_source_id: card_source,
        to_player,
        receiver_hash: receiver,
        osty_flag,
        dealer_hash: dealer,
        dealer_slot: 0,
        receiver_slot: 0,
        card_source_slot,
    });
    if player_killed {
        // The shim's Kill patch double-fires for a player damage kill; the
        // walk drives the same double-fire.
        events::player_died(0);
    }
}

fn drive_block_gained(rng: &mut Rng) {
    let card_id = if rng.below(3) == 0 {
        rng.pick(&CARD_POOL)
    } else {
        ""
    };
    events::block_gained(rng.range_i32(1, 25), card_id, 0, rng.range_i32(0, 4));
}

fn drive_power_applied(rng: &mut Rng) {
    let power = rng.pick(&POWER_POOL);
    let creature = if rng.below(4) == 0 { 0 } else { rng.next_u64() };
    let is_player = rng.range_i32(0, 1);
    let player_slot = if is_player != 0 {
        rng.range_i32(0, 3)
    } else {
        0
    };
    events::power_applied(power, rng.range_i32(1, 5), creature, is_player, player_slot);
}

fn drive_card_generated(rng: &mut Rng) {
    let source = if rng.below(3) == 0 {
        rng.pick(&CARD_POOL)
    } else {
        ""
    };
    events::card_generated(
        rng.range_i32(1, 60_000),
        source,
        rng.range_i32(0, 5),
        rng.range_i32(0, 3),
    );
}

#[test]
fn randomized_combat_lifecycle_invariants() {
    let base_seed = sim_seed();
    for scenario in 0..SCENARIOS {
        let mut rng = Rng::new(base_seed ^ u64::from(scenario).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let base = wiped_dir(&format!("sim/lifecycle-{scenario}"));
        events::test_reset();
        events::init(&base);
        events::set_run_meta(7);
        events::run_started(
            "SIM_CHAR",
            rng.range_i32(0, 20),
            "Standard",
            "SIM_SEED",
            rng.range_i32(0, 1),
            "SIM_NET",
            // No StartTime forwarded: started_at falls back to the clock.
            0,
        );
        events::combat_started("SIM_ENCOUNTER", "test");
        let mut follow_up = false;
        for step in 0..EVENTS_PER_SCENARIO {
            drive_one_event(&mut rng, &mut follow_up);
            check_invariants(step);
        }
        events::combat_ended();
        // The walk may have killed the player (drive_damage's rare to-player
        // kill); the run result must mirror the combat record it closes.
        let player_died = STATE.with(|cell| {
            let st = cell.borrow();
            !st.per_player.is_empty() && st.per_player.iter().all(|slot| slot.died)
        });
        events::run_ended(if player_died {
            RunOutcome::Defeat
        } else {
            RunOutcome::Victory
        });
        check_written_files(&base, player_died);
        let _ = fs::remove_dir_all(&base);
    }
}

fn store_combat_ids(runs_dir: &Path) -> Vec<u32> {
    let mut ids: Vec<u32> = fs::read_dir(runs_dir)
        .expect("runs dir must be written")
        .flatten()
        .filter_map(|entry| {
            let run_dir = entry.path();
            if !run_dir.is_dir() {
                return None;
            }
            run_dir
                .file_name()
                .and_then(|name| name.to_string_lossy().parse::<u32>().ok())
        })
        .flat_map(|run_id| {
            fs::read_dir(runs_dir.join(run_id.to_string()))
                .expect("run dir must be written")
                .flatten()
                .filter_map(move |entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let stem = name.strip_suffix(".json")?;
                    stem.parse::<u32>().ok()
                })
        })
        .collect();
    ids.sort_unstable();
    ids
}

fn check_written_files(base: &Path, player_died: bool) {
    let runs_dir = base.join("runs");
    let ids = store_combat_ids(&runs_dir);
    assert_eq!(ids.len(), 1, "exactly one combat record per scenario");
    let combats_text =
        fs::read_to_string(runs_dir.join("1").join("1.json")).expect("combat file must be written");
    let parsed = [records::parse_combat_doc(&combats_text).expect("combat doc must parse back")];
    assert_eq!(parsed.len(), 1, "exactly one combat record per scenario");
    let rec = &parsed[0];
    assert_eq!(rec.combat_id, 1, "the scenario's only combat is seq 1");
    assert_eq!(
        rec.result,
        if player_died { "defeat" } else { "completed" },
        "the combat result must mirror whether the walk killed the player"
    );
    // The generation-tree model carries no origin field on any card row, so
    // any hit here is a stale assertion string rather than a real field.
    assert!(
        !combats_text.contains("\"origin\""),
        "the persisted record must contain no origin field"
    );
    check_wire_shape(&combats_text, rec);
    // The persisted record must mirror the finished in-memory combat
    // (the write/read pairing rule).
    STATE.with(|cell| {
        let st = cell.borrow();
        let combat = st
            .current
            .as_ref()
            .expect("the finished combat stays in state");
        assert_eq!(
            rec.turns, combat.turns,
            "written turns must match the ledger"
        );
        assert_eq!(
            rec.damage_received, combat.damage_received,
            "written damage must match the ledger"
        );
        assert_eq!(
            rec.cards.len(),
            combat.cards.len(),
            "written card count must match the ledger"
        );
    });
    check_no_sub_rows();
    check_run_and_store_files(base, player_died);
}

/// i64 epoch timestamps, no roster/profile, zero-omission on numeric fields.
fn check_wire_shape(combats_text: &str, rec: &records::CombatRec) {
    assert!(
        !combats_text.contains("\"started_at\":\""),
        "combat started_at must be an epoch integer, not an ISO string"
    );
    assert!(
        !combats_text.contains("\"players\"") && !combats_text.contains("\"profile\""),
        "combat docs must not carry the roster or profile"
    );
    assert!(
        !combats_text.contains("\"damage_unblocked\""),
        "card rows must not carry the derivable damage_unblocked"
    );
    check_absent_equals_zero(combats_text, rec);
}

const CARD_NUMERIC_FIELDS: [&str; 14] = [
    "plays",
    "damage_dealt",
    "damage_blocked",
    "block_gained",
    "block_effective",
    "forge",
    "dmg_direct",
    "dmg_attributed",
    "dmg_modifier",
    "blk_modifier",
    "mitigate_debuff",
    "mitigate_buff",
    "mitigate_str",
    "self_damage",
];

fn card_numeric(card: &records::CardRec, name: &str) -> i64 {
    match name {
        "plays" => i64::from(card.plays),
        "damage_dealt" => card.damage_dealt,
        "damage_blocked" => card.damage_blocked,
        "block_gained" => card.block_gained,
        "block_effective" => card.block_effective,
        "forge" => card.forge,
        "dmg_direct" => card.dmg_direct,
        "dmg_attributed" => card.dmg_attributed,
        "dmg_modifier" => card.dmg_modifier,
        "blk_modifier" => card.blk_modifier,
        "mitigate_debuff" => card.mitigate_debuff,
        "mitigate_buff" => card.mitigate_buff,
        "mitigate_str" => card.mitigate_str,
        "self_damage" => card.self_damage,
        _ => unreachable!("every schema field name is covered"),
    }
}

fn check_absent_equals_zero(combats_text: &str, rec: &records::CombatRec) {
    let doc: serde_json::Value = serde_json::from_str(combats_text).expect("combat doc is JSON");
    let rows = doc["cards"]
        .as_array()
        .expect("the raw record carries a cards array");
    assert_eq!(
        rows.len(),
        rec.cards.len(),
        "raw and parsed row counts must agree"
    );
    for (raw, parsed) in rows.iter().zip(&rec.cards) {
        for name in CARD_NUMERIC_FIELDS {
            let value = card_numeric(parsed, name);
            match raw.get(name) {
                Some(json) => assert_eq!(
                    json.as_i64(),
                    Some(value),
                    "present field '{name}' must carry the parsed value"
                ),
                None => assert_eq!(
                    value, 0,
                    "absent field '{name}' must read as zero (absent == zero)"
                ),
            }
        }
    }
}

fn check_run_and_store_files(base: &Path, player_died: bool) {
    let runs_text =
        fs::read_to_string(base.join("runs.jsonl")).expect("runs.jsonl must be written");
    let runs: serde_json::Value = serde_json::from_str(
        runs_text
            .lines()
            .next()
            .expect("runs.jsonl must hold one run line"),
    )
    .expect("the run line must be valid JSON");
    let runs = [runs];
    assert_eq!(runs.len(), 1, "exactly one run record per scenario");
    assert_eq!(runs[0]["run_id"], 1);
    assert_eq!(
        runs[0]["outcome"],
        if player_died { "defeat" } else { "victory" },
        "the run record's outcome must mirror the walk's player death"
    );
    assert!(
        !runs_text.contains("\"started_at\":\""),
        "run started_at must be an epoch integer, not an ISO string"
    );
    // runs.jsonl keeps the roster but no per-source cards[] array.
    assert!(
        runs_text.contains("\"players\""),
        "the run record must carry the roster"
    );
    assert!(
        !runs_text.contains("\"cards\""),
        "runs.jsonl must not carry the per-source cards array"
    );
    let snapshot = fs::read_to_string(base.join("runs").join("1").join("1.json"))
        .expect("combat store file must be written");
    assert!(snapshot.contains("\"combat_id\":1"));
}

fn check_no_sub_rows() {
    let mut rows =
        [profiler_core::ui::ui_model::UiRow::default(); profiler_core::ui::ui_model::MAX_UI_ROWS];
    let n = profiler_core::ui::snapshot::ui_snapshot_rows(
        profiler_core::ui::ui_model::UiTab::Combat,
        &mut rows,
    );
    const ALLOWED: u8 = profiler_core::ui::ui_model::ROW_FLAG_SELF
        | profiler_core::ui::ui_model::ROW_FLAG_SELF_SOLO;
    for row in &rows[..n] {
        assert_eq!(
            row.flags & !ALLOWED,
            0,
            "no flags other than the self-damage pair may be set (sub rows are gone)"
        );
    }
}

#[derive(Debug)]
struct NaiveChunk {
    id: String,
    kind: SourceKind,
    base_original: i64,
    base_consumed: i64,
    remaining: i64,
    mods: Vec<NaiveMod>,
}

fn push_chunk(id: &str, base: i64) {
    STATE.with(|cell| {
        ledger::block_pool_push_in(&mut cell.borrow_mut(), id, SourceKind::Card, base, 0, 0);
    });
}

fn consume_chunk(amount: i64) -> i64 {
    STATE.with(|cell| ledger::block_pool_consume_in(&mut cell.borrow_mut(), amount, 0))
}

#[derive(Debug)]
struct NaiveMod {
    id: String,
    kind: SourceKind,
    original: i64,
    consumed: i64,
}

fn naive_push(
    pool: &mut Vec<NaiveChunk>,
    pending: &[(String, SourceKind, i64)],
    id: &str,
    kind: SourceKind,
    base: i64,
) {
    if pending.is_empty()
        && let Some(entry) = pool
            .iter_mut()
            .find(|e| e.mods.is_empty() && e.id == id && e.kind == kind)
    {
        entry.remaining += base;
        entry.base_original += base;
        return;
    }
    if pool.len() >= state::caps::BLOCK_POOL {
        return;
    }
    let mut entry = NaiveChunk {
        id: id.to_owned(),
        kind,
        base_original: base,
        base_consumed: 0,
        remaining: base,
        mods: Vec::new(),
    };
    for (mod_id, mod_kind, amount) in pending.iter().take(state::BlockEntry::MAX_MODS) {
        entry.mods.push(NaiveMod {
            id: mod_id.clone(),
            kind: *mod_kind,
            original: *amount,
            consumed: 0,
        });
        entry.remaining += amount;
    }
    if entry.remaining <= 0 {
        return;
    }
    pool.push(entry);
}

fn naive_consume(
    pool: &mut Vec<NaiveChunk>,
    credits: &mut HashMap<String, (i64, i64)>,
    blocked: i64,
) -> i64 {
    let mut remaining = blocked;
    let mut credited = 0;
    let mut i = 0;
    while i < pool.len() && remaining > 0 {
        let take = pool[i].remaining.min(remaining);
        if take > 0 {
            let total =
                pool[i].base_original + pool[i].mods.iter().map(|m| m.original).sum::<i64>();
            let consumed_after = (total - pool[i].remaining) + take;
            let mut base_delta = if total > 0 {
                (pool[i].base_original * consumed_after) / total - pool[i].base_consumed
            } else {
                0
            };
            let mut allocated = base_delta;
            let mut mod_deltas = vec![0_i64; pool[i].mods.len()];
            for (j, m) in pool[i].mods.iter().enumerate() {
                mod_deltas[j] = if total > 0 {
                    (m.original * consumed_after) / total - m.consumed
                } else {
                    0
                };
                allocated += mod_deltas[j];
            }
            base_delta += take - allocated;
            if base_delta > 0 {
                let entry = credits.entry(pool[i].id.clone()).or_insert((0, 0));
                entry.0 += base_delta;
                credited += base_delta;
            }
            for (j, m) in pool[i].mods.iter_mut().enumerate() {
                m.consumed += mod_deltas[j];
                if mod_deltas[j] > 0 {
                    let entry = credits.entry(m.id.clone()).or_insert((0, 0));
                    entry.1 += mod_deltas[j];
                    credited += mod_deltas[j];
                }
            }
            pool[i].base_consumed += base_delta;
            pool[i].remaining -= take;
            remaining -= take;
        }
        if pool[i].remaining <= 0 {
            pool.remove(i);
        } else {
            i += 1;
        }
    }
    credited
}

#[test]
fn block_pool_consume_matches_naive_model() {
    const SOURCES: [&str; 5] = [
        "DEFEND",
        "ARMAMENTS",
        "IRON_WAVE",
        "BODYGUARD",
        "CRIMSON_MANTLE",
    ];
    const MODIFIERS: [&str; 3] = ["DEXTERITY_POWER", "FOOTWORK", "TEMPORARY_DEXTERITY_POWER"];
    const ROUNDS: u32 = 40;

    let mut rng = Rng::new(sim_seed() ^ 0xB10C_3001_C0DE);
    let base = wiped_dir("sim/blockpool");
    events::test_reset();
    events::init(&base);
    events::combat_started("BLOCKPOOL_SIM", "test");

    let mut naive_pool: Vec<NaiveChunk> = Vec::new();
    let mut naive_credits: HashMap<String, (i64, i64)> = HashMap::new();
    let mut pending: Vec<(String, SourceKind, i64)> = Vec::new();

    for _round in 0..ROUNDS {
        for _ in 0..rng.below(5) + 1 {
            if rng.below(3) == 0 {
                for _ in 0..rng.below(4) + 1 {
                    pending.push((
                        rng.pick(&MODIFIERS).to_owned(),
                        SourceKind::Power,
                        rng.range(1, 10),
                    ));
                }
            }
            let id = rng.pick(&SOURCES);
            let base = rng.range(0, 25);
            STATE.with(|cell| {
                let mut st = cell.borrow_mut();
                st.slot_state_mut(0).pending_block_contribs = pending
                    .iter()
                    .map(|(id, kind, amount)| PendingContrib {
                        id: id.clone(),
                        kind: *kind,
                        player: 0,
                        amount: *amount,
                    })
                    .collect();
            });
            push_chunk(id, base);
            naive_push(&mut naive_pool, &pending, id, SourceKind::Card, base);
            pending.clear();
        }
        for _ in 0..rng.below(3) + 1 {
            let amount = if rng.below(4) == 0 {
                rng.range(1, 500)
            } else {
                rng.range(1, 30)
            };
            let credited = consume_chunk(amount);
            let naive_credited = naive_consume(&mut naive_pool, &mut naive_credits, amount);
            assert_eq!(
                credited, naive_credited,
                "the pool's credited total must match the naive model"
            );
            compare_pool_and_credits(&naive_pool, &naive_credits);
        }
    }
    let credited = consume_chunk(10_000);
    let naive_credited = naive_consume(&mut naive_pool, &mut naive_credits, 10_000);
    assert_eq!(credited, naive_credited);
    compare_pool_and_credits(&naive_pool, &naive_credits);
    let _ = fs::remove_dir_all(&base);
}

fn compare_pool_and_credits(
    naive_pool: &[NaiveChunk],
    naive_credits: &HashMap<String, (i64, i64)>,
) {
    STATE.with(|cell| {
        let st = cell.borrow();
        let pool = &st.per_player[0].block_pool;
        assert_eq!(
            pool.len(),
            naive_pool.len(),
            "the pool must hold the same chunks as the naive model"
        );
        for (real, naive) in pool.iter().zip(naive_pool) {
            assert_eq!(real.id, naive.id, "chunk sources must match");
            assert_eq!(real.kind, naive.kind, "chunk kinds must match");
            assert_eq!(
                real.remaining, naive.remaining,
                "chunk remaining must match"
            );
            assert_eq!(
                real.base_original, naive.base_original,
                "chunk base_original must match"
            );
            assert_eq!(
                real.base_consumed, naive.base_consumed,
                "chunk base_consumed must match"
            );
            assert_eq!(
                real.mods.len(),
                naive.mods.len(),
                "chunk modifier counts must match"
            );
            for (m, n) in real.mods.iter().zip(&naive.mods) {
                assert_eq!(m.id, n.id, "modifier sources must match");
                assert_eq!(m.kind, n.kind, "modifier kinds must match");
                assert_eq!(m.original, n.original, "modifier originals must match");
                assert_eq!(m.consumed, n.consumed, "modifier consumed must match");
            }
        }
        let combat = st
            .current
            .as_ref()
            .expect("the block pool sim runs inside a combat");
        for (id, (base, modifier)) in naive_credits {
            let card = combat
                .cards
                .iter()
                .find(|card| card.id == *id)
                .unwrap_or_else(|| panic!("credited source '{id}' must have a ledger entry"));
            assert_eq!(
                card.block_effective, *base,
                "base credit for '{id}' must match the naive model"
            );
            assert_eq!(
                card.blk_modifier, *modifier,
                "modifier credit for '{id}' must match the naive model"
            );
        }
        // Every ledger entry in the combat came from a credit: no phantom
        // or empty sources.
        for card in &combat.cards {
            assert!(
                naive_credits.contains_key(&card.id),
                "card '{}' must have been credited something",
                card.id
            );
        }
    });
}
