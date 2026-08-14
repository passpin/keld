//! Safe, source-independent runtime core used by generated Native-1 programs.

#![forbid(unsafe_code)]

use keld_native_abi::{
    FaultKind, KeldEntity, KeldFault, KeldHandle, KeldLifecycle, KeldLink, KeldPlaceStep,
    KeldValue, RuntimeStatus,
};
use keld_runtime::{EntityId, Link, RuntimeLifecycleId, RuntimeTypeId, Store, StoreError};
#[cfg(feature = "test-controls")]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
#[cfg(feature = "test-controls")]
use std::fmt::Write as _;
#[cfg(feature = "test-controls")]
use std::path::PathBuf;

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

#[derive(Clone, Debug)]
struct NativeEntityPayload {
    definition: u32,
    fields: Vec<(KeldValue, bool)>,
}

/// Failure raised by an opaque native value operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeValueError {
    Allocation,
    Capacity,
    InvalidHandle,
    TypeMismatch,
    Bounds,
}

impl fmt::Display for NativeValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Allocation => "native value allocation failed",
            Self::Capacity => "native list capacity is impossible",
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
    store: Store<NativeEntityPayload>,
    handles: Vec<HandleSlot>,
    reusable_handles: Vec<u32>,
    status: RuntimeStatus,
    first_failure: Option<KeldFault>,
    #[cfg(feature = "test-controls")]
    test_controls: TestControls,
}

#[cfg(feature = "test-controls")]
#[derive(Clone, Debug, Default)]
struct TestControls {
    failures: BTreeSet<(u32, String, u64)>,
    attempts: BTreeMap<(u32, String), u64>,
    observations: Vec<TestAllocationObservation>,
    observation_path: Option<PathBuf>,
    current_site: u32,
}

#[cfg(feature = "test-controls")]
#[derive(Clone, Debug)]
struct TestAllocationObservation {
    site_id: u32,
    phase: String,
    attempt: u64,
    allowed: bool,
}

#[cfg(feature = "test-controls")]
impl TestControls {
    fn load() -> Self {
        let mut controls = Self {
            observation_path: std::env::var_os("KELD_TEST_OBSERVATION").map(PathBuf::from),
            ..Self::default()
        };
        let Some(path) = std::env::var_os("KELD_TEST_CONTROL").map(PathBuf::from) else {
            return controls;
        };
        let Ok(contents) = std::fs::read_to_string(path) else {
            return controls;
        };
        for line in contents.lines() {
            let Some(site_id) = parse_control_number(line, "site_id") else {
                continue;
            };
            let Some(attempt) = parse_control_u64(line, "attempt") else {
                continue;
            };
            let Some(phase) = parse_control_string(line, "phase") else {
                continue;
            };
            controls.failures.insert((site_id, phase, attempt));
        }
        controls
    }

    fn set_site(&mut self, site_id: u32) {
        self.current_site = site_id;
    }

    fn allow(&mut self, phase: &str) -> bool {
        let key = (self.current_site, phase.to_owned());
        let attempt = self
            .attempts
            .entry(key.clone())
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(1);
        let attempt = *attempt;
        let allowed = !self.failures.contains(&(key.0, key.1.clone(), attempt));
        self.observations.push(TestAllocationObservation {
            site_id: key.0,
            phase: key.1,
            attempt,
            allowed,
        });
        allowed
    }

    fn write_observation(&self, status: RuntimeStatus, fault: Option<KeldFault>) {
        let Some(path) = &self.observation_path else {
            return;
        };
        let mut output = String::from("version=1\n");
        let _ = writeln!(output, "status={}", status as u32);
        if let Some(fault) = fault {
            let _ = writeln!(
                output,
                "fault_kind={} fault_location={}",
                fault.kind, fault.location
            );
        }
        for observation in &self.observations {
            let _ = writeln!(
                output,
                "allocation site_id={} phase={} attempt={} allowed={}",
                observation.site_id,
                observation.phase,
                observation.attempt,
                u8::from(observation.allowed)
            );
        }
        let _ = std::fs::write(path, output);
    }
}

#[cfg(feature = "test-controls")]
fn parse_control_number(line: &str, key: &str) -> Option<u32> {
    parse_control_u64(line, key).and_then(|value| u32::try_from(value).ok())
}

