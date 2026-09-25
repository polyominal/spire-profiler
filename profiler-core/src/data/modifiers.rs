//! Credit arithmetic follows .NET decimal: each subtraction and product rounds
//! to a 96-bit coefficient and at most 28 decimal places before integer
//! truncation. The host supplies callback results in their original order.

use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq)]
pub(crate) enum PolicyFailure {
    Packet,
    Arithmetic,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct ModifierObservation {
    pub basis_low: u64,
    pub basis_high: u64,
    pub value_low: u64,
    pub value_high: u64,
    pub limit_low: u64,
    pub limit_high: u64,
    pub kind: i32,
}

impl ModifierObservation {
    pub(crate) fn credit(self) -> Result<i32, PolicyFailure> {
        let value = Decimal::parse(self.value_low, self.value_high)?;
        match self.kind {
            0 => value
                .truncated()?
                .checked_abs()
                .ok_or(PolicyFailure::Arithmetic),
            1 => value.truncated(),
            2 | 3 => {
                let basis = Decimal::parse(self.basis_low, self.basis_high)?
                    .min(Decimal::parse(self.limit_low, self.limit_high)?);
                let delta = if self.kind == 2 {
                    value.subtract_one()?
                } else {
                    value
                };
                basis.product(&delta)?.truncated()
            }
            _ => Err(PolicyFailure::Packet),
        }
    }
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
        let scale = (flags >> 16) & 0xff;
        if flags & 0x7f00ffff != 0 || scale > 28 {
            return Err(PolicyFailure::Packet);
        }
        let mut units = i128::from(low) | (i128::from(high as u32) << 64);
        if flags >> 31 != 0 {
            units = -units;
        }
        Ok(Self { units, scale })
    }

    fn rounded(units: BigInt, scale: u32) -> Result<Self, PolicyFailure> {
        let negative = units.sign() == num_bigint::Sign::Minus;
        let magnitude = if negative { -units } else { units };
        let maximum = BigInt::from((1_u128 << 96) - 1);
        for discarded in scale.saturating_sub(28)..=scale {
            let divisor = BigInt::from(10_u8).pow(discarded);
            let mut quotient = &magnitude / &divisor;
            let remainder = (&magnitude % &divisor) * 2;
            if remainder > divisor
                || (remainder == divisor && (&quotient % 2_u8) != BigInt::from(0_u8))
            {
                quotient += 1;
            }
            if quotient <= maximum {
                let units = i128::try_from(quotient).map_err(|_| PolicyFailure::Arithmetic)?;
                return Ok(Self {
                    units: if negative { -units } else { units },
                    scale: scale - discarded,
                });
            }
        }
        Err(PolicyFailure::Arithmetic)
    }

    fn subtract_one(&self) -> Result<Self, PolicyFailure> {
        Self::rounded(
            BigInt::from(self.units) - BigInt::from(10_u8).pow(self.scale),
            self.scale,
        )
    }

    fn min(self, other: Self) -> Self {
        let left = BigInt::from(self.units) * BigInt::from(10_u8).pow(other.scale);
        let right = BigInt::from(other.units) * BigInt::from(10_u8).pow(self.scale);
        if left <= right { self } else { other }
    }

    fn product(&self, other: &Self) -> Result<Self, PolicyFailure> {
        Self::rounded(
            BigInt::from(self.units) * BigInt::from(other.units),
            self.scale + other.scale,
        )
    }

    fn truncated(&self) -> Result<i32, PolicyFailure> {
        i32::try_from(self.units / 10_i128.pow(self.scale)).map_err(|_| PolicyFailure::Arithmetic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(
        basis: i128,
        basis_scale: u32,
        value: i128,
        value_scale: u32,
        kind: i32,
    ) -> ModifierObservation {
        let bits = |units: i128, scale: u32| {
            let magnitude = units.unsigned_abs();
            let flags = (scale << 16) | if units < 0 { 1 << 31 } else { 0 };
            (
                magnitude as u64,
                (magnitude >> 64) as u64 | (u64::from(flags) << 32),
            )
        };
        let (basis_low, basis_high) = bits(basis, basis_scale);
        let (value_low, value_high) = bits(value, value_scale);
        ModifierObservation {
            basis_low,
            basis_high,
            value_low,
            value_high,
            limit_low: u64::MAX,
            limit_high: u64::from(u32::MAX),
            kind,
        }
    }
    #[test]
    fn decimal_rounding_precedes_credit_truncation() {
        let one = 10_i128.pow(28);
        assert_eq!(observation(one - 1, 28, 2 * one + 1, 28, 2).credit(), Ok(1));
        assert_eq!(observation(one - 1, 28, one + 1, 28, 3).credit(), Ok(1));
        assert_eq!(observation(0, 0, -19, 1, 0).credit(), Ok(1));
        assert_eq!(observation(0, 0, -19, 1, 1).credit(), Ok(-1));
        assert_eq!(
            observation(0, 0, i128::from(i32::MIN), 0, 0).credit(),
            Err(PolicyFailure::Arithmetic)
        );
        assert_eq!(
            observation((1_i128 << 96) - 1, 0, 2, 0, 3).credit(),
            Err(PolicyFailure::Arithmetic)
        );
    }
    #[test]
    fn product_rounds_ties_to_even_at_the_decimal_scale_limit() {
        let maximum = (1_i128 << 96) - 1;
        let a = Decimal {
            units: 5,
            scale: 28,
        };
        assert_eq!(
            a.product(&Decimal { units: 1, scale: 1 })
                .expect("fits")
                .units,
            0
        );
        let a = Decimal {
            units: 15,
            scale: 28,
        };
        assert_eq!(
            a.product(&Decimal { units: 1, scale: 1 })
                .expect("fits")
                .units,
            2
        );
        assert_eq!(
            Decimal::rounded(BigInt::from(maximum) * 10 + 4, 1)
                .expect("rounds into coefficient")
                .units,
            maximum
        );
        assert!(Decimal::rounded(BigInt::from(maximum) * 10 + 5, 1).is_err());
    }
}
