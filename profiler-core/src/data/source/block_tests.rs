use std::collections::{BTreeMap, VecDeque};

use super::*;

fn fixture() -> State {
    let mut state = State::default();
    state.combat_started(7, "BLOCK", "normal", 0, 2);
    state
}

fn gain(state: &mut State, amount: i32, source: u64, modifiers: &[(u64, i64)], incomplete: bool) {
    let modifiers = state.parse_block_modifiers(7, modifiers, incomplete);
    assert_eq!(state.block_gained(7, amount, source, 0, modifiers), 1);
}

fn receive(state: &mut State, amount: i32, receiver: i32) {
    assert_eq!(
        state.damage_unattributed(7, amount, 0, amount, 1, receiver, 0),
        1
    );
}

fn defense(state: &State) -> i64 {
    state
        .current
        .as_ref()
        .expect("fixture combat exists")
        .cards
        .iter()
        .map(|row| row.block_effective + row.blk_modifier)
        .sum()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "each step compares the independent unit queue, physical totals, and credited rows"
)]
fn fifo_tail_merges_and_capacity_match_a_queue_of_individual_points() {
    let mut state = fixture();
    let handles: Vec<_> = (0..3)
        .map(|index| state.source_capture(7, 1, index + 1, &format!("S{index}"), 0, 0, 0))
        .collect();
    let mut points: VecDeque<(usize, u64)> = VecDeque::new();
    let mut next_slice = 0;
    let mut expected = [0_i64; 4];
    let mut blocked = 0;
    let mut gained = 0;
    let mut seed = 0x57ab_10c0_u64;
    for event in 0..1600 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let operation = if event < 80 { 0 } else { (seed >> 32) % 9 };
        if operation < 5 {
            let source = if event < 80 {
                event % 3
            } else {
                (seed % 3) as usize
            };
            let amount = (seed % 4 + 1) as i32;
            let runs = points
                .iter()
                .map(|(_, slice)| *slice)
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let slice = if let Some((last_source, slice)) = points.back().copied()
                && last_source == source
            {
                slice
            } else if runs == caps::BLOCK_POOL {
                let tail = points.back().expect("full model queue has points").1;
                for (owner, slice) in &mut points {
                    if *slice == tail {
                        *owner = 3;
                    }
                }
                tail
            } else {
                next_slice += 1;
                next_slice
            };
            let owner = if points
                .back()
                .is_some_and(|(owner, last)| *owner == 3 && *last == slice)
            {
                3
            } else {
                source
            };
            points.extend(std::iter::repeat_n((owner, slice), amount as usize));
            gain(&mut state, amount, handles[source], &[], false);
            gained += i64::from(amount);
        } else if operation < 8 {
            let amount = (seed % 11 + 1) as i32;
            for _ in 0..amount {
                expected[points.pop_front().map_or(3, |(source, _)| source)] += 1;
            }
            receive(&mut state, amount, 0);
            blocked += i64::from(amount);
        } else {
            points.clear();
            assert_eq!(state.block_pool_clear(7, 0), 1);
        }
        let combat = state.current.as_ref().expect("fixture combat exists");
        assert_eq!(combat.block_total, gained);
        assert_eq!(defense(&state), blocked, "event {event}");
        for (index, expected) in expected.iter().enumerate() {
            let actual: i64 = combat
                .cards
                .iter()
                .filter(|row| {
                    if index == 3 {
                        row.kind == crate::source_kind::SourceKind::Unknown
                    } else {
                        row.id.as_ref() == format!("S{index}")
                    }
                })
                .map(|row| row.block_effective)
                .sum();
            assert_eq!(actual, *expected, "event {event}, source {index}");
        }
        if let Some(pool) = state.provenance.pools.first() {
            assert!(pool.blocks.len() <= caps::BLOCK_POOL);
            assert_eq!(
                pool.blocks.iter().map(|slice| slice.remaining).sum::<u64>(),
                points.len() as u64
            );
        }
    }
    assert!(!serde_json::from_str::<serde_json::Value>(&state.snapshot()).expect("summary parses")["coverage"]["complete"].as_bool().expect("boolean coverage"));
}

