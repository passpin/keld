//! Safe Native-1 lowering boundary.

#![forbid(unsafe_code)]

use keld_ir::{Instruction, IrType, Module, Register, Terminator, validate};
use keld_native_abi::{RUNTIME_DLL_NAME, RUNTIME_IMPORT_LIBRARY_NAME};
use keld_native_llvm::{self, OptimizationLevel as LlvmOptimizationLevel};
use keld_source::{Diagnostic, SourceText};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const TARGET_TRIPLE: &str = "x86_64-w64-windows-gnu";
const LLVM_VERSION: &str = "22.1.8";
static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

/// Optimization levels frozen by the Native-1 CLI contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptimizationLevel {
    O0,
    O2,
}

/// Source bytes and rendered path used to attach metadata to a native build.
#[derive(Clone, Debug)]
pub struct SourceMetadata {
    pub path: PathBuf,
    pub source: SourceText,
}

/// Inputs required for one atomic native executable build.
#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub output: PathBuf,
    pub optimization: OptimizationLevel,
    pub llvm_prefix: PathBuf,
    pub gcc: Option<PathBuf>,
    pub runtime_dll: PathBuf,
    pub runtime_import_library: PathBuf,
}

/// Files published by a successful native build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeArtifact {
    pub executable: PathBuf,
    pub runtime_dll: PathBuf,
}

/// Errors are kept distinct so the CLI can report the failing boundary.
#[derive(Debug)]
pub enum BackendError {
    InvalidIr(Vec<Diagnostic>),
    Toolchain(String),
    Llvm(String),
    Linker(String),
    Io(std::io::Error),
    Unsupported(String),
}

