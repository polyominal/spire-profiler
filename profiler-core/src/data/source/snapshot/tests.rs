use super::super::{State, Token, TokenKind};
use super::*;
use crate::data::state::{CardStat, Combat, SourceSlot};

fn epoch() -> CombatEpoch {
    CombatEpoch::from_wire(7).expect("fixture epoch is nonzero and fits u32")
}

fn source(weights: &[(usize, u64)]) -> SourceSnapshot {
    SourceSnapshot::normalized(
        epoch(),
        caps::COMBAT_CARDS,
        weights
            .iter()
            .map(|(row, weight)| (Destination::Row(*row), u128::from(*weight)))
            .collect(),
    )
    .expect("fixture rows and positive weights fit the source bounds")
}

fn state(rows: usize) -> State {
    State {
        current: Some(Combat {
            seq: epoch().0.get(),
            cards: (0..rows).map(|_| CardStat::default()).collect(),
            ..Combat::default()
        }),
        ..State::default()
    }
}

fn unknown(slot: SourceSlot) -> u64 {
    Destination::Unknown(slot).token(epoch())
}

#[test]
fn normalization_merges_roots_without_losing_small_suppliers() {
    let snapshot = source(&[(3, 2), (1, 3), (3, 2), (5, 1)]);
    assert_eq!(
        snapshot
            .shares
            .iter()
            .map(|share| (share.destination, share.weight))
            .collect::<Vec<_>>(),
        vec![
            (Destination::Row(3), 4),
            (Destination::Row(1), 3),
            (Destination::Row(5), 1)
        ]
    );
    let large_duplicate = source(&[(2, u64::MAX), (2, u64::MAX)]);
    assert_eq!(large_duplicate.shares[0].weight, 1);
    assert_eq!(
        SourceSnapshot::normalized(
            epoch(),
            3,
            vec![
                (Destination::Row(0), u128::from(u64::MAX)),
                (Destination::Row(1), u128::from(u64::MAX - 1)),
            ]
        ),
        Err(SourceFailure::Arithmetic)
    );
}

#[test]
fn proportional_allocation_matches_naive_unit_threshold_model() {
    for a in 0..=16_u64 {
        for b in 0..=16_u64 {
            for c in 0..=4_u64 {
                let weights = [a, b, c];
                let total: u64 = weights.iter().sum();
                for amount in 0..=32_u64 {
                    let mut expected = vec![0_u64; weights.len()];
                    if total > 0 {
                        // Count integer quota boundaries inside each root's interval.
                        for unit in 1..=amount {
                            let mut end = 0;
                            for (index, weight) in weights.iter().enumerate() {
                                end += weight;
                                if amount * end >= unit * total {
                                    expected[index] += 1;
                                    break;
                                }
                            }
                        }
                    }
                    let actual = RootBudgets::proportional(amount, &weights)
                        .expect("small fixture totals fit");
                    assert_eq!(actual, expected, "amount={amount}, weights={weights:?}");
                    if amount <= total {
                        assert!(
                            actual
                                .iter()
                                .zip(weights)
                                .all(|(share, weight)| *share <= weight)
                        );
                    }
                }
            }
        }
    }
    assert_eq!(
        RootBudgets::proportional(u64::MAX, &[u64::MAX - 1, 1]),
        Ok(vec![u64::MAX - 1, 1])
    );
    assert_eq!(
        RootBudgets::proportional(1, &[u64::MAX, 1]),
        Err(SourceFailure::Arithmetic)
    );
}

