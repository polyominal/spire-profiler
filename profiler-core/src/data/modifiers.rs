//! Modifier credit is accounting policy, independent of game execution. Damage
//! additive terms use their truncated magnitude; block uses positive terms.
//! Multipliers use min(observed input, final result). Vulnerable's baseline and
//! nested increases receive separate credit. All decimal arithmetic is exact
//! until truncation toward zero; intermediate .NET rounding is not reproduced.

use std::cmp::Ordering;

use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

const MAX_TOP_LEVEL: usize = 64;
const MAX_NESTED: usize = 16;
pub(crate) const MAX_OBSERVATIONS: usize = MAX_TOP_LEVEL * (MAX_NESTED + 1);

#[repr(C)]
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ModifierObservation {
    pub source: u64,
    pub input_low: u64,
    pub input_high: u64,
    pub output_low: u64,
    pub output_high: u64,
    pub parent: i32,
    pub kind: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModifierCredit {
    pub source: u64,
    pub amount: i32,
}

const _: () = assert!(std::mem::size_of::<ModifierObservation>() == 48);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, source) == 0);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, input_low) == 8);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, input_high) == 16);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, output_low) == 24);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, output_high) == 32);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, parent) == 40);
const _: () = assert!(std::mem::offset_of!(ModifierObservation, kind) == 44);
const _: () = assert!(std::mem::size_of::<ModifierCredit>() == 16);
const _: () = assert!(std::mem::offset_of!(ModifierCredit, source) == 0);
const _: () = assert!(std::mem::offset_of!(ModifierCredit, amount) == 8);

#[derive(Debug, PartialEq)]
pub(crate) enum PolicyFailure {
    Packet,
    Capacity,
    Arithmetic,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct WeakObservation {
    pub total: i32,
    pub receiver_slot: i32,
    pub receiver_player: i32,
    pub weak: i32,
    pub debilitate: i32,
    pub krane_slots: u32,
}

impl WeakObservation {
    pub(crate) fn prevention(self) -> Result<i32, PolicyFailure> {
        if self.total < 0
            || !(0..=4).contains(&self.receiver_slot)
            || !(0..=1).contains(&self.receiver_player)
            || !(0..=1).contains(&self.weak)
            || !(0..=1).contains(&self.debilitate)
        {
            return Err(PolicyFailure::Packet);
        }
        if self.receiver_player == 0 || self.weak == 0 {
            return Ok(0);
        }
        let krane = self.receiver_slot < 4 && self.krane_slots & (1 << self.receiver_slot) != 0;
        // Weak retains 75% damage, or 60% with Krane. Debilitate doubles
        // the reduction. The counterfactual prevented amount is total*(1/m-1).
        let (numerator, denominator) = match (krane, self.debilitate != 0) {
            (false, false) => (i64::from(self.total), 3),
            (true, false) => (i64::from(self.total) * 2, 3),
            (false, true) => (i64::from(self.total), 1),
            (true, true) => (i64::from(self.total) * 4, 1),
        };
        let quotient = numerator / denominator;
        let twice_remainder = numerator % denominator * 2;
        let round_up =
            twice_remainder > denominator || (twice_remainder == denominator && quotient % 2 != 0);
        i32::try_from(quotient + i64::from(round_up)).map_err(|_| PolicyFailure::Arithmetic)
    }
}

#[derive(Clone, Debug)]
struct Decimal {
    units: i128,
    scale: u32,
}

impl Decimal {
    fn parse(low: u64, high: u64) -> Result<Self, PolicyFailure> {
        let flags = (high >> 32) as u32;
        let mut scale = (flags >> 16) & 0xff;
        if flags & 0x7f00ffff != 0 || scale > 28 {
            return Err(PolicyFailure::Packet);
        }
        let mut units = i128::from(low) | (i128::from(high as u32) << 64);
        if flags >> 31 != 0 {
            units = -units;
        }
        while scale > 0 && units % 10 == 0 {
            units /= 10;
            scale -= 1;
        }
        Ok(Self { units, scale })
    }

    fn compare(&self, other: &Self) -> Ordering {
        if self.scale == other.scale {
            return self.units.cmp(&other.units);
        }
        let scale = self.scale.max(other.scale);
        let left = BigInt::from(self.units) * BigInt::from(10_u8).pow(scale - self.scale);
        let right = BigInt::from(other.units) * BigInt::from(10_u8).pow(scale - other.scale);
        left.cmp(&right)
    }

