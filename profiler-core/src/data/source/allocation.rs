use super::{
    DamageSegment, Destination, ProducerSegment, RootBudgets, SourceFailure, SourceSnapshot, caps,
};

#[derive(Clone, Debug)]
pub(super) struct ModifierContribution {
    pub(super) source: SourceSnapshot,
    pub(super) amount: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AllocatedCredit {
    pub(super) destination: Destination,
    pub(super) segment: DamageSegment,
    pub(super) damage: u64,
    pub(super) blocked: u64,
}

impl AllocatedCredit {
    fn append(
        credits: &mut Vec<Self>,
        segment: DamageSegment,
        portions: Vec<(Destination, u64)>,
    ) -> Result<(), SourceFailure> {
        for (destination, damage) in portions {
            if let Some(credit) = credits
                .iter_mut()
                .find(|credit| credit.destination == destination && credit.segment == segment)
            {
                credit.damage = credit
                    .damage
                    .checked_add(damage)
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                credits.push(Self {
                    destination,
                    segment,
                    damage,
                    blocked: 0,
                });
            }
        }
        Ok(())
    }
}

pub(super) struct DamageAllocation;

impl DamageAllocation {
    fn sum(weights: &[u64]) -> Result<u64, SourceFailure> {
        weights
            .iter()
            .try_fold(0_u64, |total, weight| total.checked_add(*weight))
            .ok_or(SourceFailure::Arithmetic)
    }