#[test]
fn exact_grant_mixtures_match_independent_common_unit_model() {
    let mut rng = 0x5e_ed_32_70_u64;
    for _ in 0..512 {
        let mut grants = Vec::new();
        for _ in 0..4 {
            let mut weights = Vec::new();
            for row in 0..5 {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let weight = (rng >> 32) % 5;
                if weight > 0 {
                    weights.push((row, weight));
                }
            }
            if weights.is_empty() {
                weights.push((0, 1));
            }
            grants.push(PowerGrant {
                remaining: ((rng >> 16) % 6 + 1) as u32,
                source: source(&weights),
            });
        }
        let totals: Vec<u128> = grants
            .iter()
            .map(|grant| {
                grant
                    .source
                    .shares
                    .iter()
                    .map(|share| u128::from(share.weight))
                    .sum()
            })
            .collect();
        // A product of denominators is an exact common unit, without incremental reduction.
        let unit: u128 = totals.iter().product();
        let mut model = [0_u128; 5];
        let mut order = Vec::new();
        for (grant, total) in grants.iter().zip(totals) {
            for share in &grant.source.shares {
                let Destination::Row(row) = share.destination else {
                    panic!("fixture uses rows")
                };
                if model[row] == 0 {
                    order.push(row);
                }
                model[row] +=
                    u128::from(grant.remaining) * u128::from(share.weight) * (unit / total);
            }
        }
        let actual =
            SourceSnapshot::mixture(epoch(), 5, &grants, &mut SourceDiagnostics::default());
        let actual_total: u128 = actual
            .shares
            .iter()
            .map(|share| u128::from(share.weight))
            .sum();
        let model_total: u128 = model.iter().sum();
        assert_eq!(actual.shares.len(), order.len());
        for (share, row) in actual.shares.iter().zip(order) {
            assert_eq!(share.destination, Destination::Row(row));
            assert_eq!(
                u128::from(share.weight) * model_total,
                model[row] * actual_total
            );
        }
    }
}

#[test]
fn one_point_mixed_grants_remain_mixed_before_final_allocation() {
    let grants = [
        PowerGrant {
            remaining: 1,
            source: source(&[(0, 1), (1, 1)]),
        },
        PowerGrant {
            remaining: 1,
            source: source(&[(0, 1), (2, 2)]),
        },
    ];
    let actual = SourceSnapshot::mixture(epoch(), 3, &grants, &mut SourceDiagnostics::default());
    assert_eq!(
        actual
            .shares
            .iter()
            .map(|share| share.weight)
            .collect::<Vec<_>>(),
        [5, 3, 4]
    );
    let snapshot = source(&[(0, 1), (1, 1)]);
    let mut budget = snapshot.budgets(2);
    let mut actual = [0; 2];
    for _ in 0..2 {
        for (destination, amount) in budget.take(1).expect("one point remains") {
            let Destination::Row(row) = destination else {
                panic!("fixture uses rows")
            };
            actual[row] += amount;
        }
    }
    assert_eq!(actual, [1, 1]);
}

#[test]
fn fixed_root_budgets_survive_nonmonotone_proportional_prefixes() {
    let snapshot = source(&[(0, 2), (1, 3), (2, 5)]);
    assert_eq!(
        RootBudgets::proportional(4, &[2, 3, 5]).expect("small weights fit"),
        [0, 2, 2]
    );
    assert_eq!(
        RootBudgets::proportional(5, &[2, 3, 5]).expect("small weights fit"),
        [1, 1, 3]
    );
    for total in 0..=32 {
        let mut budgets = snapshot.budgets(total);
        let mut credited = [0; 3];
        for _ in 0..total {
            for (destination, amount) in budgets
                .take(1)
                .expect("the fixed budget has one point left")
            {
                let Destination::Row(row) = destination else {
                    panic!("fixture uses rows")
                };
                credited[row] += amount;
            }
        }
        assert_eq!(
            credited.to_vec(),
            RootBudgets::proportional(total, &[2, 3, 5]).expect("small weights fit")
        );
        assert_eq!(budgets.take(1), Err(SourceFailure::Packet));
        assert_eq!(budgets.take(0), Ok(Vec::new()));
    }
}

#[test]
fn mixture_failure_collapses_the_whole_vector_to_unknown() {
    let mut diagnostics = SourceDiagnostics::default();
    let huge = [u64::MAX, u64::MAX - 2, u64::MAX - 4];
    let grants: Vec<_> = huge
        .iter()
        .map(|total| PowerGrant {
            remaining: 1,
            source: source(&[(0, 1), (1, total - 1)]),
        })
        .collect();
    assert_eq!(
        SourceSnapshot::mixture(epoch(), 2, &grants, &mut diagnostics),
        SourceSnapshot::unknown(epoch())
    );
    let first_diagnostic = diagnostics.reported;
    SourceSnapshot::mixture(epoch(), 2, &grants, &mut diagnostics);
    assert_eq!(diagnostics.reported, first_diagnostic);
    let grants: Vec<_> = (0..=caps::SOURCE_DESTINATIONS)
        .map(|row| PowerGrant {
            remaining: 1,
            source: source(&[(row, 1)]),
        })
        .collect();
    assert_eq!(
        SourceSnapshot::mixture(epoch(), caps::COMBAT_CARDS, &grants, &mut diagnostics),
        SourceSnapshot::unknown(epoch())
    );
    let mut stale = source(&[(0, 1)]);
    stale.combat_seq += 1;
    assert_eq!(
        SourceSnapshot::mixture(
            epoch(),
            1,
            &[PowerGrant {
                remaining: 1,
                source: stale
            }],
            &mut diagnostics
        ),
        SourceSnapshot::unknown(epoch())
    );
}

