use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq)]
pub(crate) enum PolicyFailure {
    Packet,
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
