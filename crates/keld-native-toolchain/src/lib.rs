//! Validation helpers for the checksummed Native-1 toolchain.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::Path;

pub const LLVM_VERSION: &str = "22.1.8";
pub const LLVM_SYS_VERSION: &str = "221.0.1";
pub const TARGET_TRIPLE: &str = "x86_64-w64-windows-gnu";
pub const RUNTIME_DLL_NAME: &str = "keld_runtime_v1.dll";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolchainError {
    ArchiveChecksum {
        archive: String,
        expected: String,
        actual: String,
    },
    WrongLlvmVersion {
        expected: String,
        actual: String,
    },
    WrongGccTarget {
        expected: String,
        actual: String,
    },
    MissingRuntimeDll(String),
    MsvcFallback(String),
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArchiveChecksum {
                archive,
                expected,
                actual,
            } => write!(
                formatter,
                "checksum mismatch for {archive}: expected {expected}, got {actual}"
            ),
            Self::WrongLlvmVersion { expected, actual } => {
                write!(
                    formatter,
                    "LLVM version mismatch: expected {expected}, got {actual}"
                )
            }
            Self::WrongGccTarget { expected, actual } => write!(
                formatter,
                "GCC target mismatch: expected x86-64 MinGW ({expected}), got {actual}"
            ),
            Self::MissingRuntimeDll(path) => {
                write!(formatter, "missing runtime DLL {RUNTIME_DLL_NAME}: {path}")
            }
            Self::MsvcFallback(path) => {
                write!(formatter, "MSVC linker fallback is forbidden: {path}")
            }
        }
    }
}

impl std::error::Error for ToolchainError {}

/// Rejects any downloaded archive whose actual digest differs from its lock entry.
///
/// # Errors
///
/// Returns [`ToolchainError::ArchiveChecksum`] when the normalized digests do
/// not match.
pub fn validate_archive_checksum(
    archive: &str,
    expected: &str,
    actual: &str,
) -> Result<(), ToolchainError> {
    if expected.trim().eq_ignore_ascii_case(actual.trim()) {
        Ok(())
    } else {
        Err(ToolchainError::ArchiveChecksum {
            archive: archive.to_owned(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

/// Requires the exact LLVM release used to build the native backend.
///
/// # Errors
///
/// Returns [`ToolchainError::WrongLlvmVersion`] for any other version string.
pub fn validate_llvm_version(actual: &str) -> Result<(), ToolchainError> {
    if actual.trim() == LLVM_VERSION {
        Ok(())
    } else {
        Err(ToolchainError::WrongLlvmVersion {
            expected: LLVM_VERSION.to_owned(),
            actual: actual.trim().to_owned(),
        })
    }
}

/// Requires a 64-bit MinGW GCC target and rejects MSVC and 32-bit toolchains.
///
/// # Errors
///
/// Returns [`ToolchainError::WrongGccTarget`] when the target is not a
/// x86-64 MinGW target.
pub fn validate_gcc_target(actual: &str) -> Result<(), ToolchainError> {
    let normalized = actual.trim().to_ascii_lowercase();
    let valid = normalized.starts_with("x86_64-")
        && normalized.contains("mingw")
        && !normalized.contains("msvc");
    if valid {
        Ok(())
    } else {
        Err(ToolchainError::WrongGccTarget {
            expected: TARGET_TRIPLE.to_owned(),
            actual: actual.trim().to_owned(),
        })
    }
}

/// Requires a present, correctly named sibling runtime DLL.
///
/// # Errors
///
/// Returns [`ToolchainError::MissingRuntimeDll`] when the path is absent or
/// does not name the versioned runtime DLL.
pub fn require_runtime_dll(path: &Path) -> Result<(), ToolchainError> {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(RUNTIME_DLL_NAME));
    if named && path.is_file() {
        Ok(())
    } else {
        Err(ToolchainError::MissingRuntimeDll(
            path.display().to_string(),
        ))
    }
}

/// Explicitly rejects MSVC executables so no linker fallback can be inferred.
///
/// # Errors
///
/// Returns [`ToolchainError::MsvcFallback`] for `cl.exe`, `link.exe`,
/// `lld-link.exe`, or paths under an MSVC directory.
pub fn reject_msvc_fallback(path: &Path) -> Result<(), ToolchainError> {
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    let forbidden_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "cl.exe" | "link.exe" | "lld-link.exe"
            )
        });
    if forbidden_name || normalized.contains("\\msvc\\") || normalized.contains("/msvc/") {
        Err(ToolchainError::MsvcFallback(path.display().to_string()))
    } else {
        Ok(())
    }
}