#[test]
fn transfer_round_trip_preserves_destinations_and_normalizes_duplicates() {
    let mut state = state(1);
    let transfer = state.source_transfer_begin(7);
    assert_ne!(transfer, 0);
    assert_eq!(state.source_count(transfer), -1);
    assert_eq!(state.source_transfer_add(transfer, unknown(2), 6), 1);
    let row = Destination::Row(0).token(epoch());
    assert_eq!(state.source_transfer_add(transfer, row, 3), 1);
    assert_eq!(state.source_transfer_add(transfer, unknown(2), 3), 1);
    assert_eq!(state.source_transfer_seal(transfer), 1);
    assert_eq!(state.source_count(transfer), 2);
    assert_eq!(
        (
            state.source_destination(transfer, 0),
            state.source_weight(transfer, 0)
        ),
        (unknown(2), 3)
    );
    assert_eq!(
        (
            state.source_destination(transfer, 1),
            state.source_weight(transfer, 1)
        ),
        (row, 1)
    );
    for index in [i32::MIN, -1, 2, i32::MAX] {
        assert_eq!(state.source_destination(transfer, index), 0);
        assert_eq!(state.source_weight(transfer, index), 0);
    }
    assert_eq!(state.source_count(transfer), 2);
    assert_eq!(state.source_transfer_release(transfer), 1);
    assert_eq!(state.source_transfer_release(transfer), 0);
    assert_eq!(state.source_count(transfer), -1);
    let next = state.source_transfer_begin(7);
    assert_ne!(next, transfer);
    assert_eq!(state.source_transfer_release(transfer), 0);
    assert_eq!(state.source_transfer_add(next, unknown(4), 1), 1);
}

#[test]
fn malformed_transfer_writes_cannot_leave_a_partial_source_usable() {
    let mut state = state(1);
    let invalid_destinations = [
        0,
        u64::MAX,
        unknown(0) ^ (1 << 32),
        Destination::Row(1).token(epoch()),
        (7_u64 << 32) | (5 << 3) | 1,
        (7_u64 << 32) | 6,
    ];
    for destination in invalid_destinations {
        let transfer = state.source_transfer_begin(7);
        assert_eq!(state.source_transfer_add(transfer, unknown(0), 1), 1);
        assert_eq!(state.source_transfer_add(transfer, destination, 1), 0);
        assert_eq!(state.source_transfer_seal(transfer), 0);
        assert_eq!(state.source_count(transfer), -1);
        assert_eq!(state.source_transfer_release(transfer), 1);
    }
    let transfer = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_add(transfer, unknown(0), 0), 0);
    assert_eq!(state.source_transfer_add(transfer, unknown(0), 1), 0);
    assert_eq!(state.source_transfer_seal(transfer), 0);
    state.source_transfer_release(transfer);
    let empty = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_seal(empty), 0);
    assert_eq!(state.source_count(empty), -1);
}

