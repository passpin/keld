//! Safe, source-independent runtime core used by generated Native-1 programs.

#![forbid(unsafe_code)]

use keld_native_abi::{FaultKind, KeldFault, KeldHandle, KeldLifecycle, KeldValue, RuntimeStatus};
use keld_runtime::{Store, StoreError};
use std::fmt;

#[allow(dead_code)]
#[derive(Clone, Debug)]
enum NativePayload {
    Text(Vec<u8>),
    Struct {
        definition: u32,
        fields: Vec<(KeldValue, bool)>,
    },
    List {
        elements: Vec<(KeldValue, bool)>,
    },
}

#[derive(Debug)]
struct HandleSlot {
    generation: u32,
    payload: Option<NativePayload>,
}

/// Failure raised by an opaque native value operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeValueError {
    Allocation,
    InvalidHandle,
    TypeMismatch,
    Bounds,
}

impl fmt::Display for NativeValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Allocation => "native value allocation failed",
            Self::InvalidHandle => "native value handle is stale or foreign",
            Self::TypeMismatch => "native value kind does not match the operation",
            Self::Bounds => "native value index is out of bounds",
        })
    }
}

impl std::error::Error for NativeValueError {}

/// Error raised while creating the native runtime context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeContextError {
    Store(StoreError),
}

impl fmt::Display for RuntimeContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "runtime context setup failed: {error}"),
        }
    }
}

impl std::error::Error for RuntimeContextError {}

/// Runtime state shared by generated functions. The store remains source
/// independent; later Native-1 layers add managed payload descriptors here.
pub struct RuntimeContext {
    store: Store<()>,
    handles: Vec<HandleSlot>,
    reusable_handles: Vec<u32>,
    status: RuntimeStatus,
    first_failure: Option<KeldFault>,
}

impl RuntimeContext {
    /// Creates a context with an active root lifecycle.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeContextError::Store`] if the independent runtime store
    /// cannot allocate its brand or root lifecycle.
    pub fn new() -> Result<Self, RuntimeContextError> {
        Ok(Self {
            store: Store::new().map_err(RuntimeContextError::Store)?,
            handles: Vec::new(),
            reusable_handles: Vec::new(),
            status: RuntimeStatus::Ok,
            first_failure: None,
        })
    }

    #[must_use]
    pub const fn status(&self) -> RuntimeStatus {
        self.status
    }

    #[must_use]
    pub const fn first_failure(&self) -> Option<KeldFault> {
        self.first_failure
    }

    #[must_use]
    pub fn root_lifecycle(&self) -> KeldLifecycle {
        let (brand, index) = self.store.root_lifecycle().raw_parts();
        KeldLifecycle {
            brand,
            index,
            reserved: 0,
        }
    }

    /// Records a language fault only if no earlier status was recorded.
    pub fn record_language_fault(&mut self, kind: FaultKind, location: u32) {
        if self.status.is_ok() {
            self.status = RuntimeStatus::LanguageFault;
            self.first_failure = Some(KeldFault {
                kind: kind as u32,
                location,
            });
        }
    }

    /// Records an internal failure only if no earlier status was recorded.
    pub fn record_internal_failure(&mut self, location: u32) {
        if self.status.is_ok() {
            self.status = RuntimeStatus::InternalFailure;
            self.first_failure = Some(KeldFault { kind: 0, location });
        }
    }

    /// Creates an Int envelope without allocating a managed handle.
    #[must_use]
    pub const fn int_value(value: i64) -> KeldValue {
        KeldValue {
            optional_some_layers: 0,
            reserved: 0,
            words: [value.cast_unsigned(), 0, 0],
        }
    }

    /// Creates a canonical Bool envelope without allocating a managed handle.
    #[must_use]
    pub const fn bool_value(value: bool) -> KeldValue {
        KeldValue {
            optional_some_layers: 0,
            reserved: 0,
            words: [if value { 1 } else { 0 }, 0, 0],
        }
    }

    /// Allocates immutable UTF-8 Text storage.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::Allocation`] when the handle or byte buffer
    /// cannot be reserved, and [`NativeValueError::TypeMismatch`] for invalid
    /// UTF-8 input.
    pub fn text_new(&mut self, bytes: &[u8]) -> Result<KeldValue, NativeValueError> {
        if std::str::from_utf8(bytes).is_err() {
            return Err(NativeValueError::TypeMismatch);
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(bytes.len())
            .map_err(|_| NativeValueError::Allocation)?;
        owned.extend_from_slice(bytes);
        self.allocate(NativePayload::Text(owned))
    }

    /// Concatenates two Text handles with checked capacity.
    ///
    /// # Errors
    ///
    /// Returns a handle, type, or allocation failure without mutating either
    /// source value.
    pub fn text_concat(
        &mut self,
        lhs: KeldValue,
        rhs: KeldValue,
    ) -> Result<KeldValue, NativeValueError> {
        let left = self.text_bytes(lhs)?.to_vec();
        let right = self.text_bytes(rhs)?;
        let length = left
            .len()
            .checked_add(right.len())
            .ok_or(NativeValueError::Allocation)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| NativeValueError::Allocation)?;
        bytes.extend_from_slice(&left);
        bytes.extend_from_slice(right);
        self.allocate(NativePayload::Text(bytes))
    }

