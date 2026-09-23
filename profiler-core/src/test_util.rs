//! Test fixtures for deterministic accounting walks.
use std::sync::atomic::Ordering;
pub fn combat_epoch() -> u64 {
    crate::data::state::STATE.with(|cell| {
        u64::from(
            cell.borrow()
                .current
                .as_ref()
                .expect("fixture combat exists")
                .seq,
        )
    })
}

pub struct SourceFixture {
    epoch: u64,
    instance: u64,
    id: Box<str>,
    slot: i32,
    generation: i32,
    role: i32,
    segment: i32,
    handle: u64,
}

impl SourceFixture {
    fn identity() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("fixture identities fit the u64 counter")
    }

    pub fn card(id: &str, slot: i32) -> Self {
        Self::capture(id, slot, Self::identity(), 1, 0, 0, 1, 0)
    }

    pub fn generated(id: &str, slot: i32, instance: u64) -> Self {
        Self::capture(id, slot, instance, 1, 0, 1, 1, 0)
    }

    pub fn relic(id: &str, slot: i32) -> Self {
        Self::capture(id, slot, 0, 4, 1, 0, 3, 0)
    }

    pub fn power(instance: u64) -> Self {
        Self::capture("", 4, instance, 2, 2, 0, 2, 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn capture(
        id: &str,
        slot: i32,
        instance: u64,
        capture: i32,
        kind: i32,
        generation: i32,
        role: i32,
        segment: i32,
    ) -> Self {
        use crate::data::events;
        let epoch = combat_epoch();
        let transfer = events::source_capture(epoch, capture, instance, id, kind, slot, generation);
        assert_ne!(transfer, 0);
        Self {
            epoch,
            instance,
            id: id.into(),
            slot,
            generation,
            role,
            segment,
            handle: transfer,
        }
    }

    pub fn with_transfer<T>(&self, operation: impl FnOnce(u64) -> T) -> T {
        operation(self.handle)
    }

    pub fn play(&self) -> u64 {
        self.with_transfer(|source| {
            crate::data::events::card_play_started(
                self.epoch,
                Self::identity(),
                self.instance,
                &self.id,
                self.slot,
                0,
                1,
                self.generation,
                source,
            )
        })
    }

    pub fn finish(&self, play: u64) {
        assert_eq!(crate::data::events::card_play_finished(play), 1);
    }

    pub fn hit(&self, total: i32, blocked: i32, kind: i32, receiver: i32) {
        use crate::data::events;
        let calculation = self.with_transfer(|source| {
            events::damage_calculation_begin(self.epoch, source, self.role, self.segment, 999)
        });
        assert_ne!(calculation, 0);
        assert_eq!(
            events::damage_result_append(
                calculation,
                total,
                total - blocked,
                blocked,
                kind,
                receiver,
                0
            ),
            1
        );
        assert_eq!(events::damage_calculation_commit(calculation), 1);
    }

    pub fn deal(&self, total: i32, blocked: i32) {
        self.hit(total, blocked, 0, 4);
    }

    pub fn block(&self, amount: i32, receiver: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::block_gained(
                self.epoch, amount, source, receiver
            )),
            1
        );
    }

    pub fn forge(&self, amount: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::forge(self.epoch, source, amount)),
            1
        );
    }

    pub fn generate(&self, instance: u64) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::card_generated(
                self.epoch, instance, source, self.role
            )),
            1
        );
    }

    pub fn contribution(&self, calculation: u64, amount: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::damage_modifier_contribution(
                calculation,
                source,
                amount
            )),
            1
        );
    }
}