#[test]
fn capacity_and_representation_failures_invalidate_then_release() {
    let mut state = state(caps::SOURCE_DESTINATIONS + 1);
    let mut transfers = Vec::new();
    for _ in 0..caps::SOURCE_TRANSFERS {
        let token = state.source_transfer_begin(7);
        assert_ne!(token, 0);
        transfers.push(token);
    }
    assert_eq!(state.source_transfer_begin(7), 0);
    for transfer in transfers {
        assert_eq!(state.source_transfer_release(transfer), 1);
    }
    let transfer = state.source_transfer_begin(7);
    for row in 0..caps::SOURCE_DESTINATIONS {
        assert_eq!(
            state.source_transfer_add(transfer, Destination::Row(row).token(epoch()), 1),
            1
        );
    }
    assert_eq!(
        state.source_transfer_add(transfer, Destination::Row(0).token(epoch()), 1),
        1
    );
    assert_eq!(
        state.source_transfer_add(
            transfer,
            Destination::Row(caps::SOURCE_DESTINATIONS).token(epoch()),
            1
        ),
        0
    );
    assert_eq!(state.source_transfer_seal(transfer), 0);
    assert_eq!(state.source_count(transfer), -1);
    state.source_transfer_release(transfer);
    let overflow = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_add(overflow, unknown(0), u64::MAX), 1);
    assert_eq!(
        state.source_transfer_add(overflow, unknown(1), u64::MAX - 1),
        1
    );
    assert_eq!(state.source_transfer_seal(overflow), 0);
    assert_eq!(state.source_count(overflow), -1);
}

#[test]
fn token_kind_epoch_membership_and_serial_exhaustion_are_checked() {
    let mut state = state(1);
    for wire in [0, u64::from(u32::MAX) + 1, u64::MAX, 6, 8] {
        assert_eq!(state.source_transfer_begin(wire), 0);
    }
    let transfer = state.source_transfer_begin(7);
    for invalid in [
        0,
        unknown(0),
        Destination::Row(0).token(epoch()),
        transfer + 8,
        transfer ^ (1 << 32),
        (7_u64 << 32) | 2,
        u64::MAX,
    ] {
        assert_eq!(state.source_count(invalid), -1);
        assert_eq!(state.source_transfer_release(invalid), 0);
    }
    for kind in [
        TokenKind::DamageCalculation,
        TokenKind::CardPlay,
        TokenKind::DoomBatch,
    ] {
        let wrong_kind = Token {
            epoch: epoch(),
            kind,
            payload: 1,
        }
        .encode();
        assert_eq!(state.source_transfer_add(wrong_kind, unknown(0), 1), 0);
        assert_eq!(state.source_count(wrong_kind), -1);
    }
    assert_eq!(state.source_transfer_add(transfer, unknown(0), 1), 1);
    assert_eq!(state.source_transfer_seal(transfer), 1);
    state.current.as_mut().expect("fixture combat exists").seq = 8;
    assert_eq!(state.source_count(transfer), -1);
    assert_eq!(state.source_transfer_release(transfer), 0);
    state.source_epoch(8).expect("replacement epoch is active");
    state.source_transfers.serial = PAYLOAD_MAX - 1;
    let final_token = state.source_transfer_begin(8);
    assert_ne!(final_token, 0);
    assert_eq!(state.source_transfer_release(final_token), 1);
    assert_eq!(state.source_transfer_begin(8), 0);
}

#[test]
fn synchronous_source_copy_survives_release_without_retargeting_an_epoch() {
    let mut state = state(0);
    assert_eq!(
        state.source_snapshot(7, 0),
        Ok(SourceSnapshot::unknown(epoch()))
    );
    let transfer = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_add(transfer, unknown(1), 1), 1);
    assert_eq!(state.source_transfer_add(transfer, unknown(3), 2), 1);
    assert_eq!(state.source_transfer_seal(transfer), 1);
    let saved = state
        .source_snapshot(7, transfer)
        .expect("sealed live lease can be copied");
    assert_eq!(state.source_transfer_release(transfer), 1);
    assert_eq!(
        saved
            .shares
            .iter()
            .map(|share| share.weight)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        state.source_snapshot(7, transfer),
        Err(SourceFailure::Token)
    );
    state.current.as_mut().expect("fixture combat exists").seq = 8;
    assert_eq!(state.source_snapshot(7, 0), Err(SourceFailure::Epoch));
    assert_eq!(
        state.source_snapshot(8, transfer),
        Err(SourceFailure::Epoch)
    );
    let forged = transfer ^ (7_u64 << 32) ^ (8_u64 << 32);
    assert_eq!(state.source_snapshot(8, forged), Err(SourceFailure::Token));
}