    pub(super) fn build(
        producer: &SourceSnapshot,
        segment: ProducerSegment,
        modifiers: &[ModifierContribution],
        results: &[(u64, u64)],
    ) -> Result<Vec<Vec<AllocatedCredit>>, SourceFailure> {
        if results.len() > caps::DAMAGE_RESULTS || modifiers.len() > caps::DAMAGE_MODIFIERS {
            return Err(SourceFailure::Capacity);
        }
        let mut destinations = Vec::new();
        for source in std::iter::once(producer).chain(modifiers.iter().map(|event| &event.source)) {
            if source.combat_seq() != producer.combat_seq() {
                return Err(SourceFailure::Epoch);
            }
            for share in source.shares() {
                if !destinations.contains(&share.destination()) {
                    if destinations.len() == caps::DAMAGE_DESTINATIONS {
                        return Err(SourceFailure::Capacity);
                    }
                    destinations.push(share.destination());
                }
            }
        }
        if results.iter().any(|(total, blocked)| blocked > total) {
            return Err(SourceFailure::Packet);
        }
        let totals: Vec<_> = results.iter().map(|(total, _)| *total).collect();
        let contributions: Vec<_> = modifiers.iter().map(|event| event.amount).collect();
        let total = Self::sum(&totals)?;
        let modifier_total = Self::sum(&contributions)?.min(total);
        let budgets = RootBudgets::proportional(modifier_total, &contributions)?;
        let mut roots: Vec<_> = modifiers
            .iter()
            .zip(budgets)
            .map(|(event, amount)| event.source.budgets(amount))
            .collect();
        let mut producer_roots = producer.budgets(total - modifier_total);
        let result_budgets = RootBudgets::proportional(modifier_total, &totals)?;
        let mut allocated = Vec::with_capacity(results.len());
        for ((total, blocked), modifier_amount) in results.iter().zip(result_budgets) {
            let remaining: Vec<_> = roots.iter().map(|roots| roots.remaining()).collect();
            let portions = RootBudgets::proportional(modifier_amount, &remaining)?;
            let mut credits = Vec::new();
            for (roots, amount) in roots.iter_mut().zip(portions) {
                AllocatedCredit::append(
                    &mut credits,
                    DamageSegment::Modifier,
                    roots.take(amount)?,
                )?;
            }
            AllocatedCredit::append(
                &mut credits,
                segment.into(),
                producer_roots.take(total - modifier_amount)?,
            )?;
            let weights: Vec<_> = credits.iter().map(|credit| credit.damage).collect();
            for (credit, blocked) in credits
                .iter_mut()
                .zip(RootBudgets::proportional(*blocked, &weights)?)
            {
                credit.blocked = blocked;
            }
            allocated.push(credits);
        }
        Ok(allocated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::source::CombatEpoch;

    fn source(weights: &[(usize, u64)]) -> SourceSnapshot {
        SourceSnapshot::normalized(
            CombatEpoch::from_wire(7).expect("fixture epoch fits the nonzero u32 domain"),
            caps::COMBAT_CARDS,
            weights
                .iter()
                .map(|(row, weight)| (Destination::Row(*row), u128::from(*weight)))
                .collect(),
        )
        .expect("fixture roots and weights fit source bounds")
    }

    fn credit(
        destination: Destination,
        segment: DamageSegment,
        damage: u64,
        blocked: u64,
    ) -> AllocatedCredit {
        AllocatedCredit {
            destination,
            segment,
            damage,
            blocked,
        }
    }

    struct Units<T>(Vec<T>);

    impl<T: Copy> Units<T> {
        fn indices(amount: u64, length: usize) -> Vec<usize> {
            // Each credit unit lands at the first weighted-strip boundary it reaches.
            (1..=amount as usize)
                .map(|unit| {
                    (0..length)
                        .find(|index| (index + 1) * amount as usize >= unit * length)
                        .expect("positive model budgets have a nonempty weighted strip")
                })
                .collect()
        }

        fn sample(amount: u64, weights: impl IntoIterator<Item = (T, u64)>) -> Self {
            let strip: Vec<_> = weights
                .into_iter()
                .flat_map(|(key, weight)| std::iter::repeat_n(key, weight as usize))
                .collect();
            Self(
                Self::indices(amount, strip.len())
                    .into_iter()
                    .map(|index| strip[index])
                    .collect(),
            )
        }

        fn take(&mut self, amount: u64) -> Vec<T> {
            assert!(amount <= self.0.len() as u64, "budgets never go negative");
            let positions = Self::indices(amount, self.0.len());
            let drawn = positions.iter().map(|index| self.0[*index]).collect();
            self.0 = self
                .0
                .iter()
                .enumerate()
                .filter_map(|(index, key)| (!positions.contains(&index)).then_some(*key))
                .collect();
            drawn
        }
    }

    impl Units<Destination> {
        fn source(amount: u64, source: &SourceSnapshot) -> Self {
            Self::sample(
                amount,
                source
                    .shares()
                    .iter()
                    .map(|share| (share.destination(), share.weight())),
            )
        }
    }

    impl Units<usize> {
        fn counts(amount: u64, weights: impl Iterator<Item = u64>) -> Self {
            Self::sample(amount, weights.enumerate())
        }
    }

    struct UnitModel;

    impl UnitModel {
        fn build(
            producer: &SourceSnapshot,
            segment: ProducerSegment,
            modifiers: &[ModifierContribution],
            results: &[(u64, u64)],
        ) -> Vec<Vec<AllocatedCredit>> {
            let total: u64 = results.iter().map(|(total, _)| total).sum();
            let captured: u64 = modifiers.iter().map(|event| event.amount).sum();
            let bound = captured.min(total);
            let mut events = Units::counts(bound, modifiers.iter().map(|event| event.amount));
            let result_units = Units::counts(bound, results.iter().map(|(total, _)| *total));
            let mut roots: Vec<_> = modifiers
                .iter()
                .enumerate()
                .map(|(i, event)| {
                    Units::source(
                        events.0.iter().filter(|event| **event == i).count() as u64,
                        &event.source,
                    )
                })
                .collect();
            let mut producer = Units::source(total - bound, producer);
            let mut output = Vec::new();
            for (i, (total, blocked)) in results.iter().enumerate() {
                let modifier_amount = result_units.0.iter().filter(|r| **r == i).count() as u64;
                let selected = events.take(modifier_amount);
                let mut units = Vec::new();
                for (j, roots) in roots.iter_mut().enumerate() {
                    let count = selected.iter().filter(|event| **event == j).count() as u64;
                    units.extend(
                        roots
                            .take(count)
                            .into_iter()
                            .map(|root| (root, DamageSegment::Modifier)),
                    );
                }
                units.extend(
                    producer
                        .take(total - modifier_amount)
                        .into_iter()
                        .map(|r| (r, segment.into())),
                );
                let mut credits: Vec<AllocatedCredit> = Vec::new();
                for (destination, segment) in units {
                    match credits
                        .iter_mut()
                        .find(|c| c.destination == destination && c.segment == segment)
                    {
                        Some(credit) => credit.damage += 1,
                        None => credits.push(credit(destination, segment, 1, 0)),
                    }
                }
                let blocked_units =
                    Units::counts(*blocked, credits.iter().map(|credit| credit.damage));
                for index in blocked_units.0 {
                    credits[index].blocked += 1;
                }
                output.push(credits);
            }
            assert!(events.0.is_empty() && producer.0.is_empty());
            assert!(roots.iter().all(|root| root.0.is_empty()));
            output
        }
    }

    #[test]
    fn redirected_groups_match_independent_unit_model() {
        let mut seed = 0x5eed_dada_9321_u64;
        let mut random = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            seed >> 32
        };
        for _ in 0..512 {
            let mut sources = Vec::new();
            for _ in 0..4 {
                let offset = random() as usize % 6;
                let weights: Vec<_> = (0..4)
                    .map(|row| ((row + offset) % 6, random() % 4 + 1))
                    .collect();
                sources.push(source(&weights));
            }
            let modifiers: Vec<_> = sources[1..]
                .iter()
                .map(|source| ModifierContribution {
                    source: source.clone(),
                    amount: random() % 12,
                })
                .collect();
            let results: Vec<_> = (0..2)
                .map(|_| {
                    let total = random() % 16;
                    (total, random() % (total + 1))
                })
                .collect();
            let segment = if random() % 2 == 0 {
                ProducerSegment::Direct
            } else {
                ProducerSegment::Attributed
            };
            let actual = DamageAllocation::build(&sources[0], segment, &modifiers, &results)
                .expect("bounded unit-model fixtures are valid groups");
            assert_eq!(
                actual,
                UnitModel::build(&sources[0], segment, &modifiers, &results)
            );
            for (credits, (total, blocked)) in actual.iter().zip(&results) {
                assert_eq!(
                    credits.iter().map(|credit| credit.damage).sum::<u64>(),
                    *total
                );
                assert_eq!(
                    credits.iter().map(|credit| credit.blocked).sum::<u64>(),
                    *blocked
                );
                assert!(credits.iter().all(|credit| credit.blocked <= credit.damage));
            }
        }
    }

    #[test]
    fn redirected_one_unit_shares_exhaust_each_supplier_once() {
        let producer = source(&[(0, 1), (1, 1)]);
        let modifiers = [ModifierContribution {
            source: source(&[(2, 1), (3, 1)]),
            amount: 2,
        }];
        let results = [(2, 1), (2, 2)];
        let actual =
            DamageAllocation::build(&producer, ProducerSegment::Direct, &modifiers, &results)
                .expect("four actual points exhaust two producer and two modifier roots");
        let expected = vec![
            vec![
                credit(Destination::Row(3), DamageSegment::Modifier, 1, 0),
                credit(Destination::Row(1), DamageSegment::Direct, 1, 1),
            ],
            vec![
                credit(Destination::Row(2), DamageSegment::Modifier, 1, 1),
                credit(Destination::Row(0), DamageSegment::Direct, 1, 1),
            ],
        ];
        assert_eq!(actual, expected);
        assert_eq!(
            actual,
            UnitModel::build(&producer, ProducerSegment::Direct, &modifiers, &results)
        );
    }

    #[test]
    fn overlapping_modifier_roots_coalesce_before_blocked_split() {
        let producer = source(&[(1, 1)]);
        let modifiers = [
            ModifierContribution {
                source: source(&[(1, 1), (2, 1)]),
                amount: 2,
            },
            ModifierContribution {
                source: source(&[(1, 1)]),
                amount: 1,
            },
        ];
        let results = [(4, 2), (0, 0)];
        let actual =
            DamageAllocation::build(&producer, ProducerSegment::Direct, &modifiers, &results)
                .expect("overlapping roots retain separate damage segments");
        assert_eq!(
            actual,
            vec![
                vec![
                    credit(Destination::Row(1), DamageSegment::Modifier, 2, 1),
                    credit(Destination::Row(2), DamageSegment::Modifier, 1, 0),
                    credit(Destination::Row(1), DamageSegment::Direct, 1, 1),
                ],
                vec![],
            ]
        );
        assert_eq!(
            actual,
            UnitModel::build(&producer, ProducerSegment::Direct, &modifiers, &results)
        );
    }

    #[test]
    fn lethal_and_unavailable_sources_preserve_actual_totals() {
        let unknown = SourceSnapshot::unknown(
            CombatEpoch::from_wire(7).expect("fixture epoch fits the nonzero u32 domain"),
        );
        let modifiers = [ModifierContribution {
            source: source(&[(0, 1), (1, 1)]),
            amount: 100,
        }];
        let lethal = DamageAllocation::build(
            &unknown,
            ProducerSegment::Attributed,
            &modifiers,
            &[(1, 1), (0, 0)],
        )
        .expect("modifier claims are capped by the one actual lethal point");
        assert_eq!(
            lethal,
            vec![
                vec![credit(Destination::Row(1), DamageSegment::Modifier, 1, 1)],
                vec![]
            ]
        );
        assert_eq!(
            lethal,
            UnitModel::build(
                &unknown,
                ProducerSegment::Attributed,
                &modifiers,
                &[(1, 1), (0, 0)]
            )
        );
        for segment in [ProducerSegment::Direct, ProducerSegment::Attributed] {
            let results = [(0, 0), (3, 3)];
            let actual = DamageAllocation::build(&unknown, segment, &[], &results)
                .expect("explicit unknown preserves the known producer segment");
            assert_eq!(actual, UnitModel::build(&unknown, segment, &[], &results));
            assert_eq!(
                actual[1][0].destination,
                Destination::Unknown(super::super::TEAM_SLOT)
            );
            assert_eq!(actual[1][0].segment, segment.into());
            assert_eq!((actual[1][0].damage, actual[1][0].blocked), (3, 3));
        }
    }

    #[test]
    fn group_limits_count_distinct_roots_and_actual_modifier_events() {
        let producer = source(&[(0, 1)]);
        let mut modifiers = vec![
            ModifierContribution {
                source: producer.clone(),
                amount: 1
            };
            caps::DAMAGE_MODIFIERS
        ];
        assert!(
            DamageAllocation::build(
                &producer,
                ProducerSegment::Direct,
                &modifiers,
                &[(2, 1), (0, 0)]
            )
            .is_ok()
        );
        modifiers.push(modifiers[0].clone());
        assert_eq!(
            DamageAllocation::build(&producer, ProducerSegment::Direct, &modifiers, &[]),
            Err(SourceFailure::Capacity)
        );
        assert_eq!(
            DamageAllocation::build(&producer, ProducerSegment::Direct, &[], &[(0, 0); 3]),
            Err(SourceFailure::Capacity)
        );
        let weights: Vec<_> = (0..caps::DAMAGE_DESTINATIONS).map(|row| (row, 1)).collect();
        let all_roots = source(&weights);
        assert!(
            DamageAllocation::build(&all_roots, ProducerSegment::Direct, &[], &[(1, 0)]).is_ok()
        );
        let extra = [ModifierContribution {
            source: source(&[(caps::DAMAGE_DESTINATIONS, 1)]),
            amount: 0,
        }];
        assert_eq!(
            DamageAllocation::build(&all_roots, ProducerSegment::Direct, &extra, &[(0, 0)]),
            Err(SourceFailure::Capacity)
        );
    }

    #[test]
    fn malformed_groups_reject_without_partial_credits() {
        let producer = source(&[(0, 1)]);
        assert_eq!(
            DamageAllocation::build(&producer, ProducerSegment::Direct, &[], &[(1, 2)]),
            Err(SourceFailure::Packet)
        );
        assert_eq!(
            DamageAllocation::build(
                &producer,
                ProducerSegment::Direct,
                &[],
                &[(u64::MAX, 0), (1, 0)]
            ),
            Err(SourceFailure::Arithmetic)
        );
        let modifiers = [
            ModifierContribution {
                source: producer.clone(),
                amount: u64::MAX,
            },
            ModifierContribution {
                source: producer.clone(),
                amount: 1,
            },
        ];
        assert_eq!(
            DamageAllocation::build(&producer, ProducerSegment::Direct, &modifiers, &[(0, 0)]),
            Err(SourceFailure::Arithmetic)
        );
        let stale = [ModifierContribution {
            source: SourceSnapshot::unknown(
                CombatEpoch::from_wire(8).expect("fixture epoch is nonzero"),
            ),
            amount: 0,
        }];
        assert_eq!(
            DamageAllocation::build(&producer, ProducerSegment::Direct, &stale, &[]),
            Err(SourceFailure::Epoch)
        );
    }

    #[test]
    fn maximum_representable_totals_keep_root_budgets_nonnegative() {
        let producer = source(&[(0, u64::MAX - 1), (1, 1)]);
        let actual = DamageAllocation::build(
            &producer,
            ProducerSegment::Attributed,
            &[],
            &[(u64::MAX - 1, u64::MAX - 1), (1, 0)],
        )
        .expect("widened products preserve a representable u64 total");
        assert_eq!(
            actual,
            vec![
                vec![
                    credit(
                        Destination::Row(0),
                        DamageSegment::Attributed,
                        u64::MAX - 2,
                        u64::MAX - 2
                    ),
                    credit(Destination::Row(1), DamageSegment::Attributed, 1, 1),
                ],
                vec![credit(Destination::Row(0), DamageSegment::Attributed, 1, 0)],
            ]
        );
    }
}
