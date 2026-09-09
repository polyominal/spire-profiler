//! Immutable normalized supplier vectors. Representation fields stay private
//! here; gameplay code can clone values and consume checked allocation cursors.

use super::{
    CombatEpoch, Destination, PAYLOAD_MAX, SourceDiagnostics, SourceFailure, TEAM_SLOT, caps,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WeightedDestination {
    destination: Destination,
    weight: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceSnapshot {
    combat_seq: u32,
    shares: Vec<WeightedDestination>,
}

impl SourceSnapshot {
    pub(super) fn combat_seq(&self) -> u32 {
        self.combat_seq
    }
    pub(super) fn shares(&self) -> &[WeightedDestination] {
        &self.shares
    }

    pub(super) fn normalized(
        epoch: CombatEpoch,
        rows: usize,
        entries: Vec<(Destination, u128)>,
    ) -> Result<Self, SourceFailure> {
        if entries.is_empty() || entries.len() > caps::SOURCE_DESTINATIONS {
            return Err(SourceFailure::Capacity);
        }
        let mut merged: Vec<(Destination, u128)> = Vec::with_capacity(entries.len());
        for (destination, weight) in entries {
            if weight == 0 {
                return Err(SourceFailure::Packet);
            }
            match destination {
                Destination::Row(row) if row >= rows || row > PAYLOAD_MAX as usize => {
                    return Err(SourceFailure::Token);
                }
                Destination::Unknown(slot) if slot > TEAM_SLOT => {
                    return Err(SourceFailure::Packet);
                }
                _ => {}
            }
            if let Some((_, previous)) = merged.iter_mut().find(|(key, _)| *key == destination) {
                *previous = previous
                    .checked_add(weight)
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                merged.push((destination, weight));
            }
        }
        let divisor = merged
            .iter()
            .fold(0, |gcd, (_, weight)| Self::gcd(gcd, *weight));
        let mut total = 0_u64;
        let mut shares = Vec::with_capacity(merged.len());
        for (destination, weight) in merged {
            let weight = u64::try_from(weight / divisor).map_err(|_| SourceFailure::Arithmetic)?;
            total = total.checked_add(weight).ok_or(SourceFailure::Arithmetic)?;
            shares.push(WeightedDestination {
                destination,
                weight,
            });
        }
        Ok(Self {
            combat_seq: epoch.0.get(),
            shares,
        })
    }

    fn gcd(mut a: u128, mut b: u128) -> u128 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    }

    pub(super) fn unknown(epoch: CombatEpoch) -> Self {
        Self {
            combat_seq: epoch.0.get(),
            shares: vec![WeightedDestination {
                destination: Destination::Unknown(TEAM_SLOT),
                weight: 1,
            }],
        }
    }

    pub(super) fn mixture(
        epoch: CombatEpoch,
        rows: usize,
        grants: &[PowerGrant],
        diagnostics: &mut SourceDiagnostics,
    ) -> Self {
        let result = (|| {
            if grants.len() > caps::POWER_GRANTS_TOTAL {
                return Err(SourceFailure::Capacity);
            }
            let mut entries: Vec<(Destination, u128)> = Vec::new();
            let mut denominator = 1_u128;
            for grant in grants.iter().filter(|grant| grant.remaining > 0) {
                if grant.source.combat_seq != epoch.0.get() {
                    return Err(SourceFailure::Epoch);
                }
                let total = grant
                    .source
                    .shares
                    .iter()
                    .map(|share| u128::from(share.weight))
                    .sum();
                let divisor = Self::gcd(denominator, total);
                let old_scale = total / divisor;
                let new_scale = denominator / divisor;
                denominator = denominator
                    .checked_mul(old_scale)
                    .ok_or(SourceFailure::Arithmetic)?;
                for (_, weight) in &mut entries {
                    *weight = weight
                        .checked_mul(old_scale)
                        .ok_or(SourceFailure::Arithmetic)?;
                }
                for share in &grant.source.shares {
                    let weight = u128::from(share.weight)
                        .checked_mul(u128::from(grant.remaining))
                        .and_then(|value| value.checked_mul(new_scale))
                        .ok_or(SourceFailure::Arithmetic)?;
                    if let Some((_, existing)) = entries
                        .iter_mut()
                        .find(|(key, _)| *key == share.destination)
                    {
                        *existing = existing
                            .checked_add(weight)
                            .ok_or(SourceFailure::Arithmetic)?;
                    } else {
                        if entries.len() == caps::SOURCE_DESTINATIONS {
                            return Err(SourceFailure::Capacity);
                        }
                        entries.push((share.destination, weight));
                    }
                }
                let common = entries
                    .iter()
                    .fold(denominator, |gcd, (_, weight)| Self::gcd(gcd, *weight));
                denominator /= common;
                for (_, weight) in &mut entries {
                    *weight /= common;
                }
            }
            Self::normalized(epoch, rows, entries)
        })();
        match result {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                diagnostics.report(failure);
                Self::unknown(epoch)
            }
        }
    }

    pub(super) fn prefix(&self) -> SourcePrefix {
        SourcePrefix {
            source: self.clone(),
            credited_total: 0,
            credits: vec![0; self.shares.len()],
        }
    }

    pub(super) fn budgets(&self, amount: u64) -> RootBudgets {
        let weights: Vec<_> = self.shares.iter().map(|share| share.weight).collect();
        let amounts = RootBudgets::proportional(amount, &weights)
            .expect("normalized snapshots have a positive checked u64 weight total");
        RootBudgets {
            remaining: amount,
            shares: self
                .shares
                .iter()
                .zip(amounts)
                .map(|(share, amount)| (share.destination, amount))
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PowerGrant {
    remaining: u32,
    source: SourceSnapshot,
}

#[derive(Clone, Debug)]
pub(super) struct RootBudgets {
    remaining: u64,
    shares: Vec<(Destination, u64)>,
}

impl RootBudgets {
    pub(super) fn remaining(&self) -> u64 {
        self.remaining
    }

    pub(super) fn proportional(amount: u64, weights: &[u64]) -> Result<Vec<u64>, SourceFailure> {
        let total = weights
            .iter()
            .try_fold(0_u64, |sum, weight| sum.checked_add(*weight))
            .ok_or(SourceFailure::Arithmetic)?;
        if total == 0 {
            return Ok(vec![0; weights.len()]);
        }
        let mut cumulative = 0_u64;
        let mut previous = 0_u64;
        let mut allocation = Vec::with_capacity(weights.len());
        for weight in weights {
            cumulative += weight;
            let next = ((u128::from(amount) * u128::from(cumulative)) / u128::from(total)) as u64;
            allocation.push(next - previous);
            previous = next;
        }
        Ok(allocation)
    }

    pub(super) fn take(&mut self, amount: u64) -> Result<Vec<(Destination, u64)>, SourceFailure> {
        if amount > self.remaining {
            return Err(SourceFailure::Packet);
        }
        let weights: Vec<_> = self.shares.iter().map(|(_, amount)| *amount).collect();
        debug_assert_eq!(
            weights.iter().sum::<u64>(),
            self.remaining,
            "remaining roots must cover all future consumption"
        );
        let portions = Self::proportional(amount, &weights)?;
        let mut result = Vec::new();
        for ((destination, remaining), portion) in self.shares.iter_mut().zip(portions) {
            *remaining -= portion;
            if portion > 0 {
                result.push((*destination, portion));
            }
        }
        self.remaining -= amount;
        Ok(result)
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourcePrefix {
    source: SourceSnapshot,
    credited_total: u64,
    credits: Vec<u64>,
}

impl SourcePrefix {
    pub(super) fn source(&self) -> &SourceSnapshot {
        &self.source
    }

    fn allocation(&self, total: u64) -> Result<Vec<u64>, SourceFailure> {
        let weight_total: u64 = self.source.shares.iter().map(|share| share.weight).sum();
        let mut credits: Vec<u64> = self
            .source
            .shares
            .iter()
            .map(|share| {
                ((u128::from(total) * u128::from(share.weight)) / u128::from(weight_total)) as u64
            })
            .collect();
        let remainder = total - credits.iter().sum::<u64>();
        debug_assert!(
            remainder < credits.len() as u64,
            "quota floors leave less than one missing credit per root"
        );
        // Floors include every quotient seat >= sum(weights)/total. Fewer than
        // N further seats complete the same fixed order, so prefixes are nested.
        for _ in 0..remainder {
            let mut winner = 0;
            for index in 1..credits.len() {
                let candidate = u128::from(self.source.shares[index].weight)
                    .checked_mul(u128::from(credits[winner]) + 1)
                    .ok_or(SourceFailure::Arithmetic)?;
                let incumbent = u128::from(self.source.shares[winner].weight)
                    .checked_mul(u128::from(credits[index]) + 1)
                    .ok_or(SourceFailure::Arithmetic)?;
                if candidate >= incumbent {
                    winner = index;
                }
            }
            credits[winner] = credits[winner]
                .checked_add(1)
                .ok_or(SourceFailure::Arithmetic)?;
        }
        Ok(credits)
    }

    pub(super) fn credit(&mut self, amount: u64) -> Result<Vec<(Destination, u64)>, SourceFailure> {
        let total = self
            .credited_total
            .checked_add(amount)
            .ok_or(SourceFailure::Arithmetic)?;
        let credits = self.allocation(total)?;
        let mut result = Vec::new();
        for ((share, before), after) in self.source.shares.iter().zip(&self.credits).zip(&credits) {
            let delta = after
                .checked_sub(*before)
                .ok_or(SourceFailure::Arithmetic)?;
            if delta > 0 {
                result.push((share.destination, delta));
            }
        }
        self.credited_total = total;
        self.credits = credits;
        Ok(result)
    }
}

impl WeightedDestination {
    pub(super) fn destination(&self) -> Destination {
        self.destination
    }
    pub(super) fn weight(&self) -> u64 {
        self.weight
    }
}

impl PowerGrant {
    pub(super) fn new(remaining: u32, source: SourceSnapshot) -> Self {
        Self { remaining, source }
    }
    pub(super) fn remaining(&self) -> u32 {
        self.remaining
    }
    pub(super) fn source(&self) -> &SourceSnapshot {
        &self.source
    }
    pub(super) fn add(&mut self, amount: u32) -> Result<(), SourceFailure> {
        self.remaining = self
            .remaining
            .checked_add(amount)
            .ok_or(SourceFailure::Arithmetic)?;
        Ok(())
    }
    pub(super) fn consume(&mut self, amount: u32) -> u32 {
        let take = amount.min(self.remaining);
        self.remaining -= take;
        take
    }
}

#[cfg(test)]
mod tests;
