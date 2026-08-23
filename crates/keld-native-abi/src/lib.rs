//! Frozen C-layout records shared by the Native-1 compiler and runtime.

#![forbid(unsafe_code)]

use core::fmt;

/// Native runtime ABI version.
pub const ABI_VERSION: u32 = 1;
/// Versioned runtime DLL name.
pub const RUNTIME_DLL_NAME: &str = "keld_runtime_v1.dll";
/// Versioned MinGW import library name.
pub const RUNTIME_IMPORT_LIBRARY_NAME: &str = "libkeld_runtime_v1.dll.a";
/// Prefix used by every exported runtime symbol.
pub const RUNTIME_EXPORT_PREFIX: &str = "keld_rt_v1_";

/// Version of the test-only semantic allocation control record format.
pub const TEST_CONTROL_SCHEMA_VERSION: u32 = 1;

/// Semantic allocation phases shared by the interpreter and native test
/// runtime. Engine-local allocations (frames, LLVM temporaries, place
/// metadata, and handle-table growth) deliberately do not appear here.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AllocationPhase {
    Context,
    Lifecycle,
    Text,
    Copy,
    Concat,
    Struct,
    Entity,
    Handle,
    ListGrowthPreferred,
    ListGrowthExact,
}

impl AllocationPhase {
    /// Stable phase spelling used by `KELD_TEST_CONTROL` and observations.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Lifecycle => "lifecycle",
            Self::Text => "text",
            Self::Copy => "copy",
            Self::Concat => "concat",
            Self::Struct => "struct",
            Self::Entity => "entity",
            Self::Handle => "handle",
            Self::ListGrowthPreferred => "list_growth_preferred",
            Self::ListGrowthExact => "list_growth_exact",
        }
    }

    const fn code(self) -> u32 {
        match self {
            Self::Context => 1,
            Self::Lifecycle => 2,
            Self::Text => 3,
            Self::Copy => 4,
            Self::Concat => 5,
            Self::Struct => 6,
            Self::Entity => 7,
            Self::Handle => 8,
            Self::ListGrowthPreferred => 9,
            Self::ListGrowthExact => 10,
        }
    }
}

/// Computes a deterministic semantic allocation site ID.
///
/// The high 16 bits identify the ordered instruction coordinate and the low
/// 16 bits identify the phase and structural-copy preorder ordinal. Base IDs
/// are one-based; ordinal zero denotes the instruction's outermost value.
/// `None` is returned when the frozen representation cannot encode the input.
#[must_use]
pub const fn checked_allocation_site_id(
    base_id: u32,
    phase: AllocationPhase,
    ordinal: u32,
) -> Option<u32> {
    if base_id == 0 || base_id > 65_535 || ordinal >= 4095 {
        return None;
    }
    let value = (base_id - 1) * 65_536 + phase.code() * 4_096 + ordinal + 1;
    if value == 0 { None } else { Some(value) }
}

/// Computes a site ID for valid frozen inputs.
///
/// # Panics
///
/// Panics if the base or structural ordinal exceeds the frozen encoding.
#[must_use]
pub const fn allocation_site_id(base_id: u32, phase: AllocationPhase, ordinal: u32) -> u32 {
    match checked_allocation_site_id(base_id, phase, ordinal) {
        Some(value) => value,
        None => panic!("allocation site exceeds the frozen Native-1 encoding"),
    }
}

/// A generational opaque runtime handle. Zero is the empty handle.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldHandle(pub u64);

impl KeldHandle {
    pub const EMPTY: Self = Self(0);

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A generational entity identity crossing the native boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldEntity {
    pub brand: u64,
    pub slot: u32,
    pub generation: u32,
    pub definition: u32,
    pub reserved: u32,
}

/// A weak link to an entity definition.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldLink {
    pub brand: u64,
    pub slot: u32,
    pub generation: u32,
    pub expected: u32,
    pub reserved: u32,
}

/// A lifecycle identity.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldLifecycle {
    pub brand: u64,
    pub index: u32,
    pub reserved: u32,
}

/// A fixed-size value envelope. Managed payloads are opaque handles in words.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldValue {
    pub optional_some_layers: u32,
    pub reserved: u32,
    pub words: [u64; 3],
}

/// A source location associated with a language fault.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldFault {
    pub kind: u32,
    pub location: u32,
}

/// One stack-bounded projection in a generated call place.
///
/// `kind` is `0` for a field projection and `1` for a list index. The value is
/// a field ordinal or a checked non-negative index respectively.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct KeldPlaceStep {
    pub kind: u32,
    pub reserved: u32,
    pub value: u64,
}

/// Status returned by every fallible runtime operation.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuntimeStatus {
    Ok = 0,
    LanguageFault = 1,
    InternalFailure = 2,
}

impl RuntimeStatus {
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
}

impl TryFrom<u32> for RuntimeStatus {
    type Error = InvalidDiscriminant;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::LanguageFault),
            2 => Ok(Self::InternalFailure),
            other => Err(InvalidDiscriminant(other)),
        }
    }
}

/// Frozen language-fault identifiers.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FaultKind {
    Arithmetic = 1,
    DivisionByZero = 2,
    Shift = 3,
    Allocation = 4,
    Capacity = 5,
    Bounds = 6,
}

impl TryFrom<u32> for FaultKind {
    type Error = InvalidDiscriminant;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Arithmetic),
            2 => Ok(Self::DivisionByZero),
            3 => Ok(Self::Shift),
            4 => Ok(Self::Allocation),
            5 => Ok(Self::Capacity),
            6 => Ok(Self::Bounds),
            other => Err(InvalidDiscriminant(other)),
        }
    }
}

/// An unknown numeric enum value received from an ABI boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InvalidDiscriminant(pub u32);

impl fmt::Display for InvalidDiscriminant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown native ABI discriminant {}", self.0)
    }
}

impl std::error::Error for InvalidDiscriminant {}
