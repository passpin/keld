//! Safe, source-independent runtime core used by generated Native-1 programs.

#![forbid(unsafe_code)]

use keld_native_abi::{FaultKind, KeldFault, KeldLifecycle, RuntimeStatus};
use keld_runtime::{Store, StoreError};
use std::fmt;

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
}