#[cfg(feature = "test-controls")]
fn parse_control_u64(line: &str, key: &str) -> Option<u64> {
    let marker = format!("{key}=");
    let json_marker = format!("\"{key}\":");
    let start = line
        .find(&marker)
        .map(|index| index + marker.len())
        .or_else(|| {
            line.find(&json_marker)
                .map(|index| index + json_marker.len())
        })?;
    let digits = line[start..]
        .trim_start()
        .trim_start_matches(':')
        .trim_start()
        .trim_start_matches('"')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok()
}

#[cfg(feature = "test-controls")]
fn parse_control_string(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let json_marker = format!("\"{key}\":");
    let start = line
        .find(&marker)
        .map(|index| index + marker.len())
        .or_else(|| {
            line.find(&json_marker)
                .map(|index| index + json_marker.len())
        })?;
    let value = line[start..]
        .trim_start()
        .trim_start_matches(':')
        .trim_start()
        .trim_start_matches('"');
    let end = value
        .find(|character: char| character == '"' || character == ',' || character.is_whitespace())
        .unwrap_or(value.len());
    (!value[..end].is_empty()).then(|| value[..end].to_owned())
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
            #[cfg(feature = "test-controls")]
            test_controls: TestControls::load(),
        })
    }

    /// Selects the deterministic semantic-allocation site for the next
    /// synchronous runtime operation. This exists only in the test runtime
    /// and is deliberately absent from the production ABI.
    #[cfg(feature = "test-controls")]
    pub fn begin_test_operation(&mut self, site_id: u32) {
        self.test_controls.set_site(site_id);
    }

    #[cfg(feature = "test-controls")]
    fn allow_test_allocation(&mut self, phase: &str) -> bool {
        self.test_controls.allow(phase)
    }

    #[cfg(not(feature = "test-controls"))]
    fn allow_test_allocation(&mut self, _phase: &str) -> bool {
        true
    }

    /// Writes the optional test observation record before the context is
    /// dropped. Production contexts have no observation side effect.
    #[cfg(feature = "test-controls")]
    pub fn finish_test_observation(&self) {
        self.test_controls
            .write_observation(self.status, self.first_failure);
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

    fn store_error(error: StoreError) -> NativeValueError {
        match error {
            // Store growth is a semantic allocation point; preserve that
            // distinction so the FFI maps it to AllocationFault instead of
            // misclassifying it as a broken identity.
            StoreError::Allocation => NativeValueError::Allocation,
            StoreError::BrandExhausted | StoreError::InvalidOperation(_) => {
                NativeValueError::InvalidHandle
            }
        }
    }

    fn lifecycle_id(value: KeldLifecycle) -> RuntimeLifecycleId {
        RuntimeLifecycleId::from_raw_parts(value.brand, value.index)
    }

    fn entity_id(value: KeldEntity) -> EntityId {
        EntityId::from_raw_parts(value.brand, value.slot, value.generation, value.definition)
    }

    fn link_id(value: KeldLink) -> Link {
        Link::from_raw_parts(value.brand, value.slot, value.generation, value.expected)
    }

    fn entity_value(value: EntityId) -> KeldEntity {
        let (brand, slot, generation, definition) = value.raw_parts();
        KeldEntity {
            brand,
            slot,
            generation,
            definition,
            reserved: 0,
        }
    }

    fn link_value(value: Link) -> KeldLink {
        let (brand, slot, generation, expected) = value.raw_parts();
        KeldLink {
            brand,
            slot,
            generation,
            expected,
            reserved: 0,
        }
    }

    /// Starts a child lifecycle in the independent custody store.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign or inactive parent, or an
    /// allocation failure when the store cannot grow.
    pub fn begin_lifecycle(
        &mut self,
        parent: KeldLifecycle,
    ) -> Result<KeldLifecycle, NativeValueError> {
        if !self.allow_test_allocation("lifecycle") {
            return Err(NativeValueError::Allocation);
        }
        let lifecycle = self
            .store
            .begin_lifecycle(Self::lifecycle_id(parent))
            .map_err(Self::store_error)?;
        let (brand, index) = lifecycle.raw_parts();
        Ok(KeldLifecycle {
            brand,
            index,
            reserved: 0,
        })
    }

    /// Ends a child lifecycle and drops all managed fields in reverse custody
    /// order.
    ///
    /// # Errors
    ///
    /// Returns an internal failure when the lifecycle is foreign, inactive, or
    /// still has active children.
    pub fn end_lifecycle(&mut self, lifecycle: KeldLifecycle) -> Result<(), NativeValueError> {
        let mut payloads = Vec::new();
        self.store
            .end_lifecycle_with(Self::lifecycle_id(lifecycle), |payload| {
                payloads.push(payload);
            })
            .map_err(Self::store_error)?;
        for payload in payloads {
            self.drop_entity_payload(payload)?;
        }
        Ok(())
    }

    /// Allocates one entity payload in the supplied lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an allocation failure, a stale managed field, or an invalid
    /// lifecycle error.
    pub fn allocate_entity(
        &mut self,
        definition: u32,
        fields: &[KeldValue],
        managed: &[bool],
        lifecycle: KeldLifecycle,
    ) -> Result<KeldEntity, NativeValueError> {
        if fields.len() != managed.len() {
            return Err(NativeValueError::TypeMismatch);
        }
        for (field, is_managed) in fields.iter().copied().zip(managed.iter().copied()) {
            if is_managed {
                self.validate_handle(field)?;
            }
        }
        if !self.allow_test_allocation("entity") {
            return Err(NativeValueError::Allocation);
        }
        let entity = self
            .store
            .allocate(
                RuntimeTypeId(definition),
                Self::lifecycle_id(lifecycle),
                NativeEntityPayload {
                    definition,
                    fields: fields
                        .iter()
                        .copied()
                        .zip(managed.iter().copied())
                        .collect(),
                },
            )
            .map_err(Self::store_error)?;
        Ok(Self::entity_value(entity))
    }

    /// Converts a live entity into a weak link.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign or stale entity.
    pub fn entity_to_link(&self, entity: KeldEntity) -> Result<KeldLink, NativeValueError> {
        self.store
            .link(Self::entity_id(entity))
            .map(Self::link_value)
            .map_err(Self::store_error)
    }

    /// Resolves a weak link, returning `None` for absent or stale links.
    #[must_use]
    pub fn resolve_link(&self, link: KeldLink) -> Option<KeldEntity> {
        self.store
            .resolve(Self::link_id(link))
            .map(Self::entity_value)
    }

    /// Reads one entity field and its managed flag.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign or stale entity, or a bounds
    /// failure for an absent field.
    pub fn entity_field(
        &self,
        entity: KeldEntity,
        field: u32,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        self.store
            .read(Self::entity_id(entity), |payload| {
                let _ = payload.definition;
                payload
                    .fields
                    .get(field as usize)
                    .copied()
                    .ok_or(NativeValueError::Bounds)
            })
            .map_err(Self::store_error)?
    }

    /// Replaces one entity field and returns the displaced value.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign or stale entity, a bounds
    /// failure for an absent field, or a stale managed value.
    pub fn replace_entity_field(
        &mut self,
        entity: KeldEntity,
        field: u32,
        value: KeldValue,
        managed: bool,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        if managed {
            self.validate_handle(value)?;
        }
        self.store
            .edit(Self::entity_id(entity), |payload| {
                let destination = payload
                    .fields
                    .get_mut(field as usize)
                    .ok_or(NativeValueError::Bounds)?;
                Ok(std::mem::replace(destination, (value, managed)))
            })
            .map_err(Self::store_error)?
    }

    /// Moves entity custody to an ancestor lifecycle.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign entity, lifecycle, or invalid
    /// custody relation.
    pub fn keep_entity(
        &mut self,
        entity: KeldEntity,
        lifecycle: KeldLifecycle,
    ) -> Result<(), NativeValueError> {
        self.store
            .keep(Self::entity_id(entity), Self::lifecycle_id(lifecycle))
            .map_err(Self::store_error)
    }

    /// Retires an entity and drops its managed fields.
    ///
    /// # Errors
    ///
    /// Returns an internal failure for a foreign or stale entity.
    pub fn retire_entity(&mut self, entity: KeldEntity) -> Result<(), NativeValueError> {
        let mut payload = None;
        self.store
            .retire_with(Self::entity_id(entity), |value| payload = Some(value))
            .map_err(Self::store_error)?;
        if let Some(payload) = payload {
            self.drop_entity_payload(payload)?;
        }
        Ok(())
    }

    fn drop_entity_payload(
        &mut self,
        payload: NativeEntityPayload,
    ) -> Result<(), NativeValueError> {
        for (field, managed) in payload.fields {
            if managed {
                self.drop_managed(field)?;
            }
        }
        Ok(())
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
        if !self.allow_test_allocation("text") {
            return Err(NativeValueError::Allocation);
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
        let right = self.text_bytes(rhs)?.to_vec();
        let length = left
            .len()
            .checked_add(right.len())
            .ok_or(NativeValueError::Allocation)?;
        if !self.allow_test_allocation("concat") {
            return Err(NativeValueError::Allocation);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| NativeValueError::Allocation)?;
        bytes.extend_from_slice(&left);
        bytes.extend_from_slice(&right);
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

    /// Compares two immutable Text payloads by exact UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns a stale handle or type mismatch.
    pub fn text_equal(&self, lhs: KeldValue, rhs: KeldValue) -> Result<bool, NativeValueError> {
        Ok(self.text_bytes(lhs)? == self.text_bytes(rhs)?)
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
        if !self.allow_test_allocation("struct") {
            return Err(NativeValueError::Allocation);
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

    /// Resolves a stack-bounded field/index projection synchronously.
    ///
    /// Entity-rooted places consume a leading field step. The returned value
    /// is an envelope copy; no runtime pointer or projection slice is retained.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, invalid projection kind, type mismatch, or
    /// bounds failure.
    pub fn place_resolve(
        &self,
        root: KeldValue,
        entity: Option<KeldEntity>,
        steps: &[KeldPlaceStep],
    ) -> Result<KeldValue, NativeValueError> {
        let (mut current, remaining) = if let Some(entity) = entity {
            let Some(first) = steps.first() else {
                return Err(NativeValueError::TypeMismatch);
            };
            if first.kind != 0 {
                return Err(NativeValueError::TypeMismatch);
            }
            let field = u32::try_from(first.value).map_err(|_| NativeValueError::Bounds)?;
            (self.entity_field(entity, field)?.0, &steps[1..])
        } else {
            (root, steps)
        };
        for step in remaining {
            current = match step.kind {
                0 => {
                    let field = u32::try_from(step.value).map_err(|_| NativeValueError::Bounds)?;
                    self.struct_field(current, field)?.0
                }
                1 => self.list_get(current, step.value)?.0,
                _ => return Err(NativeValueError::TypeMismatch),
            };
        }
        Ok(current)
    }

    /// Replaces a projected field or list element and returns its displaced
    /// envelope. The projection is resolved and mutated during this call only.
    ///
    /// # Errors
    ///
    /// Returns a stale handle, invalid projection kind, type mismatch, or
    /// bounds failure. The destination is unchanged on any error.
    pub fn place_replace(
        &mut self,
        root: KeldValue,
        entity: Option<KeldEntity>,
        steps: &[KeldPlaceStep],
        value: KeldValue,
        managed: bool,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        if managed {
            self.validate_handle(value)?;
        }
        if let Some(entity) = entity {
            let Some(first) = steps.first() else {
                return Err(NativeValueError::TypeMismatch);
            };
            if first.kind != 0 {
                return Err(NativeValueError::TypeMismatch);
            }
            if steps.len() == 1 {
                let field = u32::try_from(first.value).map_err(|_| NativeValueError::Bounds)?;
                return self.replace_entity_field(entity, field, value, managed);
            }
            let field = u32::try_from(first.value).map_err(|_| NativeValueError::Bounds)?;
            let child = self.entity_field(entity, field)?.0;
            return self.replace_nested(child, &steps[1..], value, managed);
        }
        self.replace_nested(root, steps, value, managed)
    }

    fn replace_nested(
        &mut self,
        current: KeldValue,
        steps: &[KeldPlaceStep],
        value: KeldValue,
        managed: bool,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        let Some(first) = steps.first() else {
            return Err(NativeValueError::TypeMismatch);
        };
        match first.kind {
            0 => {
                if steps.len() == 1 {
                    let field = u32::try_from(first.value).map_err(|_| NativeValueError::Bounds)?;
                    self.replace_struct_field(current, field, value, managed)
                } else {
                    let field = u32::try_from(first.value).map_err(|_| NativeValueError::Bounds)?;
                    let child = self.struct_field(current, field)?.0;
                    self.replace_nested(child, &steps[1..], value, managed)
                }
            }
            1 => {
                if steps.len() == 1 {
                    self.list_replace(current, first.value, value, managed)
                } else {
                    let child = self.list_get(current, first.value)?.0;
                    self.replace_nested(child, &steps[1..], value, managed)
                }
            }
            _ => Err(NativeValueError::TypeMismatch),
        }
    }

    fn replace_struct_field(
        &mut self,
        value: KeldValue,
        field: u32,
        incoming: KeldValue,
        managed: bool,
    ) -> Result<(KeldValue, bool), NativeValueError> {
        if managed {
            self.validate_handle(incoming)?;
        }
        let (index, generation) = decode_handle(value)?;
        let slot = self
            .handles
            .get_mut(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::Struct { fields, .. } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        let destination = fields
            .get_mut(field as usize)
            .ok_or(NativeValueError::Bounds)?;
        Ok(std::mem::replace(destination, (incoming, managed)))
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
        let (length, capacity) = self.list_len_capacity(index, generation)?;
        if length == capacity {
            let required = length.checked_add(1).ok_or(NativeValueError::Capacity)?;
            let preferred = required.max(capacity.saturating_mul(2).max(4));
            let mut reserved = false;
            if self.allow_test_allocation("list_growth_preferred") {
                let slot = self
                    .handles
                    .get_mut(index as usize)
                    .ok_or(NativeValueError::InvalidHandle)?;
                let NativePayload::List { elements } = slot
                    .payload
                    .as_mut()
                    .ok_or(NativeValueError::InvalidHandle)?
                else {
                    return Err(NativeValueError::TypeMismatch);
                };
                reserved = elements
                    .try_reserve(preferred.saturating_sub(elements.len()))
                    .is_ok();
            }
            if !reserved
                && (preferred == required || !self.allow_test_allocation("list_growth_exact"))
            {
                return Err(NativeValueError::Allocation);
            }
            if !reserved {
                let slot = self
                    .handles
                    .get_mut(index as usize)
                    .ok_or(NativeValueError::InvalidHandle)?;
                let NativePayload::List { elements } = slot
                    .payload
                    .as_mut()
                    .ok_or(NativeValueError::InvalidHandle)?
                else {
                    return Err(NativeValueError::TypeMismatch);
                };
                if elements
                    .try_reserve(required.saturating_sub(elements.len()))
                    .is_err()
                {
                    return Err(NativeValueError::Allocation);
                }
            }
        }
        let slot = self
            .handles
            .get_mut(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
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

    /// Reserves additional List capacity using checked required-capacity
    /// arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`NativeValueError::Capacity`] when the required length cannot
    /// be represented, or [`NativeValueError::Allocation`] when allocation is
    /// rejected by the host allocator.
    pub fn list_reserve(
        &mut self,
        list: KeldValue,
        additional: i64,
    ) -> Result<(), NativeValueError> {
        let (slot_index, generation) = decode_handle(list)?;
        let additional = usize::try_from(additional).map_err(|_| NativeValueError::Capacity)?;
        let (length, capacity) = self.list_len_capacity(slot_index, generation)?;
        let required = length
            .checked_add(additional)
            .ok_or(NativeValueError::Capacity)?;
        if required <= capacity {
            return Ok(());
        }
        let preferred = required.max(capacity.saturating_mul(2).max(4));
        if self.allow_test_allocation("list_growth_preferred") {
            let slot = self
                .handles
                .get_mut(slot_index as usize)
                .ok_or(NativeValueError::InvalidHandle)?;
            let NativePayload::List { elements } = slot
                .payload
                .as_mut()
                .ok_or(NativeValueError::InvalidHandle)?
            else {
                return Err(NativeValueError::TypeMismatch);
            };
            if elements
                .try_reserve(preferred.saturating_sub(elements.len()))
                .is_ok()
            {
                return Ok(());
            }
        }
        if preferred == required || !self.allow_test_allocation("list_growth_exact") {
            return Err(NativeValueError::Allocation);
        }
        let slot = self
            .handles
            .get_mut(slot_index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        let NativePayload::List { elements } = slot
            .payload
            .as_mut()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        if elements
            .try_reserve(required.saturating_sub(elements.len()))
            .is_ok()
        {
            return Ok(());
        }
        Err(NativeValueError::Allocation)
    }

    /// Tries preferred then exact List growth without recording a fault.
    ///
    /// # Errors
    ///
    /// Returns handle/type/capacity errors. Allocation failures are reported
    /// as `Ok(false)` so callers can implement `try_reserve` semantics.
    pub fn list_try_reserve(
        &mut self,
        list: KeldValue,
        additional: i64,
    ) -> Result<bool, NativeValueError> {
        let (slot_index, generation) = decode_handle(list)?;
        // `try_reserve` is the non-faulting form: the interpreter returns
        // `false` for a negative or otherwise unrepresentable request rather
        // than turning the capacity check into a language fault. Handle and
        // type errors remain hard runtime failures and are returned below.
        let Ok(additional) = usize::try_from(additional) else {
            return Ok(false);
        };
        let (length, capacity) = self.list_len_capacity(slot_index, generation)?;
        let Some(required) = length.checked_add(additional) else {
            return Ok(false);
        };
        let Some(bytes) = required.checked_mul(std::mem::size_of::<(KeldValue, bool)>()) else {
            return Ok(false);
        };
        if bytes > isize::MAX as usize {
            return Ok(false);
        }
        if required <= capacity {
            return Ok(true);
        }
        let preferred = required.max(capacity.saturating_mul(2).max(4));
        if self.allow_test_allocation("list_growth_preferred") {
            let slot = self
                .handles
                .get_mut(slot_index as usize)
                .ok_or(NativeValueError::InvalidHandle)?;
            let NativePayload::List { elements } = slot
                .payload
                .as_mut()
                .ok_or(NativeValueError::InvalidHandle)?
            else {
                return Err(NativeValueError::TypeMismatch);
            };
            if elements
                .try_reserve(preferred.saturating_sub(elements.len()))
                .is_ok()
            {
                return Ok(true);
            }
        }
        if preferred != required && self.allow_test_allocation("list_growth_exact") {
            let slot = self
                .handles
                .get_mut(slot_index as usize)
                .ok_or(NativeValueError::InvalidHandle)?;
            let NativePayload::List { elements } = slot
                .payload
                .as_mut()
                .ok_or(NativeValueError::InvalidHandle)?
            else {
                return Err(NativeValueError::TypeMismatch);
            };
            if elements
                .try_reserve(required.saturating_sub(elements.len()))
                .is_ok()
            {
                return Ok(true);
            }
        }
        Ok(false)
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
        if !self.allow_test_allocation("copy") {
            return Err(NativeValueError::Allocation);
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
                        match self.copy_managed(field) {
                            Ok(field) => field,
                            Err(error) => {
                                for (copied, copied_managed) in copied_fields {
                                    if copied_managed {
                                        let _ = self.drop_managed(copied);
                                    }
                                }
                                return Err(error);
                            }
                        }
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
                        match self.copy_managed(element) {
                            Ok(element) => element,
                            Err(error) => {
                                for (copied, copied_managed) in copied_elements {
                                    if copied_managed {
                                        let _ = self.drop_managed(copied);
                                    }
                                }
                                return Err(error);
                            }
                        }
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
        if !self.allow_test_allocation("handle") {
            let _ = self.drop_payload(copied);
            return Err(NativeValueError::Allocation);
        }
        let mut result = self.allocate_payload(copied)?;
        result.optional_some_layers = value.optional_some_layers;
        Ok(result)
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
        self.drop_payload(payload)
    }

    /// Moves one managed envelope and clears the source representation.
    #[must_use]
    pub fn take_value(source: &mut KeldValue) -> KeldValue {
        let value = *source;
        *source = KeldValue::default();
        value
    }

    fn allocate(&mut self, payload: NativePayload) -> Result<KeldValue, NativeValueError> {
        if !self.allow_test_allocation("handle") {
            return Err(NativeValueError::Allocation);
        }
        self.allocate_payload(payload)
    }

    fn allocate_payload(&mut self, payload: NativePayload) -> Result<KeldValue, NativeValueError> {
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

    fn drop_payload(&mut self, payload: NativePayload) -> Result<(), NativeValueError> {
        match payload {
            NativePayload::Text(_) => Ok(()),
            NativePayload::Struct { fields, .. } | NativePayload::List { elements: fields } => {
                for (field, managed) in fields {
                    if managed {
                        self.drop_managed(field)?;
                    }
                }
                Ok(())
            }
        }
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

    fn list_len_capacity(
        &self,
        index: u32,
        generation: u32,
    ) -> Result<(usize, usize), NativeValueError> {
        let slot = self
            .handles
            .get(index as usize)
            .ok_or(NativeValueError::InvalidHandle)?;
        if slot.generation != generation {
            return Err(NativeValueError::InvalidHandle);
        }
        let NativePayload::List { elements } = slot
            .payload
            .as_ref()
            .ok_or(NativeValueError::InvalidHandle)?
        else {
            return Err(NativeValueError::TypeMismatch);
        };
        Ok((elements.len(), elements.capacity()))
    }

    fn validate_handle(&self, value: KeldValue) -> Result<(), NativeValueError> {
        if value.words[0] == 0 {
            Ok(())
        } else {
            self.payload(value).map(|_| ())
        }
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
