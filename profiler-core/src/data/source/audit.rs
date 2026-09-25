//! Audit version 1 exposes captured provenance, not independently observed game
//! state. Poison entries retain attachment order; grants are FIFO, and source
//! weights are exact normalized integers in allocation order. `source` is the
//! retained mixture, including when empty grants or `trusted: false` make it
//! unavailable for attribution. Unknown uses the same identity as ledger rows.
//! Queries borrow state and construct output only; they never capture sources,
//! allocate handles, alter coverage, or append recording entries.

use serde::Serialize;

use super::{Destination, SourceSnapshot, State};
use crate::data::state::{CardStat, Coverage, SourceKind};
use crate::data::summary::POLICY_VERSION;

#[derive(Serialize)]
struct Audit<'a> {
    audit_version: u32,
    policy_version: u32,
    combat_id: u32,
    poison: Vec<Poison<'a>>,
    cards: &'a [CardStat],
    coverage: Coverage,
}

#[derive(Serialize)]
struct Poison<'a> {
    instance: u64,
    owner: u64,
    owner_kind: i32,
    owner_slot: u8,
    amount: i32,
    trusted: bool,
    grants: Vec<Grant<'a>>,
    source: Vec<Source<'a>>,
}

#[derive(Serialize)]
struct Grant<'a> {
    remaining: u32,
    source: Vec<Source<'a>>,
}

#[derive(Serialize)]
struct Source<'a> {
    id: &'a str,
    kind: SourceKind,
    player: u8,
    weight: u64,
}

impl SourceSnapshot {
    fn audit<'a>(&self, cards: &'a [CardStat]) -> Vec<Source<'a>> {
        self.shares()
            .iter()
            .map(|share| {
                let (id, kind, player) = match share.destination() {
                    Destination::Row(index) => {
                        debug_assert!(
                            (index as usize) < cards.len(),
                            "captured destinations belong to the unchanged combat row table"
                        );
                        let row = &cards[index as usize];
                        (row.id.as_ref(), row.kind, row.player)
                    }
                    Destination::Unknown(slot) => ("UNATTRIBUTED", SourceKind::Unknown, slot),
                };
                Source {
                    id,
                    kind,
                    player,
                    weight: share.weight(),
                }
            })
            .collect()
    }
}

impl State {
    pub(crate) fn audit_snapshot(&self) -> String {
        let Some(combat) = self.current.as_ref() else {
            return "null".into();
        };
        let audit = Audit {
            audit_version: 1,
            policy_version: POLICY_VERSION,
            combat_id: combat.seq,
            poison: self
                .provenance
                .powers
                .iter()
                .filter(|power| power.id.as_ref() == "POISON_POWER")
                .map(|power| Poison {
                    instance: power.instance,
                    owner: power.owner,
                    owner_kind: power.owner_kind as i32,
                    owner_slot: power.owner_slot,
                    amount: power.observed,
                    trusted: power.trusted,
                    grants: power
                        .grants
                        .iter()
                        .map(|grant| Grant {
                            remaining: grant.remaining(),
                            source: grant.source().audit(&combat.cards),
                        })
                        .collect(),
                    source: power.source.audit(&combat.cards),
                })
                .collect(),
            cards: &combat.cards,
            coverage: self.snapshot_coverage(combat),
        };
        serde_json::to_string(&audit)
            .expect("audit contains only serializable integers and strings")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn audit(state: &State) -> Value {
        serde_json::from_str(&state.audit_snapshot()).expect("audit JSON parses")
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the fixture keeps all FIFO transitions and their tick credits in order"
    )]
    fn poison_audit_exposes_fifo_decay_and_the_credited_tick_mixture() {
        let mut state = State::default();
        state.combat_started(7, "AUDIT", "normal", 0, 2);
        let envenom = state.source_capture(7, 1, 100, "ENVENOM", 0, 0, 0);
        let snakebite = state.source_capture(7, 1, 101, "SNAKEBITE", 0, 1, 0);
        assert_eq!(
            state.power_attached(7, 200, "POISON_POWER", 900, 1, 4, 3, envenom),
            1
        );
        assert_eq!(
            state.power_amount_changed(7, 200, "POISON_POWER", 900, 1, 4, 3, 5, snakebite),
            1
        );
        let first = audit(&state);
        assert_eq!(
            first["poison"],
            json!([{
                "instance": 200, "owner": 900, "owner_kind": 1, "owner_slot": 4,
                "amount": 5, "trusted": true,
                "grants": [
                    {"remaining": 3, "source": [{"id":"ENVENOM","kind":0,"player":0,"weight":1}]},
                    {"remaining": 2, "source": [{"id":"SNAKEBITE","kind":0,"player":1,"weight":1}]}
                ],
                "source": [
                    {"id":"ENVENOM","kind":0,"player":0,"weight":3},
                    {"id":"SNAKEBITE","kind":0,"player":1,"weight":2}
                ]
            }])
        );
        let mut expected = [0, 0];
        for amount in (1..=5).rev() {
            let source = state.source_capture(7, 2, 200, "POISON_POWER", 2, 4, 0);
            let calculation = state.damage_calculation_begin(7, source, 2, 1, 900);
            assert_ne!(calculation, 0);
            assert_eq!(
                state.damage_result_append(calculation, amount, amount, 0, 0, 4, 0),
                1
            );
            assert_eq!(state.damage_calculation_commit(calculation), 1);
            expected[0] += (amount - 2).max(0);
            expected[1] += amount.min(2);
            let after_tick = audit(&state);
            for (row, expected) in after_tick["cards"]
                .as_array()
                .expect("rows")
                .iter()
                .zip(expected)
            {
                assert_eq!(row["dmg_attributed"], expected);
                assert_eq!(row["damage_dealt"], expected);
            }
            assert_eq!(
                state.power_amount_changed(
                    7,
                    200,
                    "POISON_POWER",
                    900,
                    1,
                    4,
                    amount,
                    amount - 1,
                    0
                ),
                1
            );
            let after_decay = audit(&state);
            let grants = after_decay["poison"][0]["grants"]
                .as_array()
                .expect("grants");
            let remaining: Vec<_> = grants
                .iter()
                .map(|grant| grant["remaining"].as_i64().expect("amount"))
                .collect();
            let expected_grants: Vec<_> = [
                i64::from((amount - 3).max(0)),
                i64::from((amount - 1).clamp(0, 2)),
            ]
            .into_iter()
            .filter(|amount| *amount > 0)
            .collect();
            assert_eq!(remaining, expected_grants);
        }
        assert_eq!(expected, [6, 9]);
        let drained = audit(&state);
        assert_eq!(drained["poison"][0]["amount"], 0);
        assert_eq!(drained["poison"][0]["source"][0]["id"], "SNAKEBITE");
        assert_eq!(state.power_removed(7, 200), 1);
        assert_eq!(audit(&state)["poison"], json!([]));
    }

