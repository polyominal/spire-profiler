//! Snapshot policy version changes when identical observations receive different
//! credit. Coverage describes observation and accounting failures, independently
//! of unknown source rows. The host wraps these summaries with storage metadata.

use serde::Serialize;

use super::state::{CardStat, Combat, CombatPhase, Coverage, PlayerSlotState, State, caps};

pub(super) const POLICY_VERSION: u32 = 1;

#[derive(Serialize)]
struct Summary<'a> {
    policy_version: u32,
    combat_id: u32,
    encounter_id: &'a str,
    encounter_type: &'a str,
    started_at: i64,
    result: &'a str,
    turns: u32,
    plays: u32,
    generated_plays: u32,
    generation_triggers: u32,
    damage_received: i64,
    block_total: i64,
    potions_used: u32,
    cards: &'a [CardStat],
    coverage: Coverage,
}

impl State {
    pub fn combat_started(
        &mut self,
        seq: u32,
        encounter: &str,
        kind: &str,
        started_at: i64,
        player_count: i32,
    ) -> u64 {
        if seq == 0
            || seq <= self.last_combat_seq
            || self
                .current
                .as_ref()
                .is_some_and(|combat| combat.seq >= seq)
        {
            self.capture_failed("combat-epoch");
            return 0;
        }
        self.last_combat_seq = seq;
        self.discard_combat();
        self.poisoned = false;
        self.coverage = Coverage {
            complete: true,
            ..Coverage::default()
        };
        let players = player_count.clamp(0, caps::MAX_PLAYERS as i32) as usize;
        self.per_player = (0..players).map(|_| PlayerSlotState::default()).collect();
        self.current = Some(Combat {
            seq,
            encounter_id: encounter.into(),
            encounter_type: kind.into(),
            started_at,
            player_count: players,
            ..Combat::default()
        });
        if player_count != players as i32 {
            self.capture_failed("player-count");
        }
        self.revision = self.revision.saturating_add(1);
        u64::from(seq)
    }

    pub(crate) fn capture_failed(&mut self, reason: &str) {
        self.coverage.complete = false;
        self.coverage.failures = self.coverage.failures.saturating_add(1);
        self.coverage.add_reason(reason);
    }

    pub fn snapshot(&self) -> String {
        let Some(combat) = self.current.as_ref() else {
            return "null".into();
        };
        let mut coverage = self.coverage.clone();
        self.sources.append_coverage(&mut coverage);
        if combat.row_capacity_logged {
            coverage.complete = false;
            coverage.failures = coverage.failures.saturating_add(1);
            coverage.add_reason("row-capacity");
        }
        let summary = Summary {
            policy_version: POLICY_VERSION,
            combat_id: combat.seq,
            encounter_id: &combat.encounter_id,
            encounter_type: &combat.encounter_type,
            started_at: combat.started_at,
            result: match combat.phase {
                CombatPhase::Active => "active",
                CombatPhase::Finished(result) => result.name(),
            },
            turns: combat.turns,
            plays: combat.plays,
            generated_plays: combat.generated_plays,
            generation_triggers: combat.generation_triggers,
            damage_received: combat.damage_received,
            block_total: combat.block_total,
            potions_used: combat.potions_used,
            cards: &combat.cards,
            coverage,
        };
        serde_json::to_string(&summary)
            .expect("snapshot contains only serializable integers and strings")
    }
}

