use crate::value::{CopyAllocation, Value, try_copy_value};
use keld_ir::AllocationPhase;
use keld_runtime::{LIST_CAPACITY_ELEMENT_BYTES, checked_list_capacity};
use std::collections::BTreeSet;

const _: () = assert!(std::mem::size_of::<Value>() == LIST_CAPACITY_ELEMENT_BYTES);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityError {
    Impossible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReserveFailure {
    Capacity,
    Allocation,
}

/// Policy hook used by executable List growth. The interpreter's historical
/// [`AllocationController`] implements the hook, while the full Native-1 test
/// controller layers frozen site IDs on top of the same preferred/exact
/// growth points.
pub trait AllocationPolicy {
    fn allow_list_attempt(&mut self, phase: AllocationPhase) -> bool;
}

#[derive(Clone, Debug, Default)]
pub struct AllocationController {
    list_attempt: u64,
    fail_list_attempts: BTreeSet<u64>,
}

impl AllocationController {
    #[must_use]
    pub fn fail_list_attempts(attempts: impl IntoIterator<Item = u64>) -> Self {
        Self {
            list_attempt: 0,
            fail_list_attempts: attempts.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn list_attempts(&self) -> u64 {
        self.list_attempt
    }

    fn allow_list_attempt(&mut self) -> bool {
        self.list_attempt = self.list_attempt.saturating_add(1);
        !self.fail_list_attempts.contains(&self.list_attempt)
    }
}

impl AllocationPolicy for AllocationController {
    fn allow_list_attempt(&mut self, _phase: AllocationPhase) -> bool {
        self.allow_list_attempt()
    }
}

/// Computes the checked element capacity required by a reservation.
///
/// # Errors
///
/// Returns [`CapacityError::Impossible`] when the signed source value, element
/// count, or byte address calculation cannot be represented.
pub fn required_capacity(length: usize, additional: i64) -> Result<usize, CapacityError> {
    checked_list_capacity(length, additional).ok_or(CapacityError::Impossible)
}

#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeList {
    elements: Vec<Value>,
}

impl RuntimeList {
    #[doc(hidden)]
    #[must_use]
    pub fn with_capacity_for_test(capacity: usize) -> Self {
        Self {
            elements: Vec::with_capacity(capacity),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn from_values(values: Vec<Value>) -> Self {
        Self { elements: values }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn length(&self) -> usize {
        self.elements.len()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn capacity_for_test(&self) -> usize {
        self.elements.capacity()
    }

    #[doc(hidden)]
    pub fn push_for_test(&mut self, value: Value) {
        self.elements.push(value);
    }

    #[doc(hidden)]
    #[must_use]
    pub fn values_for_test(&self) -> &[Value] {
        &self.elements
    }

    #[doc(hidden)]
    pub fn push(
        &mut self,
        value: Value,
        allocations: &mut impl AllocationPolicy,
    ) -> Result<(), ReserveFailure> {
        self.reserve(1, allocations)?;
        self.elements.push(value);
        Ok(())
    }

    #[doc(hidden)]
    pub fn push_with_controller(
        &mut self,
        value: Value,
        allocations: &mut AllocationController,
    ) -> Result<(), ReserveFailure> {
        self.push(value, allocations)
    }

    #[doc(hidden)]
    pub fn reserve(
        &mut self,
        additional: i64,
        allocations: &mut impl AllocationPolicy,
    ) -> Result<(), ReserveFailure> {
        let required = required_capacity(self.elements.len(), additional)
            .map_err(|CapacityError::Impossible| ReserveFailure::Capacity)?;
        self.ensure_capacity(required, allocations)
    }

    #[doc(hidden)]
    pub fn try_reserve(
        &mut self,
        additional: i64,
        allocations: &mut impl AllocationPolicy,
    ) -> bool {
        self.reserve(additional, allocations).is_ok()
    }

    fn ensure_capacity(
        &mut self,
        required: usize,
        allocations: &mut impl AllocationPolicy,
    ) -> Result<(), ReserveFailure> {
        if required <= self.elements.capacity() {
            return Ok(());
        }
        let preferred = required.max(self.elements.capacity().saturating_mul(2).max(4));
        if self.try_allocate(preferred, AllocationPhase::ListGrowthPreferred, allocations)
            || (preferred != required
                && self.try_allocate(required, AllocationPhase::ListGrowthExact, allocations))
        {
            Ok(())
        } else {
            Err(ReserveFailure::Allocation)
        }
    }

    fn try_allocate(
        &mut self,
        target: usize,
        phase: AllocationPhase,
        allocations: &mut impl AllocationPolicy,
    ) -> bool {
        if !allocations.allow_list_attempt(phase) {
            return false;
        }
        let additional = target
            .checked_sub(self.elements.len())
            .expect("target exceeds current length");
        if self.elements.try_reserve(additional).is_err() {
            return false;
        }
        debug_assert!(
            self.elements.capacity() >= target,
            "successful Vec reservation must meet the requested target"
        );
        self.elements.capacity() >= target
    }

    #[doc(hidden)]
    pub fn get_copy(&self, index: usize) -> Result<Option<Value>, CopyAllocation> {
        self.elements.get(index).map(try_copy_value).transpose()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn into_elements(self) -> std::vec::IntoIter<Value> {
        self.elements.into_iter()
    }

    #[doc(hidden)]
    pub fn remove(&mut self, index: usize) -> Value {
        self.elements.remove(index)
    }

    #[doc(hidden)]
    pub fn try_remove(&mut self, index: usize) -> Option<Value> {
        (index < self.elements.len()).then(|| self.remove(index))
    }

    #[doc(hidden)]
    pub fn pop(&mut self) -> Option<Value> {
        self.elements.pop()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&Value> {
        self.elements.get(index)
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.elements.get_mut(index)
    }

    pub(crate) fn as_slice(&self) -> &[Value] {
        &self.elements
    }
}