#[test]
fn repeated_nonadjacent_sources_do_not_jump_over_an_earlier_grant() {
    let mut state = fixture();
    let a = state.source_capture(7, 1, 1, "A", 0, 0, 0);
    let b = state.source_capture(7, 1, 2, "B", 0, 0, 0);
    for source in [a, b, a] {
        gain(&mut state, 2, source, &[], false);
    }
    receive(&mut state, 3, 0);
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(
        combat
            .cards
            .iter()
            .find(|row| row.id.as_ref() == "A")
            .expect("A exists")
            .block_effective,
        2
    );
    assert_eq!(
        combat
            .cards
            .iter()
            .find(|row| row.id.as_ref() == "B")
            .expect("B exists")
            .block_effective,
        1
    );
}

#[test]
fn full_pool_keeps_earlier_slices_before_the_conservative_unknown_tail() {
    let mut state = fixture();
    let a = state.source_capture(7, 1, 1, "A", 0, 0, 0);
    let b = state.source_capture(7, 1, 2, "B", 0, 0, 0);
    let overflow = state.source_capture(7, 1, 3, "OVERFLOW", 0, 1, 0);
    for index in 0..caps::BLOCK_POOL {
        gain(
            &mut state,
            3,
            if index % 2 == 0 { a } else { b },
            &[],
            false,
        );
    }
    receive(&mut state, 1, 0);
    gain(&mut state, 5, overflow, &[], false);
    let earlier = (caps::BLOCK_POOL as i32 - 1) * 3;
    receive(&mut state, earlier - 1, 0);
    assert!(
        state
            .current
            .as_ref()
            .expect("fixture combat exists")
            .cards
            .iter()
            .all(|row| row.kind != crate::source_kind::SourceKind::Unknown)
    );
    receive(&mut state, 8, 0);
    let combat = state.current.as_ref().expect("fixture combat exists");
    let unknown = combat
        .cards
        .iter()
        .find(|row| row.kind == crate::source_kind::SourceKind::Unknown)
        .expect("overflow absorption has explicit Unknown credit");
    assert_eq!((unknown.block_effective, unknown.block_gained), (8, 0));
    assert_eq!(combat.cards[2].block_gained, 5);
    assert_eq!(combat.cards[2].block_effective, 0);
    assert_eq!(defense(&state), combat.block_total);
    assert!(state.provenance.pools[0].blocks.is_empty());
}

#[test]
fn missing_provenance_preserves_receiver_absorption_without_inventing_gains() {
    let mut state = fixture();
    for receiver in 0..=i32::from(TEAM_SLOT) {
        receive(&mut state, receiver + 1, receiver);
    }
    let combat = state.current.as_ref().expect("fixture combat exists");
    assert_eq!(combat.block_total, 0);
    assert_eq!(defense(&state), 15);
    for row in &combat.cards {
        assert_eq!(row.block_gained, 0);
        assert_eq!(row.block_effective, i64::from(row.player) + 1);
        assert_eq!(row.kind, crate::source_kind::SourceKind::Unknown);
    }
}

struct Seats {
    weights: Vec<u64>,
    assigned: Vec<u64>,
}

impl Seats {
    fn new(weights: Vec<u64>) -> Self {
        Self {
            assigned: vec![0; weights.len()],
            weights,
        }
    }

    fn next(&mut self) -> usize {
        let winner = (0..self.weights.len())
            .max_by(|&a, &b| {
                (u128::from(self.weights[a]) * u128::from(self.assigned[b] + 1))
                    .cmp(&(u128::from(self.weights[b]) * u128::from(self.assigned[a] + 1)))
            })
            .expect("unit model has a positive weighted source");
        self.assigned[winner] += 1;
        winner
    }
}

fn mixed(weights: &[(u32, u64)]) -> SourceSnapshot {
    SourceSnapshot::normalized(
        CombatEpoch::from_wire(7).expect("epoch fits"),
        32,
        weights
            .iter()
            .map(|(row, weight)| (Destination::Row(*row), u128::from(*weight)))
            .collect(),
    )
    .expect("fixture source has positive bounded weights")
}

type Credits = BTreeMap<(u32, bool), u64>;