impl Coverage {
    pub(super) fn add_reason(&mut self, reason: &str) {
        let reason = if reason.is_empty() { "capture" } else { reason };
        let end = reason
            .char_indices()
            .nth(96)
            .map_or(reason.len(), |(index, _)| index);
        let reason = &reason[..end];
        if self.reasons.iter().any(|item| item.as_ref() == reason) {
            return;
        }
        if self.reasons.len() < caps::COVERAGE_REASONS - 1 {
            self.reasons.push(reason.into());
        } else if self.reasons.len() == caps::COVERAGE_REASONS - 1
            && !self
                .reasons
                .iter()
                .any(|item| item.as_ref() == "additional-failures")
        {
            self.reasons.push("additional-failures".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_pins_measured_totals_policy_credit_and_coverage() {
        let mut state = State::default();
        state.combat_started(7, "SELF_TEST", "normal", 1234, 1);
        let source = state.source_capture(7, 1, 10, "DEFEND", 0, 0, 0);
        assert_eq!(state.block_gained(7, 5, source, 0), 1);
        assert_eq!(state.damage_unattributed(7, 8, 3, 5, 1, 0, 0), 1);
        state.capture_failed("missing-hook");
        assert_eq!(state.combat_ended(7), 1);
        insta::assert_snapshot!(state.snapshot());
    }

    #[test]
    fn coverage_labels_are_utf8_bounded_and_deduplicated_after_normalization() {
        let mut state = State::default();
        let reason = "界".repeat(100);
        state.capture_failed(&reason);
        state.capture_failed(&reason);
        assert_eq!(state.coverage.failures, 2);
        assert_eq!(state.coverage.reasons.len(), 1);
        assert_eq!(state.coverage.reasons[0].chars().count(), 96);
        state.capture_failed("additional-failures");
        for index in 0..32 {
            state.capture_failed(&format!("runtime-{index}"));
        }
        assert_eq!(
            state
                .coverage
                .reasons
                .iter()
                .filter(|reason| reason.as_ref() == "additional-failures")
                .count(),
            1
        );
    }

    #[test]
    fn all_coverage_origins_share_one_reason_bound_in_active_and_finished_snapshots() {
        for (runtime, source, rows) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, true),
        ] {
            let mut state = State::default();
            state.combat_started(1, "CAPACITY", "normal", 1, 1);
            if runtime {
                for index in 0..32 {
                    state.capture_failed(&format!("runtime-{index}"));
                }
                state.capture_failed("runtime-0");
            }
            if source {
                assert_eq!(state.source_accumulate(1, u64::MAX, 1, 0, 1), 0);
            }
            if rows {
                for index in 0..caps::COMBAT_CARDS {
                    state.source_capture(1, 4, 0, &format!("ROW-{index}"), 1, 0, 0);
                }
                assert!(
                    state
                        .current
                        .as_ref()
                        .expect("combat started")
                        .row_capacity_logged
                );
            }
            for finished in [false, true] {
                if finished {
                    assert_eq!(state.combat_ended(1), 1);
                }
                let snapshot: serde_json::Value =
                    serde_json::from_str(&state.snapshot()).expect("snapshot parses");
                assert_eq!(
                    snapshot["result"],
                    if finished { "completed" } else { "active" }
                );
                let coverage = &snapshot["coverage"];
                assert_eq!(coverage["complete"], false);
                let failures = coverage["failures"].as_u64().expect("failures integer");
                assert!(failures >= u64::from(runtime) * 33 + u64::from(source) + u64::from(rows));
                let reasons = coverage["reasons"].as_array().expect("reasons array");
                assert!(!reasons.is_empty() && reasons.len() <= 32);
                let unique: std::collections::HashSet<_> = reasons
                    .iter()
                    .map(|reason| reason.as_str().expect("reason string"))
                    .collect();
                assert_eq!(unique.len(), reasons.len());
                if runtime {
                    assert!(unique.contains("additional-failures"));
                }
            }
        }
    }

    #[test]
    fn finished_coverage_survives_cleanup_and_discard_preserves_epoch_monotonicity() {
        let mut state = State::default();
        assert_eq!(state.combat_started(10, "ONE", "normal", 1, 1), 10);
        state.capture_failed("capture");
        assert_eq!(state.source_accumulate(10, u64::MAX, 1, 0, 1), 0);
        assert_eq!(state.combat_ended(10), 1);
        let summary: serde_json::Value =
            serde_json::from_str(&state.snapshot()).expect("summary parses");
        assert_eq!(summary["coverage"]["complete"], false);
        assert!(summary["coverage"]["failures"].as_u64().expect("count") >= 2);
        state.discard_combat();
        assert_eq!(state.snapshot(), "null");
        assert_eq!(state.combat_started(10, "STALE", "normal", 1, 1), 0);
        assert_eq!(state.combat_started(11, "TWO", "normal", 2, 1), 11);
        let summary: serde_json::Value =
            serde_json::from_str(&state.snapshot()).expect("summary parses");
        assert_eq!(summary["coverage"]["complete"], true);
    }
}