    fn truncated(&self) -> Result<i32, PolicyFailure> {
        i32::try_from(self.units / 10_i128.pow(self.scale)).map_err(|_| PolicyFailure::Arithmetic)
    }

    fn increase_credit(&self, before: &Self, after: &Self) -> Result<i32, PolicyFailure> {
        let scale = before.scale.max(after.scale);
        let before = BigInt::from(before.units) * BigInt::from(10_u8).pow(scale - before.scale);
        let after = BigInt::from(after.units) * BigInt::from(10_u8).pow(scale - after.scale);
        let numerator = BigInt::from(self.units) * (after - before);
        let denominator = BigInt::from(10_u8).pow(self.scale + scale);
        i32::try_from(numerator / denominator).map_err(|_| PolicyFailure::Arithmetic)
    }
}

#[derive(PartialEq)]
enum Kind {
    Additive,
    Multiplicative,
    Vulnerable,
}
struct Observed {
    source: u64,
    input: Decimal,
    output: Decimal,
    kind: Kind,
    nested: Vec<Observed>,
}

pub(crate) struct ModifierBatch {
    initial: Decimal,
    result: Decimal,
    damage: bool,
    observations: Vec<Observed>,
}

impl ModifierBatch {
    pub(crate) fn parse(
        initial: (u64, u64),
        result: (u64, u64),
        damage: i32,
        raw: &[ModifierObservation],
    ) -> Result<Self, PolicyFailure> {
        if raw.len() > MAX_OBSERVATIONS {
            return Err(PolicyFailure::Capacity);
        }
        let damage = match damage {
            0 => false,
            1 => true,
            _ => return Err(PolicyFailure::Packet),
        };
        let mut observations: Vec<Observed> = Vec::new();
        let mut parent_index = None;
        for (index, raw) in raw.iter().enumerate() {
            let observed = Observed {
                source: raw.source,
                input: Decimal::parse(raw.input_low, raw.input_high)?,
                output: Decimal::parse(raw.output_low, raw.output_high)?,
                kind: match raw.kind {
                    0 => Kind::Additive,
                    1 | 3 => Kind::Multiplicative,
                    2 => Kind::Vulnerable,
                    _ => return Err(PolicyFailure::Packet),
                },
                nested: Vec::new(),
            };
            if raw.kind == 3 {
                if parent_index != Some(raw.parent) {
                    return Err(PolicyFailure::Packet);
                }
                let Some(parent) = observations.last_mut() else {
                    return Err(PolicyFailure::Packet);
                };
                if parent.kind != Kind::Vulnerable || parent.nested.len() == MAX_NESTED {
                    return Err(PolicyFailure::Packet);
                }
                parent.nested.push(observed);
            } else {
                if raw.parent != -1 {
                    return Err(PolicyFailure::Packet);
                }
                if observations.len() == MAX_TOP_LEVEL {
                    return Err(PolicyFailure::Capacity);
                }
                parent_index = Some(index as i32);
                observations.push(observed);
            }
        }
        let one = Decimal { units: 1, scale: 0 };
        for observation in &observations {
            if let Some(first) = observation.nested.first() {
                if first.input.compare(&one) == Ordering::Less {
                    return Err(PolicyFailure::Packet);
                }
                let mut previous = &first.input;
                for part in &observation.nested {
                    if part.input.compare(previous) != Ordering::Equal {
                        return Err(PolicyFailure::Packet);
                    }
                    previous = &part.output;
                }
                if previous.compare(&observation.output) != Ordering::Equal {
                    return Err(PolicyFailure::Packet);
                }
            }
        }
        Ok(Self {
            initial: Decimal::parse(initial.0, initial.1)?,
            result: Decimal::parse(result.0, result.1)?,
            damage,
            observations,
        })
    }

