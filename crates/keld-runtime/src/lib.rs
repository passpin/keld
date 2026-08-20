mod id;
mod lifecycle;
mod slot;
mod store;

/// Native-1's checked semantic size for one List element.
///
/// The interpreter's `Value` is 48 bytes on the frozen x86-64 GNU target;
/// native storage uses a smaller ABI envelope, so List capacity checks must
/// use this semantic size rather than the native payload representation.
pub const LIST_CAPACITY_ELEMENT_BYTES: usize = 48;

/// Computes the checked List length shared by interpreter and native runtime.
///
/// The result is rejected when the signed source count, element count, or
/// semantic byte allocation cannot be represented by the target allocator.
#[must_use]
pub fn checked_list_capacity(length: usize, additional: i64) -> Option<usize> {
    let additional = usize::try_from(additional).ok()?;
    let required = length.checked_add(additional)?;
    let _source_length = i64::try_from(required).ok()?;
    let bytes = required.checked_mul(LIST_CAPACITY_ELEMENT_BYTES)?;
    isize::try_from(bytes).ok().map(|_| required)
}

pub use id::{
    EntityId, Generation, Link, RuntimeLifecycleId, RuntimeTypeId, SlotIndex, StoreBrand,
};
pub use store::{Store, StoreError, StoreInvariantError};