fn add(credits: &mut Credits, additions: Vec<(Destination, CreditField, u64)>) {
    for (destination, field, amount) in additions {
        let Destination::Row(row) = destination else {
            panic!("fixture uses explicit rows")
        };
        *credits
            .entry((row, matches!(field, CreditField::BlockModifier)))
            .or_default() += amount;
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the per-unit reference and cumulative comparisons remain visible in one bounded matrix"
)]
fn partial_modifier_consumption_matches_unit_seats_and_whole_slice_totals() {
    let roots = [
        vec![(0, 3), (1, 2)],
        vec![(2, 1), (3, 1)],
        vec![(1, 1), (4, 3)],
    ];
    for modifier_count in 0..=16 {
        for base in 0..=5 {
            let budgets: Vec<_> = (0..modifier_count)
                .map(|index| (index % 3 + 1) as u64)
                .collect();
            let total = base + budgets.iter().sum::<u64>();
            if total == 0 {
                continue;
            }
            let block = SourceBlock {
                base: mixed(&roots[0]).prefix(),
                base_original: base,
                remaining: total,
                mods: budgets
                    .iter()
                    .enumerate()
                    .map(|(index, amount)| SourceBlockMod {
                        source: mixed(&roots[index % 2 + 1]).prefix(),
                        original: *amount,
                        consumed: 0,
                    })
                    .collect(),
            };
            let mut whole = block.clone();
            let mut expected_whole = Credits::new();
            add(
                &mut expected_whole,
                whole.consume(total).expect("whole bounded slice fits"),
            );
            for chunk in [1, 2, 7] {
                let mut actual = block.clone();
                let mut outer = Seats::new(budgets.iter().copied().chain([base]).collect());
                let mut suppliers: Vec<_> = (0..=modifier_count)
                    .map(|index| {
                        let roots = if index == modifier_count {
                            &roots[0]
                        } else {
                            &roots[index % 2 + 1]
                        };
                        Seats::new(roots.iter().map(|(_, weight)| *weight).collect())
                    })
                    .collect();
                let mut expected = Credits::new();
                let mut credited = Credits::new();
                let mut consumed = 0;
                while consumed < total {
                    let take = chunk.min(total - consumed);
                    for point in consumed + 1..=consumed + take {
                        let channel = if modifier_count == 0 {
                            0
                        } else if modifier_count == 1 {
                            if point * budgets[0] / total > (point - 1) * budgets[0] / total {
                                0
                            } else {
                                1
                            }
                        } else {
                            outer.next()
                        };
                        let root = suppliers[channel].next();
                        let modifier = channel != modifier_count;
                        let row = if modifier {
                            roots[channel % 2 + 1][root].0
                        } else {
                            roots[0][root].0
                        };
                        *expected.entry((row, modifier)).or_default() += 1;
                    }
                    add(
                        &mut credited,
                        actual.consume(take).expect("partial bounded slice fits"),
                    );
                    consumed += take;
                    assert_eq!(
                        credited, expected,
                        "modifiers {modifier_count}, base {base}, prefix {consumed}"
                    );
                    assert_eq!(credited.values().sum::<u64>(), consumed);
                }
                assert_eq!(
                    credited, expected_whole,
                    "chunked and whole allocation agree"
                );
            }
        }
    }
}

#[test]
fn accepted_modifiers_and_overclaims_keep_only_the_physical_block_budget() {
    for (amount, credit) in [(16, 1), (3, 2), (i32::MAX, i32::MAX)] {
        let mut state = fixture();
        let producer = state.source_capture(7, 1, 1, "PRODUCER", 0, 0, 0);
        let modifiers: Vec<_> = (0..16)
            .map(|index| {
                (
                    state.source_capture(7, 1, index + 2, &format!("MOD{index}"), 0, 1, 0),
                    i64::from(credit),
                )
            })
            .collect();
        gain(&mut state, amount, producer, &modifiers, false);
        assert_eq!(state.provenance.pools[0].blocks[0].remaining, amount as u64);
        receive(&mut state, amount, 0);
        assert_eq!(defense(&state), i64::from(amount));
        let combat = state.current.as_ref().expect("fixture combat exists");
        assert_eq!(combat.block_total, i64::from(amount));
        assert_eq!(combat.cards[0].block_gained, i64::from(amount));
        assert_eq!(combat.cards[0].block_effective, 0);
        if amount == 16 {
            assert!(combat.cards.iter().skip(1).all(|row| row.blk_modifier == 1));
        }
    }
}
