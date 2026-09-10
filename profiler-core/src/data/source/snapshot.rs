//! Immutable normalized supplier vectors. Representation fields stay private
//! here; initialized owners refill checked snapshots and allocation cursors.
//! Live owners reserve the destination cap before use. Empty defaults are
//! staging placeholders; copying into them acquires their backing storage.

use std::collections::TryReserveError;

use super::{
    CombatEpoch, Destination, PAYLOAD_MAX, SourceDiagnostics, SourceFailure, TEAM_SLOT, caps,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct WeightedDestination {
    destination: Destination,
    weight: u64,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct SourceSnapshot {
    combat_seq: u32,
    shares: Vec<WeightedDestination>,
}

#[derive(Default)]
pub(super) struct SnapshotScratch {
    merged: Vec<(Destination, u128)>,
}

impl SnapshotScratch {
    pub(super) fn try_new() -> Result<Self, TryReserveError> {
        let mut merged = Vec::new();
        merged.try_reserve_exact(caps::SOURCE_DESTINATIONS)?;
        Ok(Self { merged })
    }
}

impl Clone for SourceSnapshot {
    fn clone(&self) -> Self {
        let mut result = Self {
            combat_seq: 0,
            shares: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
        };
        result.clone_from(self);
        result
    }

    fn clone_from(&mut self, source: &Self) {
        if self.shares.capacity() == 0 {
            self.shares.reserve_exact(caps::SOURCE_DESTINATIONS);
        }
        debug_assert!(
            self.shares.capacity() >= caps::SOURCE_DESTINATIONS,
            "snapshot copies require an initialized destination owner"
        );
        self.combat_seq = source.combat_seq;
        self.shares.clone_from(&source.shares);
    }
}

impl SourceSnapshot {
    pub(super) fn clear(&mut self) {
        self.combat_seq = 0;
        self.shares.clear();
    }

    pub(super) fn combat_seq(&self) -> u32 {
        self.combat_seq
    }
    pub(super) fn shares(&self) -> &[WeightedDestination] {
        &self.shares
    }

    fn total_weight(&self) -> u64 {
        self.shares.iter().map(|share| share.weight).sum()
    }

    pub(super) fn try_new() -> Result<Self, TryReserveError> {
        let mut shares = Vec::new();
        shares.try_reserve_exact(caps::SOURCE_DESTINATIONS)?;
        Ok(Self {
            combat_seq: 0,
            shares,
        })
    }

    pub(super) fn set_single(&mut self, epoch: CombatEpoch, destination: Destination) {
        debug_assert!(
            self.shares.capacity() >= caps::SOURCE_DESTINATIONS,
            "snapshot fills require an initialized destination owner"
        );
        self.shares.clear();
        self.shares.push(WeightedDestination {
            destination,
            weight: 1,
        });
        self.combat_seq = epoch.0.get();
    }

    pub(super) fn set_unknown(&mut self, epoch: CombatEpoch) {
        self.set_single(epoch, Destination::Unknown(TEAM_SLOT));
    }

    pub(super) fn is_unknown(&self) -> bool {
        self.shares.len() == 1
            && self.shares[0].destination == Destination::Unknown(TEAM_SLOT)
            && self.shares[0].weight == 1
    }

    #[cfg(test)]
    pub(super) fn normalized(
        epoch: CombatEpoch,
        rows: usize,
        entries: Vec<(Destination, u128)>,
    ) -> Result<Self, SourceFailure> {
        let mut result = Self::try_new().map_err(|_| SourceFailure::Capacity)?;
        let mut scratch = SnapshotScratch::try_new().map_err(|_| SourceFailure::Capacity)?;
        result.normalize_into(epoch, rows, &entries, &mut scratch)?;
        Ok(result)
    }

    pub(super) fn normalize_into(
        &mut self,
        epoch: CombatEpoch,
        rows: usize,
        entries: &[(Destination, u128)],
        scratch: &mut SnapshotScratch,
    ) -> Result<(), SourceFailure> {
        if entries.is_empty() || entries.len() > caps::SOURCE_DESTINATIONS {
            return Err(SourceFailure::Capacity);
        }
        debug_assert!(
            scratch.merged.capacity() >= caps::SOURCE_DESTINATIONS,
            "normalization requires initialized merge scratch"
        );
        scratch.merged.clear();
        for &(destination, weight) in entries {
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
            if let Some((_, previous)) = scratch
                .merged
                .iter_mut()
                .find(|(key, _)| *key == destination)
            {
                *previous = previous
                    .checked_add(weight)
                    .ok_or(SourceFailure::Arithmetic)?;
            } else {
                scratch.merged.push((destination, weight));
            }
        }
        self.normalize_merged(epoch, rows, &scratch.merged)
    }

    fn normalize_merged(
        &mut self,
        epoch: CombatEpoch,
        rows: usize,
        merged: &[(Destination, u128)],
    ) -> Result<(), SourceFailure> {
        if merged.is_empty() || merged.len() > caps::SOURCE_DESTINATIONS {
            return Err(SourceFailure::Capacity);
        }
        let divisor = merged
            .iter()
            .fold(0, |gcd, (_, weight)| Self::gcd(gcd, *weight));
        let mut total = 0_u64;
        for &(destination, weight) in merged {
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
            let weight = u64::try_from(weight / divisor).map_err(|_| SourceFailure::Arithmetic)?;
            total = total.checked_add(weight).ok_or(SourceFailure::Arithmetic)?;
        }
        debug_assert!(
            self.shares.capacity() >= caps::SOURCE_DESTINATIONS,
            "normalized publication requires an initialized destination owner"
        );
        self.shares.clear();
        self.shares.extend(
            merged
                .iter()
                .map(|&(destination, weight)| WeightedDestination {
                    destination,
                    weight: (weight / divisor) as u64,
                }),
        );
        self.combat_seq = epoch.0.get();
        Ok(())
    }

    fn gcd(mut a: u128, mut b: u128) -> u128 {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    }

    #[cfg(test)]
    pub(super) fn unknown(epoch: CombatEpoch) -> Self {
        let mut result = Self {
            combat_seq: 0,
            shares: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
        };
        result.set_unknown(epoch);
        result
    }

    #[cfg(test)]
    pub(super) fn mixture(
        epoch: CombatEpoch,
        rows: usize,
        grants: &[PowerGrant],
        diagnostics: &mut SourceDiagnostics,
    ) -> Self {
        let mut result = Self::unknown(epoch);
        let Ok(mut scratch) = SnapshotScratch::try_new() else {
            diagnostics.report(SourceFailure::Capacity);
            return result;
        };
        result.mixture_into(
            epoch,
            rows,
            grants.iter().map(|grant| (grant.remaining, &grant.source)),
            &mut scratch,
            diagnostics,
        );
        result
    }

    pub(super) fn mixture_into<'a>(
        &mut self,
        epoch: CombatEpoch,
        rows: usize,
        grants: impl IntoIterator<Item = (u32, &'a SourceSnapshot)>,
        scratch: &mut SnapshotScratch,
        diagnostics: &mut SourceDiagnostics,
    ) {
        let result = (|| {
            debug_assert!(
                scratch.merged.capacity() >= caps::SOURCE_DESTINATIONS,
                "mixtures require initialized merge scratch"
            );
            scratch.merged.clear();
            let mut denominator = 1_u128;
            for (index, (remaining, source)) in grants.into_iter().enumerate() {
                if index == caps::POWER_GRANTS_TOTAL {
                    return Err(SourceFailure::Capacity);
                }
                if remaining == 0 {
                    continue;
                }
                if source.combat_seq != epoch.0.get() {
                    return Err(SourceFailure::Epoch);
                }
                let total = u128::from(source.total_weight());
                if total == 0 {
                    return Err(SourceFailure::Packet);
                }
                let divisor = Self::gcd(denominator, total);
                let old_scale = total / divisor;
                let new_scale = denominator / divisor;
                denominator = denominator
                    .checked_mul(old_scale)
                    .ok_or(SourceFailure::Arithmetic)?;
                for (_, weight) in &mut scratch.merged {
                    *weight = weight
                        .checked_mul(old_scale)
                        .ok_or(SourceFailure::Arithmetic)?;
                }
                for share in &source.shares {
                    let weight = u128::from(share.weight)
                        .checked_mul(u128::from(remaining))
                        .and_then(|value| value.checked_mul(new_scale))
                        .ok_or(SourceFailure::Arithmetic)?;
                    if let Some((_, existing)) = scratch
                        .merged
                        .iter_mut()
                        .find(|(key, _)| *key == share.destination)
                    {
                        *existing = existing
                            .checked_add(weight)
                            .ok_or(SourceFailure::Arithmetic)?;
                    } else {
                        if scratch.merged.len() == caps::SOURCE_DESTINATIONS {
                            return Err(SourceFailure::Capacity);
                        }
                        scratch.merged.push((share.destination, weight));
                    }
                }
                let common = scratch
                    .merged
                    .iter()
                    .fold(denominator, |gcd, (_, weight)| Self::gcd(gcd, *weight));
                denominator /= common;
                for (_, weight) in &mut scratch.merged {
                    *weight /= common;
                }
            }
            self.normalize_merged(epoch, rows, &scratch.merged)
        })();
        if let Err(failure) = result {
            diagnostics.report(failure);
            self.set_unknown(epoch);
        }
    }

    #[cfg(test)]
    pub(super) fn prefix(&self) -> SourcePrefix {
        let mut result = SourcePrefix {
            source: self.clone(),
            credited_total: 0,
            credits: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
            candidate: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
        };
        result.reset_from(self);
        result
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

pub(super) struct PowerGrant {
    remaining: u32,
    source: SourceSnapshot,
}

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

#[derive(Default)]
pub(super) struct SourcePrefix {
    source: SourceSnapshot,
    credited_total: u64,
    credits: Vec<u64>,
    candidate: Vec<u64>,
}

impl Clone for SourcePrefix {
    fn clone(&self) -> Self {
        let mut result = Self {
            source: self.source.clone(),
            credited_total: 0,
            credits: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
            candidate: Vec::with_capacity(caps::SOURCE_DESTINATIONS),
        };
        result.clone_from(self);
        result
    }

    fn clone_from(&mut self, source: &Self) {
        if self.credits.capacity() == 0 {
            self.credits.reserve_exact(caps::SOURCE_DESTINATIONS);
        }
        if self.candidate.capacity() == 0 {
            self.candidate.reserve_exact(caps::SOURCE_DESTINATIONS);
        }
        debug_assert!(
            self.credits.capacity() >= caps::SOURCE_DESTINATIONS
                && self.candidate.capacity() >= caps::SOURCE_DESTINATIONS,
            "prefix copies require initialized cursor owners"
        );
        self.source.clone_from(&source.source);
        self.credited_total = source.credited_total;
        self.credits.clone_from(&source.credits);
        self.candidate.clear();
    }
}

impl SourcePrefix {
    pub(super) fn try_new() -> Result<Self, TryReserveError> {
        let source = SourceSnapshot::try_new()?;
        let mut credits = Vec::new();
        let mut candidate = Vec::new();
        credits.try_reserve_exact(caps::SOURCE_DESTINATIONS)?;
        candidate.try_reserve_exact(caps::SOURCE_DESTINATIONS)?;
        Ok(Self {
            source,
            credited_total: 0,
            credits,
            candidate,
        })
    }

    pub(super) fn reset_from(&mut self, source: &SourceSnapshot) {
        if self.credits.capacity() == 0 {
            self.credits.reserve_exact(caps::SOURCE_DESTINATIONS);
        }
        if self.candidate.capacity() == 0 {
            self.candidate.reserve_exact(caps::SOURCE_DESTINATIONS);
        }
        debug_assert!(
            self.credits.capacity() >= caps::SOURCE_DESTINATIONS
                && self.candidate.capacity() >= caps::SOURCE_DESTINATIONS,
            "prefix resets require initialized cursor owners"
        );
        self.source.clone_from(source);
        self.credited_total = 0;
        self.credits.clear();
        self.credits.resize(source.shares.len(), 0);
        self.candidate.clear();
    }

    pub(super) fn source(&self) -> &SourceSnapshot {
        &self.source
    }

    pub(super) fn credit(&mut self, amount: u64) -> Result<Vec<(Destination, u64)>, SourceFailure> {
        self.credit_iter(amount).map(Iterator::collect)
    }

    pub(super) fn credit_iter(
        &mut self,
        amount: u64,
    ) -> Result<impl Iterator<Item = (Destination, u64)> + '_, SourceFailure> {
        let total = self
            .credited_total
            .checked_add(amount)
            .ok_or(SourceFailure::Arithmetic)?;
        let weight_total = self.source.total_weight();
        if weight_total == 0 {
            return Err(SourceFailure::Packet);
        }
        debug_assert!(
            self.candidate.capacity() >= caps::SOURCE_DESTINATIONS,
            "prefix advancement requires initialized candidate scratch"
        );
        self.candidate.clear();
        self.candidate
            .extend(self.source.shares.iter().map(|share| {
                ((u128::from(total) * u128::from(share.weight)) / u128::from(weight_total)) as u64
            }));
        let remainder = total - self.candidate.iter().sum::<u64>();
        debug_assert!(
            remainder < self.candidate.len() as u64,
            "quota floors leave less than one missing credit per root"
        );
        // Floors include every quotient seat >= sum(weights)/total. Fewer than
        // N further seats complete the same fixed order, so prefixes are nested.
        for _ in 0..remainder {
            let mut winner = 0;
            for index in 1..self.candidate.len() {
                let candidate = u128::from(self.source.shares[index].weight)
                    .checked_mul(u128::from(self.candidate[winner]) + 1)
                    .ok_or(SourceFailure::Arithmetic)?;
                let incumbent = u128::from(self.source.shares[winner].weight)
                    .checked_mul(u128::from(self.candidate[index]) + 1)
                    .ok_or(SourceFailure::Arithmetic)?;
                if candidate >= incumbent {
                    winner = index;
                }
            }
            self.candidate[winner] = self.candidate[winner]
                .checked_add(1)
                .ok_or(SourceFailure::Arithmetic)?;
        }
        for (before, after) in self.credits.iter().zip(&self.candidate) {
            after
                .checked_sub(*before)
                .ok_or(SourceFailure::Arithmetic)?;
        }
        self.credited_total = total;
        std::mem::swap(&mut self.credits, &mut self.candidate);
        Ok(self
            .source
            .shares
            .iter()
            .zip(&self.credits)
            .zip(&self.candidate)
            .filter_map(|((share, after), before)| {
                let delta = after - before;
                (delta > 0).then_some((share.destination, delta))
            }))
    }
}

impl Clone for PowerGrant {
    fn clone(&self) -> Self {
        Self {
            remaining: self.remaining,
            source: self.source.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.remaining = source.remaining;
        self.source.clone_from(&source.source);
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
    pub(super) fn try_new() -> Result<Self, TryReserveError> {
        Ok(Self {
            remaining: 0,
            source: SourceSnapshot::try_new()?,
        })
    }

    pub(super) fn reset_from(&mut self, remaining: u32, source: &SourceSnapshot) {
        self.remaining = remaining;
        self.source.clone_from(source);
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

#[cfg(any(test, feature = "test-support"))]
impl super::State {
    #[allow(
        clippy::too_many_lines,
        reason = "fixture initialization stays outside the returned measured computation"
    )]
    pub(crate) fn snapshot_allocation_fixture() -> impl FnMut() {
        let epoch = CombatEpoch::from_wire(7).expect("fixture epoch is nonzero and fits u32");
        let mut source = SourceSnapshot::try_new().expect("fixture snapshot reservation succeeds");
        let mut copied = SourceSnapshot::try_new().expect("fixture copy reservation succeeds");
        let mut scratch = SnapshotScratch::try_new().expect("fixture scratch reservation succeeds");
        let mut prefix = SourcePrefix::try_new().expect("fixture prefix reservation succeeds");
        let mut copied_prefix =
            SourcePrefix::try_new().expect("fixture cursor reservation succeeds");
        let entries: Vec<_> = (0..caps::SOURCE_DESTINATIONS)
            .map(|row| (Destination::Row(row), row as u128 + 1))
            .collect();
        let mut excess = entries.clone();
        excess.push((Destination::Unknown(TEAM_SLOT), 1));
        let source_allocation = source.shares.as_ptr();
        let scratch_allocation = scratch.merged.as_ptr();
        let cursor_allocations = [prefix.credits.as_ptr(), prefix.candidate.as_ptr()];
        move || {
            let mut diagnostics = SourceDiagnostics::default();
            source.set_unknown(epoch);
            source
                .normalize_into(epoch, caps::COMBAT_CARDS, &entries, &mut scratch)
                .expect("all bounded fixture roots and their total fit");
            copied.clone_from(&source);
            source.mixture_into(
                epoch,
                caps::COMBAT_CARDS,
                [(2, &copied), (3, &copied)],
                &mut scratch,
                &mut diagnostics,
            );
            assert_eq!(source, copied);
            for (entries, failure) in [
                (excess.as_slice(), SourceFailure::Capacity),
                (
                    &[(Destination::Row(0), u128::MAX), (Destination::Row(0), 1)],
                    SourceFailure::Arithmetic,
                ),
            ] {
                assert_eq!(
                    source.normalize_into(epoch, caps::COMBAT_CARDS, entries, &mut scratch),
                    Err(failure)
                );
                assert_eq!(source, copied);
            }
            prefix.reset_from(&source);
            for _ in 0..129 {
                assert_eq!(
                    prefix
                        .credit_iter(1)
                        .expect("unit cursor credit fits")
                        .map(|(_, amount)| amount)
                        .sum::<u64>(),
                    1
                );
            }
            copied_prefix.clone_from(&prefix);
            assert_eq!(
                copied_prefix
                    .credit_iter(100)
                    .expect("copied cursor advances independently")
                    .map(|(_, amount)| amount)
                    .sum::<u64>(),
                100
            );
            source.set_single(epoch, Destination::Unknown(0));
            assert_eq!(prefix.source(), &copied);
            let remainder = u64::MAX - 129;
            assert_eq!(
                prefix
                    .credit_iter(remainder)
                    .expect("last representable cursor total fits")
                    .map(|(_, amount)| amount)
                    .sum::<u64>(),
                remainder
            );
            assert!(matches!(
                prefix.credit_iter(1),
                Err(SourceFailure::Arithmetic)
            ));
            assert_eq!(prefix.credited_total, u64::MAX);
            assert_eq!(
                prefix
                    .credit_iter(0)
                    .expect("zero credit fits after overflow")
                    .count(),
                0
            );
            source.mixture_into(
                epoch,
                caps::COMBAT_CARDS,
                [(0, &copied)],
                &mut scratch,
                &mut diagnostics,
            );
            assert!(source.is_unknown());
            source.clear();
            assert_eq!(source.shares.as_ptr(), source_allocation);
            assert_eq!(scratch.merged.as_ptr(), scratch_allocation);
            assert!(cursor_allocations.contains(&prefix.credits.as_ptr()));
            assert!(cursor_allocations.contains(&prefix.candidate.as_ptr()));
        }
    }
}

#[cfg(test)]
mod tests;
