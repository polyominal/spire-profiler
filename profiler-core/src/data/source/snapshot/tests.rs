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
    let mut state = State {
        current: Some(Combat {
            seq: epoch().0.get(),
            cards: (0..rows).map(|_| CardStat::default()).collect(),
            ..Combat::default()
        })
        .into(),
        ..State::default()
    };
    state
        .reserve_lifecycle()
        .expect("fixture lifetime storage reservation succeeds");
    state
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
    let mut saved = SourceSnapshot::try_new().expect("fixture snapshot reservation succeeds");
    let initial_epoch = state.source_epoch(7).expect("fixture combat is active");
    assert_eq!(
        state
            .source_transfers
            .snapshot_into(initial_epoch, 0, &mut saved),
        Ok(())
    );
    assert!(saved.is_unknown());
    let transfer = state.source_transfer_begin(7);
    assert_eq!(state.source_transfer_add(transfer, unknown(1), 1), 1);
    assert_eq!(state.source_transfer_add(transfer, unknown(3), 2), 1);
    assert_eq!(state.source_transfer_seal(transfer), 1);
    state
        .source_transfers
        .snapshot_into(initial_epoch, transfer, &mut saved)
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
        state
            .source_transfers
            .snapshot_into(initial_epoch, transfer, &mut saved),
        Err(SourceFailure::Token)
    );
    state.current.as_mut().expect("fixture combat exists").seq = 8;
    assert!(matches!(state.source_epoch(7), Err(SourceFailure::Epoch)));
    let next_epoch = state
        .source_epoch(8)
        .expect("replacement fixture combat is active");
    assert_eq!(
        state
            .source_transfers
            .snapshot_into(next_epoch, transfer, &mut saved),
        Err(SourceFailure::Epoch)
    );
    let forged = transfer ^ (7_u64 << 32) ^ (8_u64 << 32);
    assert_eq!(
        state
            .source_transfers
            .snapshot_into(next_epoch, forged, &mut saved),
        Err(SourceFailure::Token)
    );
    assert_eq!(saved.combat_seq(), 7);
    assert_eq!(
        saved
            .shares
            .iter()
            .map(|share| share.weight)
            .collect::<Vec<_>>(),
        [1, 2]
    );
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
            for (destination, amount) in prefix.credit_iter(1).expect("small credit cursor fits") {
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

#[test]
fn retained_normalization_publishes_only_checked_snapshots_and_recovers() {
    let mut snapshot = SourceSnapshot::try_new().expect("fixture snapshot reservation succeeds");
    let mut scratch = SnapshotScratch::try_new().expect("fixture scratch reservation succeeds");
    let mut copied = SourceSnapshot::try_new().expect("fixture copy reservation succeeds");
    let allocation = snapshot.shares.as_ptr();
    let scratch_allocation = scratch.merged.as_ptr();
    let copy_allocation = copied.shares.as_ptr();
    let full: Vec<_> = (0..caps::SOURCE_DESTINATIONS)
        .map(|row| (Destination::Row(row), (row + 1) as u128))
        .collect();
    let mut excess = full.clone();
    excess.push((Destination::Unknown(TEAM_SLOT), 1));
    for _ in 0..3 {
        snapshot.set_unknown(epoch());
        assert!(snapshot.is_unknown());
        snapshot
            .normalize_into(epoch(), caps::COMBAT_CARDS, &full, &mut scratch)
            .expect("all bounded roots and their total fit");
        copied.clone_from(&snapshot);
        for (entries, failure) in [
            (excess.as_slice(), SourceFailure::Capacity),
            (&[(Destination::Row(0), 0)], SourceFailure::Packet),
            (
                &[(Destination::Row(caps::COMBAT_CARDS), 1)],
                SourceFailure::Token,
            ),
            (
                &[(Destination::Unknown(TEAM_SLOT + 1), 1)],
                SourceFailure::Packet,
            ),
            (
                &[(Destination::Row(0), u128::MAX), (Destination::Row(0), 1)],
                SourceFailure::Arithmetic,
            ),
            (
                &[
                    (Destination::Row(0), u128::from(u64::MAX)),
                    (Destination::Row(1), 1),
                ],
                SourceFailure::Arithmetic,
            ),
        ] {
            assert_eq!(
                snapshot.normalize_into(epoch(), caps::COMBAT_CARDS, entries, &mut scratch),
                Err(failure)
            );
            assert_eq!(snapshot, copied);
        }
        snapshot.set_single(
            CombatEpoch::from_wire(8).expect("next fixture epoch fits"),
            Destination::Row(3),
        );
        assert_eq!(copied.combat_seq(), epoch().0.get());
        assert_eq!(copied.shares().len(), caps::SOURCE_DESTINATIONS);
        assert_eq!(snapshot.shares.as_ptr(), allocation);
        assert_eq!(scratch.merged.as_ptr(), scratch_allocation);
        assert_eq!(copied.shares.as_ptr(), copy_allocation);
    }
}

#[test]
fn retained_mixtures_clear_scratch_after_failure_and_zero_grants() {
    let mut snapshot = SourceSnapshot::try_new().expect("fixture snapshot reservation succeeds");
    let mut scratch = SnapshotScratch::try_new().expect("fixture scratch reservation succeeds");
    let mut diagnostics = SourceDiagnostics::default();
    let a = source(&[(0, 1), (1, 1)]);
    let b = source(&[(0, 1), (2, 2)]);
    let allocation = snapshot.shares.as_ptr();
    for _ in 0..3 {
        snapshot.mixture_into(
            epoch(),
            3,
            [(1, &a), (1, &b)],
            &mut scratch,
            &mut diagnostics,
        );
        assert_eq!(
            snapshot
                .shares
                .iter()
                .map(|share| share.weight)
                .collect::<Vec<_>>(),
            [5, 3, 4]
        );
        snapshot.mixture_into(epoch(), 3, [(0, &a)], &mut scratch, &mut diagnostics);
        assert!(snapshot.is_unknown());
        snapshot.mixture_into(
            epoch(),
            3,
            std::iter::repeat_n((1, &a), caps::POWER_GRANTS_TOTAL + 1),
            &mut scratch,
            &mut diagnostics,
        );
        assert!(snapshot.is_unknown());
        snapshot.mixture_into(epoch(), 3, [(1, &b)], &mut scratch, &mut diagnostics);
        assert_eq!(snapshot, b);
        assert_eq!(snapshot.shares.as_ptr(), allocation);
    }
}

#[test]
fn retained_prefixes_copy_independent_cursors_and_reuse_after_exhaustion() {
    let mut prefix = SourcePrefix::try_new().expect("fixture prefix reservation succeeds");
    let mut copied = SourcePrefix::try_new().expect("fixture prefix copy reservation succeeds");
    let mut source = source(&[(0, 2), (1, 3), (2, 5)]);
    let shares = prefix.source.shares.as_ptr();
    let cursors = [prefix.credits.as_ptr(), prefix.candidate.as_ptr()];
    for _ in 0..3 {
        prefix.reset_from(&source);
        assert_eq!(
            prefix
                .credit_iter(4)
                .expect("small cursor fits")
                .map(|(_, amount)| amount)
                .sum::<u64>(),
            4
        );
        copied.clone_from(&prefix);
        assert_eq!(
            prefix
                .credit_iter(1)
                .expect("next point fits")
                .collect::<Vec<_>>(),
            [(Destination::Row(2), 1)]
        );
        assert_eq!(
            copied
                .credit_iter(1)
                .expect("copied cursor is independent")
                .collect::<Vec<_>>(),
            [(Destination::Row(2), 1)]
        );
        let remainder = u64::MAX - 5;
        assert_eq!(
            prefix
                .credit_iter(remainder)
                .expect("last cursor total fits")
                .map(|(_, amount)| amount)
                .sum::<u64>(),
            remainder
        );
        assert!(matches!(
            prefix.credit_iter(1),
            Err(SourceFailure::Arithmetic)
        ));
        assert_eq!(
            prefix
                .credit_iter(0)
                .expect("zero credit fits after overflow")
                .count(),
            0
        );
        assert_eq!(prefix.source.shares.as_ptr(), shares);
        assert!(cursors.contains(&prefix.credits.as_ptr()));
        assert!(cursors.contains(&prefix.candidate.as_ptr()));
    }
    source.set_unknown(CombatEpoch::from_wire(8).expect("next fixture epoch fits"));
    assert_eq!(prefix.source.combat_seq(), epoch().0.get());
    assert_eq!(prefix.source.shares().len(), 3);
}