impl BackendError {
    #[must_use]
    pub const fn is_invalid_ir(&self) -> bool {
        matches!(self, Self::InvalidIr(_))
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIr(diagnostics) => {
                write!(
                    formatter,
                    "native input IR is invalid ({} diagnostics)",
                    diagnostics.len()
                )
            }
            Self::Toolchain(message) => write!(formatter, "native toolchain error: {message}"),
            Self::Llvm(message) => write!(formatter, "LLVM error: {message}"),
            Self::Linker(message) => write!(formatter, "GNU linker error: {message}"),
            Self::Io(error) => write!(formatter, "native I/O error: {error}"),
            Self::Unsupported(message) => write!(formatter, "unsupported native IR: {message}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<std::io::Error> for BackendError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

struct StageGuard {
    path: PathBuf,
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn validate_llvm_prefix(prefix: &Path) -> Result<PathBuf, BackendError> {
    let config = prefix.join("bin").join("llvm-config.exe");
    let runtime = prefix.join("bin").join("libLLVM-22.dll");
    if !config.is_file() || !runtime.is_file() {
        return Err(BackendError::Toolchain(format!(
            "LLVM prefix is missing llvm-config.exe or libLLVM-22.dll: {}",
            prefix.display()
        )));
    }
    let output = Command::new(&config)
        .arg("--version")
        .output()
        .map_err(|error| BackendError::Toolchain(format!("unable to run llvm-config: {error}")))?;
    if !output.status.success() {
        return Err(BackendError::Toolchain(format!(
            "llvm-config --version failed with {}",
            output.status
        )));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version != LLVM_VERSION {
        return Err(BackendError::Toolchain(format!(
            "LLVM version mismatch: expected {LLVM_VERSION}, got {version}"
        )));
    }
    Ok(config)
}

fn resolve_gcc(explicit: Option<&Path>) -> Result<PathBuf, BackendError> {
    let candidate = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("gcc.exe"));
    let normalized = candidate.to_string_lossy().to_ascii_lowercase();
    let forbidden = ["cl.exe", "link.exe", "lld-link.exe"]
        .iter()
        .any(|name| normalized.ends_with(name))
        || normalized.contains("\\msvc\\")
        || normalized.contains("/msvc/");
    if forbidden {
        return Err(BackendError::Toolchain(format!(
            "MSVC linker fallback is forbidden: {}",
            candidate.display()
        )));
    }
    let output = Command::new(&candidate)
        .arg("-dumpmachine")
        .output()
        .map_err(|error| BackendError::Toolchain(format!("unable to run GCC: {error}")))?;
    if !output.status.success() {
        return Err(BackendError::Toolchain(format!(
            "GCC -dumpmachine failed with {}",
            output.status
        )));
    }
    let target = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    if !target.starts_with("x86_64-") || !target.contains("mingw") || target.contains("msvc") {
        return Err(BackendError::Toolchain(format!(
            "GCC target must be x86-64 MinGW, got {target}"
        )));
    }
    Ok(candidate)
}

fn require_runtime_artifacts(request: &BuildRequest) -> Result<(), BackendError> {
    let dll_name = request
        .runtime_dll
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !dll_name.eq_ignore_ascii_case(RUNTIME_DLL_NAME) || !request.runtime_dll.is_file() {
        return Err(BackendError::Toolchain(format!(
            "runtime DLL must be present and named {RUNTIME_DLL_NAME}: {}",
            request.runtime_dll.display()
        )));
    }
    let import_name = request
        .runtime_import_library
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !import_name.eq_ignore_ascii_case(RUNTIME_IMPORT_LIBRARY_NAME)
        || !request.runtime_import_library.is_file()
    {
        return Err(BackendError::Toolchain(format!(
            "runtime import library must be present and named {RUNTIME_IMPORT_LIBRARY_NAME}: {}",
            request.runtime_import_library.display()
        )));
    }
    Ok(())
}

fn const_return_value(module: &Module) -> Result<i64, BackendError> {
    let Some(function) = module
        .functions
        .iter()
        .find(|function| function.id == module.main)
    else {
        return Err(BackendError::Unsupported(
            "module main function is missing".to_owned(),
        ));
    };
    if module.functions.len() != 1
        || !function.parameters.is_empty()
        || function.return_type != IrType::Int
        || function.blocks.len() != 1
        || function.entry.0 != 0
    {
        return Err(BackendError::Unsupported(
            "task-3 backend accepts one parameterless Int function".to_owned(),
        ));
    }
    let block = &function.blocks[0];
    let mut constants = std::collections::BTreeMap::<Register, i64>::new();
    for instruction in &block.instructions {
        match instruction {
            Instruction::ConstInt { dst, value, .. } => {
                constants.insert(*dst, *value);
            }
            _ => {
                return Err(BackendError::Unsupported(
                    "only ConstInt instructions are implemented in task 3".to_owned(),
                ));
            }
        }
    }
    let Terminator::Return(Some(result)) = block.terminator else {
        return Err(BackendError::Unsupported(
            "task-3 backend requires Return(Some(Int))".to_owned(),
        ));
    };
    constants.get(&result).copied().ok_or_else(|| {
        BackendError::Unsupported("return register is not defined by ConstInt".to_owned())
    })
}

/// Builds one standalone GNU Windows executable from validated executable IR.
///
/// # Errors
///
/// Returns a distinct error when IR validation, the LLVM toolchain, LLVM
/// lowering, GNU linking, or filesystem staging fails.
#[allow(clippy::too_many_lines)]
pub fn build_executable(
    module: &Module,
    metadata: &SourceMetadata,
    request: &BuildRequest,
) -> Result<NativeArtifact, BackendError> {
    let diagnostics = validate(module);
    if !diagnostics.is_empty() {
        return Err(BackendError::InvalidIr(diagnostics));
    }
    if request.output.exists() {
        return Err(BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("refusing to overwrite {}", request.output.display()),
        )));
    }
    let parent = request.output.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("output directory does not exist: {}", parent.display()),
        )));
    }
    validate_llvm_prefix(&request.llvm_prefix)?;
    let gcc = resolve_gcc(request.gcc.as_deref())?;
    require_runtime_artifacts(request)?;
    let value = const_return_value(module)?;
    let stage_name = format!(
        ".keld-native-stage-{}-{}",
        std::process::id(),
        NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
    );
    let stage = parent.join(stage_name);
    std::fs::create_dir(&stage)?;
    let _guard = StageGuard {
        path: stage.clone(),
    };
    let object = stage.join("program.o");
    let staged_executable = stage.join("program.exe");
    let module_name = metadata
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("keld_program");
    let llvm_level = match request.optimization {
        OptimizationLevel::O0 => LlvmOptimizationLevel::O0,
        OptimizationLevel::O2 => LlvmOptimizationLevel::O2,
    };
    keld_native_llvm::emit_const_return_program(
        &object,
        module_name,
        value,
        TARGET_TRIPLE,
        llvm_level,
    )
    .map_err(|error| BackendError::Llvm(error.to_string()))?;
    let import_directory = request
        .runtime_import_library
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let linker = Command::new(&gcc)
        .arg(&object)
        .arg(format!("-L{}", import_directory.display()))
        .arg("-lkeld_runtime_v1")
        .arg("-mconsole")
        .arg("-static-libgcc")
        .arg("-o")
        .arg(&staged_executable)
        .output()
        .map_err(|error| BackendError::Linker(format!("unable to invoke GCC: {error}")))?;
    if !linker.status.success() {
        return Err(BackendError::Linker(format!(
            "GCC exited {}: {}{}",
            linker.status,
            String::from_utf8_lossy(&linker.stdout),
            String::from_utf8_lossy(&linker.stderr)
        )));
    }
    let runtime_destination = parent.join(RUNTIME_DLL_NAME);
    let runtime_created = if runtime_destination.is_file() {
        let existing = std::fs::read(&runtime_destination)?;
        let candidate = std::fs::read(&request.runtime_dll)?;
        if existing != candidate {
            return Err(BackendError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "sibling runtime DLL differs from {}",
                    request.runtime_dll.display()
                ),
            )));
        }
        false
    } else {
        let staged_runtime = stage.join(RUNTIME_DLL_NAME);
        std::fs::copy(&request.runtime_dll, &staged_runtime)?;
        std::fs::rename(&staged_runtime, &runtime_destination)?;
        true
    };
    if let Err(error) = std::fs::rename(&staged_executable, &request.output) {
        if runtime_created {
            let _ = std::fs::remove_file(&runtime_destination);
        }
        return Err(BackendError::Io(error));
    }
    Ok(NativeArtifact {
        executable: request.output.clone(),
        runtime_dll: runtime_destination,
    })
}

/// Small source-independent helper used by focused adapter tests.
#[must_use]
pub const fn accepts_validated_module(_module: &Module) -> bool {
    true
}
