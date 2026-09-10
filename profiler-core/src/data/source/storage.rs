use std::collections::TryReserveError;
use std::ops::{Deref, DerefMut};

pub(super) struct Slots<T> {
    storage: Vec<T>,
    len: usize,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            storage: Vec::new(),
            len: 0,
        }
    }
}

impl<T> Slots<T> {
    pub(super) fn try_init(
        &mut self,
        capacity: usize,
        mut initialize: impl FnMut() -> Result<T, TryReserveError>,
    ) -> Result<(), TryReserveError> {
        if self.storage.len() < capacity {
            self.storage
                .try_reserve_exact(capacity - self.storage.len())?;
            while self.storage.len() < capacity {
                self.storage.push(initialize()?);
            }
        }
        Ok(())
    }

    pub(super) fn vacant_mut(&mut self) -> Option<&mut T> {
        self.storage.get_mut(self.len)
    }

    pub(super) fn activate(&mut self) -> bool {
        if self.len == self.storage.len() {
            return false;
        }
        self.len += 1;
        true
    }

    pub(super) fn remove(&mut self, index: usize) {
        debug_assert!(index < self.len, "removal requires an occupied slot");
        if index >= self.len {
            return;
        }
        // Preserve live order and move the removed buffer owner into the reserve.
        self.storage[index..self.len].rotate_left(1);
        self.len -= 1;
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let mut index = 0;
        while index < self.len {
            if keep(&self.storage[index]) {
                index += 1;
            } else {
                self.remove(index);
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.len = 0;
    }

    pub(super) fn all_slots(&self) -> &[T] {
        &self.storage
    }
}

impl<T> Deref for Slots<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.storage[..self.len]
    }
}

impl<T> DerefMut for Slots<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage[..self.len]
    }
}

impl<'a, T> IntoIterator for &'a Slots<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Slots<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T: Clone + Default> Clone for Slots<T> {
    fn clone(&self) -> Self {
        let mut result = Self {
            storage: (0..self.storage.len()).map(|_| T::default()).collect(),
            len: 0,
        };
        result.clone_from(self);
        result
    }

    fn clone_from(&mut self, source: &Self) {
        if self.storage.len() < source.storage.len() {
            self.storage.resize_with(source.storage.len(), T::default);
        }
        for (destination, source) in self.storage.iter_mut().zip(source.iter()) {
            destination.clone_from(source);
        }
        self.len = source.len;
    }
}