    #[test]
    fn audit_preserves_mixed_supplier_weights_unknown_and_untrusted_provenance() {
        let mut state = State::default();
        assert_eq!(state.audit_snapshot(), "null");
        state.combat_started(8, "MIXTURE", "normal", 0, 2);
        let a = state.source_capture(8, 1, 100, "A", 0, 0, 0);
        let b = state.source_capture(8, 1, 101, "B", 0, 1, 0);
        let mixed = state.source_accumulate(8, a, 2, b, 5);
        assert_ne!(mixed, 0);
        assert_eq!(
            state.power_attached(8, 200, "POISON_POWER", 900, 1, 4, 2, mixed),
            1
        );
        assert_eq!(
            state.power_amount_changed(8, 200, "POISON_POWER", 900, 1, 4, 2, 3, a),
            1
        );
        assert_eq!(
            state.power_attached(8, 201, "POISON_POWER", 901, 1, 4, 4, 0),
            1
        );
        assert_eq!(
            state.power_attached(8, 202, "WEAK_POWER", 902, 1, 4, 1, a),
            1
        );
        let before = audit(&state);
        assert_eq!(before["poison"].as_array().expect("powers").len(), 2);
        assert_eq!(
            before["poison"][0]["grants"][0]["source"],
            json!([
                {"id":"A","kind":0,"player":0,"weight":2},
                {"id":"B","kind":0,"player":1,"weight":3}
            ])
        );
        assert_eq!(
            before["poison"][0]["source"],
            json!([
                {"id":"A","kind":0,"player":0,"weight":3},
                {"id":"B","kind":0,"player":1,"weight":2}
            ])
        );
        assert_eq!(
            before["poison"][1]["source"],
            json!([
                {"id":"UNATTRIBUTED","kind":5,"player":4,"weight":1}
            ])
        );
        assert_eq!(state.power_provenance_invalidate(8, 200), 1);
        let after = audit(&state);
        assert_eq!(after["poison"][0]["trusted"], false);
        assert_eq!(after["poison"][0]["source"], before["poison"][0]["source"]);
        state.capture_failed("missing-hook");
        assert_eq!(state.source_accumulate(8, u64::MAX, 1, 0, 2), 0);
        state
            .current
            .as_mut()
            .expect("combat started")
            .row_capacity_logged = true;
        let summary: Value = serde_json::from_str(&state.snapshot()).expect("summary parses");
        assert_eq!(audit(&state)["coverage"], summary["coverage"]);
    }
}