#[test]
fn monotone_prefixes_match_independent_per_seat_quotient_model() {
    let mut rng = 0x71_03_9a_88_u64;
    for _ in 0..512 {
        let count = (rng % 8 + 1) as usize;
        let mut weights = Vec::new();
        for row in 0..count {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            weights.push((row, (rng >> 32) % 127 + 1));
        }
        let mut prefix = source(&weights).prefix();
        let mut model = vec![0_u64; count];
        let mut credited = vec![0_u64; count];
        for total in 1..=128 {
            let mut winner = 0;
            for index in 1..count {
                if u128::from(weights[index].1) * (u128::from(model[winner]) + 1)
                    >= u128::from(weights[winner].1) * (u128::from(model[index]) + 1)
                {
                    winner = index;
                }
            }
            model[winner] += 1;
            for (destination, amount) in prefix.credit(1).expect("small credit cursor fits") {
                let Destination::Row(row) = destination else {
                    panic!("fixture uses rows")
                };
                credited[row] += amount;
            }
            assert_eq!(credited, model, "weights={weights:?}, total={total}");
        }
    }
}

#[test]
fn partial_pool_prefixes_keep_their_weights_when_more_credit_arrives() {
    let snapshot = source(&[(0, 100), (1, 100)]);
    let mut prefix = snapshot.prefix();
    let mut actual = [0_u64; 2];
    for _ in 0..100 {
        for (destination, amount) in prefix.credit(1).expect("partial pool cursor fits") {
            let Destination::Row(row) = destination else {
                panic!("fixture uses rows")
            };
            actual[row] += amount;
        }
    }
    assert_eq!(actual, [50, 50]);
    // Further grants and positive residue continue the original weighted sequence.
    for (destination, amount) in prefix.credit(103).expect("additional positive credit fits") {
        let Destination::Row(row) = destination else {
            panic!("fixture uses rows")
        };
        actual[row] += amount;
    }
    assert_eq!(actual, [101, 102]);
    assert_eq!(prefix.credit(0), Ok(Vec::new()));
    assert_eq!(prefix.source, snapshot);
    let mut prefix = source(&[(0, 2), (1, 3), (2, 5)]).prefix();
    prefix.credit(4).expect("four-point cursor fits");
    assert_eq!(prefix.credits, [1, 1, 2]);
    assert_eq!(prefix.credit(1), Ok(vec![(Destination::Row(2), 1)]));
    assert_eq!(prefix.credits, [1, 1, 3]);
}

#[test]
fn wide_prefixes_are_monotone_and_cursor_overflow_is_transactional() {
    let mut prefix = source(&[(0, u64::MAX - 2), (1, 1), (2, 1)]).prefix();
    let first = prefix.credit(u64::MAX - 1).expect("wide prefix fits");
    assert_eq!(
        first.iter().map(|(_, amount)| amount).sum::<u64>(),
        u64::MAX - 1
    );
    let second = prefix.credit(1).expect("last representable credit fits");
    assert_eq!(second.iter().map(|(_, amount)| amount).sum::<u64>(), 1);
    assert_eq!(prefix.credits, [u64::MAX - 2, 1, 1]);
    let before = prefix.credits.clone();
    assert_eq!(prefix.credit(1), Err(SourceFailure::Arithmetic));
    assert_eq!(prefix.credits, before);
    assert_eq!(prefix.credited_total, u64::MAX);
    assert_eq!(prefix.credit(0), Ok(Vec::new()));
    let mut single = source(&[(0, 1)]).prefix();
    assert_eq!(
        single.credit(u64::MAX),
        Ok(vec![(Destination::Row(0), u64::MAX)])
    );
}

#[test]
fn forged_historical_serials_do_not_release_live_transfers() {
    let mut state = state(0);
    let first = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_release(first), 1);
    let live = state.source_transfer_begin(7);
    let historical = Token {
        epoch: epoch(),
        kind: TokenKind::SourceTransfer,
        payload: 1,
    }
    .encode();
    assert_eq!(state.source_transfer_release(historical), 0);
    assert_eq!(state.source_transfer_add(live, unknown(0), 1), 1);
    assert_eq!(state.source_transfer_seal(live), 1);
    assert_eq!(state.source_count(live), 1);
    assert_eq!(state.source_transfer_release(live), 1);
    assert_eq!(state.source_transfer_release(live), 0);
}
