//! Safe Native-1 lowering boundary.

#![forbid(unsafe_code)]

use keld_ir::Module;

/// The backend's public entrypoint is intentionally introduced before any
/// lowering policy: it will accept only validated executable IR and metadata.
#[must_use]
pub const fn accepts_validated_module(_module: &Module) -> bool {
    true
}