    /// Returns immutable Text bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::InvalidHandle`] or
    /// [`NativeValueError::TypeMismatch`] when the envelope is not Text.
    pub fn text_bytes(&self, value: KeldValue) -> Result<&[u8], NativeValueError> {
        match self.payload(value)? {
            NativePayload::Text(bytes) => Ok(bytes),
            NativePayload::Struct { .. } | NativePayload::List { .. } => {
                Err(NativeValueError::TypeMismatch)
            }
        }
    }

    /// Returns the byte length of a Text handle.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or type mismatch.
    pub fn text_byte_length(&self, value: KeldValue) -> Result<u64, NativeValueError> {
        u64::try_from(self.text_bytes(value)?.len()).map_err(|_| NativeValueError::Allocation)
    }

    /// Returns whether a Text handle contains no bytes.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or type mismatch.
    pub fn text_is_empty(&self, value: KeldValue) -> Result<bool, NativeValueError> {
        Ok(self.text_bytes(value)?.is_empty())
    }

    /// Constructs a struct by moving the supplied field envelopes into a new
    /// managed handle. `managed` is the validated field-type mask.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::Allocation`] or a stale managed field
    /// failure; input fields are not changed on failure.
    pub fn struct_new(
        &mut self,
        definition: u32,
        fields: &[KeldValue],
        managed: &[bool],
    ) -> Result<KeldValue, NativeValueError> {
        if fields.len() != managed.len() {
            return Err(NativeValueError::TypeMismatch);
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(fields.len())
            .map_err(|_| NativeValueError::Allocation)?;
        for (field, is_managed) in fields.iter().copied().zip(managed.iter().copied()) {
            if is_managed {
                self.validate_handle(field)?;
            }
            owned.push((field, is_managed));
        }
        self.allocate(NativePayload::Struct {
            definition,
            fields: owned,
        })
    }

    /// Reads one struct field and reports whether it is managed.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, type, or bounds failure.
    pub fn struct_field(
        &self,
        value: KeldValue,
        field: u32,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        let NativePayload::Struct { fields, .. } = self.payload(value)? else {
            return Err(NativeValueError::TypeMismatch);
        };
        fields
            .get(field as usize)
            .copied()
            .ok_or(NativeValueError::Bounds)
    }

    /// Allocates an empty List handle.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::Allocation`] if storage cannot be reserved.
    pub fn list_new(&mut self) -> Result<KeldValue, NativeValueError> {
        self.allocate(NativePayload::List {
            elements: Vec::new(),
        })
    }

    /// Returns a List's current element count.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or type mismatch.
    pub fn list_length(&self, value: KeldValue) -> Result<u64, NativeValueError> {
        let NativePayload::List { elements } = self.payload(value)? else {
            return Err(NativeValueError::TypeMismatch);
        };
        u64::try_from(elements.len()).map_err(|_| NativeValueError::Allocation)
    }

    /// Appends one element to a List without consuming the caller's envelope.
    ///
    /// # Errors
    ///
    /// Returns a stale element/list or allocation failure without mutation.
    pub fn list_push(
        &mut self,
        list: KeldValue,
        element: KeldValue,
        managed: bool,
    ) -> Result<(), NativeValueError> {
        if managed {
            self.validate_handle(element)?;
        }
        let (index, generation) = decode_handle(list)?;
        let slot = self
            .handles
            .get_mut(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        elements
            .try_reserve(1)
            .map_err(|_| NativeValueError::Allocation)?;
        elements.push((element, managed));
        Ok(())
    }

    /// Reads one List element and its validated managed flag.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, type, or bounds failure.
    pub fn list_get(
        &self,
        list: KeldValue,
        index: u64,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        let NativePayload::List { elements } = self.payload(list)? else {
            return Err(NativeValueError::TypeMismatch);
        };
        elements
            .get(usize::try_from(index).map_err(|_| NativeValueError::Bounds)?)
            .copied()
            .ok_or(NativeValueError::Bounds)
    }

    /// Replaces one List element and returns the displaced envelope.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, type, bounds, or allocation failure without
    /// committing the replacement.
    pub fn list_replace(
        &mut self,
        list: KeldValue,
        index: u64,
        element: KeldValue,
        managed: bool,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        if managed {
            self.validate_handle(element)?;
        }
        let (slot_index, generation) = decode_handle(list)?;
        let slot = self
            .handles
            .get_mut(slot_index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        let destination = elements
            .get_mut(usize::try_from(index).map_err(|_| NativeValueError::Bounds)?)
            .ok_or(NativeValueError::Bounds)?;
        Ok(std::mem::replace(destination, (element, managed)))
    }

    /// Removes one List element and closes the gap.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, type, or bounds failure without mutation.
    pub fn list_remove(
        &mut self,
        list: KeldValue,
        index: u64,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        let (slot_index, generation) = decode_handle(list)?;
        let slot = self
            .handles
            .get_mut(slot_index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        let index = usize::try_from(index).map_err(|_| NativeValueError::Bounds)?;
        if index >= elements.len() {
            return Err(NativeValueError::Bounds);
        }
        Ok(elements.remove(index))
    }

    /// Clears all List elements, recursively dropping managed children.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or type failure.
    pub fn list_clear(&mut self, list: KeldValue) -> Result<(), NativeValueError> {
        let (slot_index, generation) = decode_handle(list)?;
        let slot = self
            .handles
            .get_mut(slot_index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        let removed = std::mem::take(elements);
        for (element, managed) in removed {
            if managed {
                self.drop_managed(element)?;
            }
        }
        Ok(())
    }

    /// Deep-copies one managed value while preserving scalar envelopes.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or allocation failure; the source is unchanged.
    pub fn copy_managed(&mut self, value: KeldValue) -> Result<KeldValue, NativeValueError> {
        if value.words[0] == 0 {
            return Ok(value);
        }
        let payload = self.payload(value)?.clone();
        let copied = match payload {
            NativePayload::Text(bytes) => NativePayload::Text(bytes),
            NativePayload::Struct { definition, fields } => {
                let mut copied_fields = Vec::new();
                copied_fields
                    .try_reserve_exact(fields.len())
                    .map_err(|_| NativeValueError::Allocation)?;
                for (field, managed) in fields {
                    let field = if managed {
                        self.copy_managed(field)?
                    } else {
                        field
                    };
                    copied_fields.push((field, managed));
                }
                NativePayload::Struct {
                    definition,
                    fields: copied_fields,
                }
            }
            NativePayload::List { elements } => {
                let mut copied_elements = Vec::new();
                copied_elements
                    .try_reserve_exact(elements.len())
                    .map_err(|_| NativeValueError::Allocation)?;
                for (element, managed) in elements {
                    let element = if managed {
                        self.copy_managed(element)?
                    } else {
                        element
                    };
                    copied_elements.push((element, managed));
                }
                NativePayload::List {
                    elements: copied_elements,
                }
            }
        };
        self.allocate(copied)
    }

    /// Drops one managed handle and recursively releases owned children.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::InvalidHandle`] for stale or double drops.
    pub fn drop_managed(&mut self, value: KeldValue) -> Result<(), NativeValueError> {
        if value.words[0] == 0 {
            return Ok(());
        }
        let (index, generation) = decode_handle(value)?;
        let slot = self
            .handles
            .get_mut(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let payload = slot.payload.take().ok_or(NativeValueError::InvalidHandle)?;
        self.reusable_handles.push(index);
        match payload {
            NativePayload::Text(_) => {}
            NativePayload::Struct { fields, .. } | NativePayload::List { elements: fields } => {
                for (field, managed) in fields {
                    if managed {
                        self.drop_managed(field)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Moves one managed envelope and clears the source representation.
    #[must_use]
    pub fn take_value(source: &mut KeldValue) -> KeldValue {
        let value = *source;
        *source = KeldValue::default();
        value
    }

    fn allocate(&mut self, payload: NativePayload) -> Result<KeldValue, NativeValueError> {
        if let Some(index) = self.reusable_handles.pop() {
            let slot = self
                .handles
                .get_mut(index as usize)
                .ok_or(NativeValueError::InvalidHandle)?;
            slot.generation = slot
                .generation
                .checked_add(1)
                .ok_or(NativeValueError::Allocation)?;
            slot.payload = Some(payload);
            return Ok(encode_handle(index, slot.generation));
        }
        let index = u32::try_from(self.handles.len()).map_err(|_| NativeValueError::Allocation)?;
        self.handles
            .try_reserve(1)
            .map_err(|_| NativeValueError::Allocation)?;
        self.handles.push(HandleSlot {
            generation: 1,
            payload: Some(payload),
        });
        Ok(encode_handle(index, 1))
    }

    fn payload(&self, value: KeldValue) -> Result<&NativePayload, NativeValueError> {
        let (index, generation) = decode_handle(value)?;
        let slot = self
            .handles
            .get(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        slot.payload.as_ref().ok_or(NativeValueError::InvalidHandle)
    }

    fn validate_handle(&self, value: KeldValue) -> Result<(), NativeValueError> {
        self.payload(value).map(|_| ())
    }
}

fn encode_handle(index: u32, generation: u32) -> KeldValue {
    KeldValue {
        optional_some_layers: 0,
        reserved: 0,
        words: [
            KeldHandle((u64::from(generation) << 32) | u64::from(index + 1)).0,
            0,
            0,
        ],
    }
}

fn decode_handle(value: KeldValue) -> Result<(u32, u32), NativeValueError> {
    let raw = value.words[0];
    if raw == 0 {
        return Err(NativeValueError::InvalidHandle);
    }
    let index = u32::try_from(raw & u64::from(u32::MAX))
        .map_err(|_| NativeValueError::InvalidHandle)?
        .checked_sub(1)
        .ok_or(NativeValueError::InvalidHandle)?;
    let generation = u32::try_from(raw >> 32).map_err(|_| NativeValueError::InvalidHandle)?;
    Ok((index, generation))
}