    pub(crate) fn credits(&self) -> Result<Vec<ModifierCredit>, PolicyFailure> {
        let mut credits = Vec::new();
        if self.result.compare(&self.initial) != Ordering::Greater {
            return Ok(credits);
        }
        let one = Decimal { units: 1, scale: 0 };
        for observation in &self.observations {
            if observation.kind == Kind::Additive {
                let amount = observation.output.truncated()?;
                let amount = if self.damage {
                    amount.checked_abs().ok_or(PolicyFailure::Arithmetic)?
                } else {
                    amount
                };
                if amount > 0 {
                    credits.push(ModifierCredit {
                        source: observation.source,
                        amount,
                    });
                }
                continue;
            }
            if observation.output.compare(&one) != Ordering::Greater {
                continue;
            }
            let basis = if observation.input.compare(&self.result) == Ordering::Less {
                &observation.input
            } else {
                &self.result
            };
            let multiplier = observation
                .nested
                .first()
                .map_or(&observation.output, |first| &first.input);
            let amount = basis.increase_credit(&one, multiplier)?;
            if amount > 0 {
                credits.push(ModifierCredit {
                    source: observation.source,
                    amount,
                });
            }
            for part in &observation.nested {
                let amount = basis.increase_credit(&part.input, &part.output)?;
                if amount > 0 {
                    credits.push(ModifierCredit {
                        source: part.source,
                        amount,
                    });
                }
            }
        }
        Ok(credits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_prevention_matches_counterfactual_damage_for_each_observed_power_set() {
        for total in 0..=10_000 {
            for krane in [false, true] {
                for debilitate in [false, true] {
                    let reduction = if krane { 0.4_f64 } else { 0.25 };
                    let multiplier = 1.0 - reduction * if debilitate { 2.0 } else { 1.0 };
                    let expected =
                        (f64::from(total) / multiplier - f64::from(total)).round() as i32;
                    let observed = WeakObservation {
                        total,
                        receiver_slot: 2,
                        receiver_player: 1,
                        weak: 1,
                        debilitate: i32::from(debilitate),
                        krane_slots: if krane { 4 } else { 0 },
                    };
                    assert_eq!(observed.prevention(), Ok(expected));
                    assert_eq!(
                        WeakObservation {
                            weak: 0,
                            ..observed
                        }
                        .prevention(),
                        Ok(0)
                    );
                    assert_eq!(
                        WeakObservation {
                            receiver_player: 0,
                            ..observed
                        }
                        .prevention(),
                        Ok(0)
                    );
                }
            }
        }
        let observed = WeakObservation {
            total: i32::MAX,
            receiver_slot: 0,
            receiver_player: 1,
            weak: 1,
            debilitate: 1,
            krane_slots: 1,
        };
        assert_eq!(observed.prevention(), Err(PolicyFailure::Arithmetic));
        assert_eq!(
            WeakObservation {
                total: -1,
                ..observed
            }
            .prevention(),
            Err(PolicyFailure::Packet)
        );
        assert_eq!(
            WeakObservation {
                receiver_player: 2,
                ..observed
            }
            .prevention(),
            Err(PolicyFailure::Packet)
        );
    }

    fn bits(units: i128, scale: u32) -> (u64, u64) {
        let magnitude = units.unsigned_abs();
        assert!(magnitude < 1_u128 << 96);
        let flags = (scale << 16) | if units < 0 { 1 << 31 } else { 0 };
        (
            magnitude as u64,
            (magnitude >> 64) as u64 | (u64::from(flags) << 32),
        )
    }

    fn observation(
        source: u64,
        input: (i128, u32),
        output: (i128, u32),
        kind: i32,
        parent: i32,
    ) -> ModifierObservation {
        let input = bits(input.0, input.1);
        let output = bits(output.0, output.1);
        ModifierObservation {
            source,
            input_low: input.0,
            input_high: input.1,
            output_low: output.0,
            output_high: output.1,
            kind,
            parent,
        }
    }

    #[test]
    fn small_decimal_batches_match_an_independent_integer_ratio_model() {
        let mut seed = 0xd3c1_a120_5eed_u64;
        for damage in [0, 1] {
            for _ in 0..512 {
                let mut observations = Vec::new();
                let mut expected = Vec::new();
                for source in 0..8 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let input = ((seed >> 16) % 4001) as i128 - 100;
                    let input_scale = ((seed >> 12) % 3) as u32;
                    let output = ((seed >> 32) % 3001) as i128 - 1500;
                    let output_scale = ((seed >> 8) % 3) as u32;
                    let kind = (seed % 2) as i32;
                    observations.push(observation(
                        source,
                        (input, input_scale),
                        (output, output_scale),
                        kind,
                        -1,
                    ));
                    let input_denominator = 10_i128.pow(input_scale);
                    let output_denominator = 10_i128.pow(output_scale);
                    let amount = if kind == 0 {
                        let value = output / output_denominator;
                        if damage == 1 { value.abs() } else { value }
                    } else if output <= output_denominator {
                        0
                    } else {
                        let input = input.min(17 * input_denominator);
                        input * (output - output_denominator)
                            / (input_denominator * output_denominator)
                    };
                    if amount > 0 {
                        expected.push(ModifierCredit {
                            source,
                            amount: amount as i32,
                        });
                    }
                }
                let batch = ModifierBatch::parse(bits(3, 0), bits(17, 0), damage, &observations)
                    .expect("small fixture decimals are valid");
                assert_eq!(batch.credits(), Ok(expected));
            }
        }
    }

    #[test]
    fn nested_vulnerable_credits_baseline_and_each_observed_increment() {
        let raw = [
            observation(11, (10, 0), (20, 1), 2, -1),
            observation(12, (150, 2), (175, 2), 3, 0),
            observation(13, (1750, 3), (20, 1), 3, 0),
        ];
        let batch = ModifierBatch::parse(bits(10, 0), bits(20, 0), 1, &raw)
            .expect("observations form a continuous chain");
        assert_eq!(
            batch.credits(),
            Ok(vec![
                ModifierCredit {
                    source: 11,
                    amount: 5
                },
                ModifierCredit {
                    source: 12,
                    amount: 2
                },
                ModifierCredit {
                    source: 13,
                    amount: 2
                }
            ])
        );
        let clipped = ModifierBatch::parse(bits(0, 0), bits(4, 0), 1, &raw)
            .expect("clipped result remains a valid observation");
        assert_eq!(
            clipped.credits(),
            Ok(vec![
                ModifierCredit {
                    source: 11,
                    amount: 2
                },
                ModifierCredit {
                    source: 12,
                    amount: 1
                },
                ModifierCredit {
                    source: 13,
                    amount: 1
                }
            ])
        );
    }

    #[test]
    fn full_precision_products_truncate_only_after_exact_arithmetic() {
        let maximum = (1_i128 << 96) - 1;
        let unit = 10_i128.pow(28);
        let raw = [observation(1, (maximum, 0), (unit + 1, 28), 1, -1)];
        let batch = ModifierBatch::parse(bits(0, 0), bits(maximum, 0), 1, &raw)
            .expect("maximum decimal coefficients fit");
        assert_eq!(
            batch.credits(),
            Ok(vec![ModifierCredit {
                source: 1,
                amount: 7
            }])
        );
        let raw = [observation(2, (unit - 1, 28), (2 * unit + 1, 28), 1, -1)];
        let batch = ModifierBatch::parse(bits(0, 0), bits(2, 0), 1, &raw)
            .expect("full precision decimals fit");
        assert_eq!(
            batch.credits(),
            Ok(vec![]),
            "(1-epsilon)*(1+epsilon) remains below one"
        );
        let raw = [observation(3, (1, 0), (maximum, 0), 0, -1)];
        let batch = ModifierBatch::parse(bits(0, 0), bits(maximum, 0), 1, &raw)
            .expect("wire value is valid even when credit is too large");
        assert_eq!(batch.credits(), Err(PolicyFailure::Arithmetic));
    }

    #[test]
    fn malformed_flags_parents_chains_and_limits_reject_before_credit() {
        let top = observation(1, (10, 0), (2, 0), 2, -1);
        let nested = observation(2, (15, 1), (2, 0), 3, 0);
        for raw in [
            vec![nested],
            vec![
                top,
                ModifierObservation {
                    parent: 1,
                    ..nested
                },
            ],
            vec![ModifierObservation { kind: 1, ..top }, nested],
            vec![top, observation(2, (15, 1), (18, 1), 3, 0)],
            vec![ModifierObservation {
                output_high: top.output_high | (1_u64 << 32),
                ..top
            }],
            vec![ModifierObservation {
                output_high: 29_u64 << 48,
                ..top
            }],
            vec![top; MAX_TOP_LEVEL + 1],
        ] {
            assert!(ModifierBatch::parse(bits(0, 0), bits(20, 0), 1, &raw).is_err());
        }
    }
}
