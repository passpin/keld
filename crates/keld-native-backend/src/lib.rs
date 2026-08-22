//! Safe Native-1 lowering boundary.

#![forbid(unsafe_code)]

use keld_ir::{
    AllocationSchedule, CompareOp, FaultKind as IrFaultKind, FunctionId, Instruction, IntBinaryOp,
    IntUnaryOp, IrBlockId, IrType, Module, ParameterIndex, Register, RegisterStorage, Terminator,
    validate,
};
use keld_native_abi::{ABI_VERSION, RUNTIME_DLL_NAME, RUNTIME_IMPORT_LIBRARY_NAME};
use keld_native_llvm::{self, OptimizationLevel as LlvmOptimizationLevel};
use keld_source::{BytePos, Diagnostic, SourceText, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const TARGET_TRIPLE: &str = "x86_64-w64-windows-gnu";
const LLVM_VERSION: &str = "22.1.8";
const VALUE_IR_TYPE: &str = "{ i32, i32, [3 x i64] }";
const PLACE_STEP_IR_TYPE: &str = "{ i32, i32, i64 }";
static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

/// One deterministic source location embedded in a native executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    pub id: u32,
    pub source_id: u32,
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub column: u32,
}

/// Deterministic location table shared by scalar lowering and fault rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocationTable {
    entries: Vec<SourceLocation>,
    ids: BTreeMap<(u32, u32, u32), u32>,
}

impl LocationTable {
    /// Builds IDs after deduplicating and sorting source spans.
    #[must_use]
    pub fn from_module(module: &Module, source: &SourceText) -> Self {
        let mut spans = BTreeSet::new();
        for function in &module.functions {
            spans.insert((
                function.span.source().0,
                function.span.start().0,
                function.span.end().0,
            ));
            for block in &function.blocks {
                for instruction in &block.instructions {
                    let span = instruction_span(instruction);
                    spans.insert((span.source().0, span.start().0, span.end().0));
                }
                if let Some(span) = terminator_span(&block.terminator) {
                    spans.insert((span.source().0, span.start().0, span.end().0));
                }
            }
        }
        let mut entries = Vec::with_capacity(spans.len());
        let mut ids = BTreeMap::new();
        for (index, (source_id, start, end)) in spans.into_iter().enumerate() {
            let id = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let (line, column) = if source.id().0 == source_id {
                source.line_col(BytePos(start)).unwrap_or((1, 1))
            } else {
                (1, 1)
            };
            ids.insert((source_id, start, end), id);
            entries.push(SourceLocation {
                id,
                source_id,
                start,
                end,
                line,
                column,
            });
        }
        Self { entries, ids }
    }

    #[must_use]
    pub fn entries(&self) -> &[SourceLocation] {
        &self.entries
    }

    #[must_use]
    pub fn id_for(&self, span: Span) -> u32 {
        self.ids
            .get(&(span.source().0, span.start().0, span.end().0))
            .copied()
            .unwrap_or(0)
    }
}

fn instruction_span(instruction: &Instruction) -> Span {
    match instruction {
        Instruction::ConstInt { span, .. }
        | Instruction::ConstBool { span, .. }
        | Instruction::ConstText { span, .. }
        | Instruction::ConstNoneLink { span, .. }
        | Instruction::Copy { span, .. }
        | Instruction::Take { span, .. }
        | Instruction::InstallHome { span, .. }
        | Instruction::MoveHome { span, .. }
        | Instruction::DropHome { span, .. }
        | Instruction::DropIfLive { span, .. }
        | Instruction::DropSlot { span, .. }
        | Instruction::CleanupTrackedScope { span, .. }
        | Instruction::ReplacePlace { span, .. }
        | Instruction::ReplaceField { span, .. }
        | Instruction::ListNew { span, .. }
        | Instruction::ListLength { span, .. }
        | Instruction::ListPush { span, .. }
        | Instruction::ListPushPlace { span, .. }
        | Instruction::ListRemove { span, .. }
        | Instruction::ListRemovePlace { span, .. }
        | Instruction::ListIndex { span, .. }
        | Instruction::ListGet { span, .. }
        | Instruction::ListReplace { span, .. }
        | Instruction::ListTryRemove { span, .. }
        | Instruction::ListClear { span, .. }
        | Instruction::ListReserve { span, .. }
        | Instruction::ListTryReserve { span, .. }
        | Instruction::TextByteLength { span, .. }
        | Instruction::TextIsEmpty { span, .. }
        | Instruction::TextConcat { span, .. }
        | Instruction::CheckedUnaryInt { span, .. }
        | Instruction::CheckedBinaryInt { span, .. }
        | Instruction::Not { span, .. }
        | Instruction::Compare { span, .. }
        | Instruction::Phi { span, .. }
        | Instruction::ConstructStruct { span, .. }
        | Instruction::ReadStructField { span, .. }
        | Instruction::BeginLifecycle { span, .. }
        | Instruction::EndLifecycle { span, .. }
        | Instruction::AllocateEntity { span, .. }
        | Instruction::EntityToLink { span, .. }
        | Instruction::OpenView { span, .. }
        | Instruction::ReadField { span, .. }
        | Instruction::WriteField { span, .. }
        | Instruction::CloseView { span, .. }
        | Instruction::KeepEntity { span, .. }
        | Instruction::RetireEntity { span, .. }
        | Instruction::Call { span, .. } => *span,
    }
}

fn terminator_span(terminator: &Terminator) -> Option<Span> {
    match terminator {
        Terminator::ResolveLink { span, .. } | Terminator::Fault { span, .. } => Some(*span),
        Terminator::Goto(_)
        | Terminator::Branch { .. }
        | Terminator::Return(_)
        | Terminator::Unreachable => None,
    }
}

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

fn runtime_dll_error(path: &Path) -> BackendError {
    BackendError::Toolchain(format!(
        "runtime DLL must be a valid x86-64 PE exporting keld_rt_v1_abi_version: {}",
        path.display()
    ))
}

fn read_pe_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let value = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([value[0], value[1]]))
}

fn read_pe_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn pe_rva_to_file_offset(
    bytes: &[u8],
    section_table: usize,
    section_count: u16,
    rva: u32,
) -> Option<usize> {
    for index in 0..usize::from(section_count) {
        let section = section_table.checked_add(index.checked_mul(40)?)?;
        let virtual_size = read_pe_u32(bytes, section.checked_add(8)?)?;
        let virtual_address = read_pe_u32(bytes, section.checked_add(12)?)?;
        let raw_size = read_pe_u32(bytes, section.checked_add(16)?)?;
        let raw_pointer = read_pe_u32(bytes, section.checked_add(20)?)?;
        let Some(delta) = rva.checked_sub(virtual_address) else {
            continue;
        };
        if delta >= virtual_size.max(raw_size) {
            continue;
        }
        let file_offset = raw_pointer.checked_add(delta)?;
        let file_offset = usize::try_from(file_offset).ok()?;
        if file_offset < bytes.len() {
            return Some(file_offset);
        }
    }
    None
}

fn validate_runtime_dll(path: &Path) -> Result<(), BackendError> {
    let bytes = std::fs::read(path)?;
    let valid_dos = bytes.get(0..2) == Some(b"MZ");
    let Some(pe_offset) = read_pe_u32(&bytes, 0x3c).and_then(|value| usize::try_from(value).ok())
    else {
        return Err(runtime_dll_error(path));
    };
    let valid_signature = bytes.get(pe_offset..pe_offset.saturating_add(4)) == Some(b"PE\0\0");
    let Some(coff) = pe_offset.checked_add(4) else {
        return Err(runtime_dll_error(path));
    };
    let valid_machine = read_pe_u16(&bytes, coff) == Some(0x8664);
    let Some(section_count) = read_pe_u16(&bytes, coff.saturating_add(2)) else {
        return Err(runtime_dll_error(path));
    };
    let Some(optional_size) = read_pe_u16(&bytes, coff.saturating_add(16)) else {
        return Err(runtime_dll_error(path));
    };
    let Some(optional) = coff.checked_add(20) else {
        return Err(runtime_dll_error(path));
    };
    let valid_optional = optional_size >= 120 && read_pe_u16(&bytes, optional) == Some(0x20b);
    let Some(export_rva) = read_pe_u32(&bytes, optional.saturating_add(112)) else {
        return Err(runtime_dll_error(path));
    };
    let Some(section_table) = optional.checked_add(usize::from(optional_size)) else {
        return Err(runtime_dll_error(path));
    };
    if !valid_dos || !valid_signature || !valid_machine || !valid_optional || export_rva == 0 {
        return Err(runtime_dll_error(path));
    }
    let Some(export_directory) =
        pe_rva_to_file_offset(&bytes, section_table, section_count, export_rva)
    else {
        return Err(runtime_dll_error(path));
    };
    let Some(names_rva) = read_pe_u32(&bytes, export_directory.saturating_add(32)) else {
        return Err(runtime_dll_error(path));
    };
    let Some(name_count) = read_pe_u32(&bytes, export_directory.saturating_add(24)) else {
        return Err(runtime_dll_error(path));
    };
    let Some(name_count) = usize::try_from(name_count).ok() else {
        return Err(runtime_dll_error(path));
    };
    if name_count > bytes.len() / 4 {
        return Err(runtime_dll_error(path));
    }
    let Some(names) = pe_rva_to_file_offset(&bytes, section_table, section_count, names_rva) else {
        return Err(runtime_dll_error(path));
    };
    for index in 0..name_count {
        let Some(name_entry) = names.checked_add(index.saturating_mul(4)) else {
            return Err(runtime_dll_error(path));
        };
        let Some(export_name_rva) = read_pe_u32(&bytes, name_entry) else {
            return Err(runtime_dll_error(path));
        };
        let Some(export_name_offset) =
            pe_rva_to_file_offset(&bytes, section_table, section_count, export_name_rva)
        else {
            return Err(runtime_dll_error(path));
        };
        let Some(end) = bytes.get(export_name_offset..).and_then(|tail| {
            tail.iter()
                .position(|byte| *byte == 0)
                .map(|offset| export_name_offset + offset)
        }) else {
            return Err(runtime_dll_error(path));
        };
        if bytes.get(export_name_offset..end) == Some(b"keld_rt_v1_abi_version") {
            return Ok(());
        }
    }
    Err(runtime_dll_error(path))
}

fn archive_member_size(header: &[u8]) -> Option<usize> {
    std::str::from_utf8(header.get(48..58)?)
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn visit_gnu_archive(bytes: &[u8], mut visit: impl FnMut(&[u8])) -> bool {
    if !bytes.starts_with(b"!<arch>\n") {
        return false;
    }
    let mut offset = 8;
    while offset < bytes.len() {
        let Some(header_end) = offset.checked_add(60) else {
            return false;
        };
        let Some(header) = bytes.get(offset..header_end) else {
            return false;
        };
        if header.get(58..60) != Some(b"`\n") {
            return false;
        }
        let Some(size) = archive_member_size(header) else {
            return false;
        };
        let Some(data_end) = header_end.checked_add(size) else {
            return false;
        };
        let Some(member) = bytes.get(header_end..data_end) else {
            return false;
        };
        visit(member);
        let Some(next) = data_end.checked_add(size % 2) else {
            return false;
        };
        offset = next;
    }
    offset == bytes.len()
}

fn coff_section_data<'bytes>(bytes: &'bytes [u8], wanted: &[u8]) -> Option<&'bytes [u8]> {
    if read_pe_u16(bytes, 0) != Some(0x8664) {
        return None;
    }
    let section_count = usize::from(read_pe_u16(bytes, 2)?);
    let optional_size = usize::from(read_pe_u16(bytes, 16)?);
    let section_table = 20usize.checked_add(optional_size)?;
    for index in 0..section_count {
        let section = section_table.checked_add(index.checked_mul(40)?)?;
        let name = bytes.get(section..section.checked_add(8)?)?;
        let name_matches = name
            .get(..wanted.len())
            .is_some_and(|prefix| prefix == wanted)
            && name
                .get(wanted.len()..)
                .is_some_and(|suffix| suffix.iter().all(|byte| *byte == 0));
        if !name_matches {
            continue;
        }
        let raw_size = usize::try_from(read_pe_u32(bytes, section.checked_add(16)?)?).ok()?;
        let raw_pointer = usize::try_from(read_pe_u32(bytes, section.checked_add(20)?)?).ok()?;
        return bytes.get(raw_pointer..raw_pointer.checked_add(raw_size)?);
    }
    None
}

fn visit_coff_symbols(bytes: &[u8], mut visit: impl FnMut(&[u8], i16) -> bool) -> bool {
    if read_pe_u16(bytes, 0) != Some(0x8664) {
        return false;
    }
    let Some(symbol_table) = read_pe_u32(bytes, 8).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(symbol_count) = read_pe_u32(bytes, 12).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(symbol_bytes) = symbol_count.checked_mul(18) else {
        return false;
    };
    let Some(string_table) = symbol_table.checked_add(symbol_bytes) else {
        return false;
    };
    let Some(string_table_size) =
        read_pe_u32(bytes, string_table).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(string_table_end) = string_table.checked_add(string_table_size) else {
        return false;
    };
    let Some(string_table) = bytes.get(string_table..string_table_end) else {
        return false;
    };
    let mut index = 0;
    while index < symbol_count {
        let Some(entry_offset) = index
            .checked_mul(18)
            .and_then(|offset| symbol_table.checked_add(offset))
        else {
            return false;
        };
        let Some(entry) = entry_offset
            .checked_add(18)
            .and_then(|end| bytes.get(entry_offset..end))
        else {
            return false;
        };
        let name = if entry.get(..4) == Some([0; 4].as_slice()) {
            let Some(string_offset) =
                read_pe_u32(entry, 4).and_then(|value| usize::try_from(value).ok())
            else {
                return false;
            };
            let Some(string) = string_table.get(string_offset..) else {
                return false;
            };
            let Some(end) = string.iter().position(|byte| *byte == 0) else {
                return false;
            };
            &string[..end]
        } else {
            let end = entry[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
            &entry[..end]
        };
        let Some(section) =
            read_pe_u16(entry, 12).map(|section| i16::from_ne_bytes(section.to_ne_bytes()))
        else {
            return false;
        };
        if visit(name, section) {
            return true;
        }
        let Some(auxiliary_count) = entry.get(17).map(|count| usize::from(*count)) else {
            return false;
        };
        let Some(next) = index.checked_add(auxiliary_count + 1) else {
            return false;
        };
        index = next;
    }
    true
}

fn coff_symbol_section(bytes: &[u8], wanted: &[u8]) -> Option<i16> {
    let mut section = None;
    let valid = visit_coff_symbols(bytes, |name, symbol_section| {
        if name == wanted {
            section = Some(symbol_section);
            true
        } else {
            false
        }
    });
    valid.then_some(section).flatten()
}

fn coff_symbol_matching(
    bytes: &[u8],
    mut matches: impl FnMut(&[u8], i16) -> bool,
) -> Option<Vec<u8>> {
    let mut head = None;
    let valid = visit_coff_symbols(bytes, |name, section| {
        if matches(name, section) {
            head = Some(name.to_vec());
            true
        } else {
            false
        }
    });
    valid.then_some(head).flatten()
}

fn coff_symbols_matching(
    bytes: &[u8],
    mut matches: impl FnMut(&[u8], i16) -> bool,
) -> Option<Vec<Vec<u8>>> {
    let mut symbols = Vec::new();
    let valid = visit_coff_symbols(bytes, |name, section| {
        if matches(name, section) {
            symbols.push(name.to_vec());
        }
        false
    });
    valid.then_some(symbols)
}

fn coff_head_symbol(bytes: &[u8]) -> Option<Vec<u8>> {
    coff_symbol_matching(bytes, |name, section| {
        section > 0 && name.starts_with(b"_head_")
    })
}

fn coff_import_name_symbol(bytes: &[u8]) -> Option<Vec<u8>> {
    coff_symbol_matching(bytes, |name, section| {
        section > 0 && name.ends_with(b"_iname")
    })
}

fn section_starts_with_c_string(section: &[u8], wanted: &[u8]) -> bool {
    section
        .get(..wanted.len())
        .is_some_and(|prefix| prefix == wanted)
        && section.get(wanted.len()) == Some(&0)
}

fn coff_import_symbol(bytes: &[u8]) -> Option<Vec<u8>> {
    let import_name = coff_section_data(bytes, b".idata$6")?.get(2..)?;
    let end = import_name.iter().position(|byte| *byte == 0)?;
    Some(import_name[..end].to_vec())
}

fn validate_runtime_import_library(path: &Path) -> Result<(), BackendError> {
    let bytes = std::fs::read(path)?;
    let abi_import = b"keld_rt_v1_abi_version";
    let mut import_name_symbol = None;
    let archive_valid = visit_gnu_archive(&bytes, |member| {
        let Some(dll_name) = coff_section_data(member, b".idata$7") else {
            return;
        };
        if section_starts_with_c_string(dll_name, RUNTIME_DLL_NAME.as_bytes()) {
            import_name_symbol = coff_import_name_symbol(member);
        }
    });
    let Some(import_name_symbol) = import_name_symbol.as_deref() else {
        return Err(BackendError::Toolchain(format!(
            "runtime import library must contain a COFF import for {RUNTIME_DLL_NAME} and keld_rt_v1_abi_version: {}",
            path.display()
        )));
    };
    let mut head_members_valid = true;
    let mut approved_head_count = 0;
    let mut head_symbol = None;
    let head_archive_valid = visit_gnu_archive(&bytes, |member| {
        if coff_section_data(member, b".idata$2").is_none() {
            return;
        }
        let Some(candidate) = coff_head_symbol(member) else {
            head_members_valid = false;
            return;
        };
        if coff_symbol_section(member, import_name_symbol) == Some(0) {
            approved_head_count += 1;
            head_symbol = Some(candidate);
        } else {
            head_members_valid = false;
        }
    });
    let Some(head_symbol) = head_symbol.as_deref() else {
        return Err(BackendError::Toolchain(format!(
            "runtime import library must contain a COFF import for {RUNTIME_DLL_NAME} and keld_rt_v1_abi_version: {}",
            path.display()
        )));
    };
    if !head_members_valid || approved_head_count != 1 {
        return Err(BackendError::Toolchain(format!(
            "runtime import library must contain a COFF import for {RUNTIME_DLL_NAME} and keld_rt_v1_abi_version: {}",
            path.display()
        )));
    }
    let mut found_abi_import = false;
    let mut runtime_imports_valid = true;
    let import_archive_valid = visit_gnu_archive(&bytes, |member| {
        if coff_section_data(member, b".idata$6").is_none() {
            return;
        }
        let Some(import_name) = coff_import_symbol(member) else {
            runtime_imports_valid = false;
            return;
        };
        let Some(head_references) = coff_symbols_matching(member, |name, section| {
            section == 0 && name.starts_with(b"_head_")
        }) else {
            runtime_imports_valid = false;
            return;
        };
        let head_matches =
            head_references.len() == 1 && head_references[0].as_slice() == head_symbol;
        let import_defined =
            coff_symbol_section(member, &import_name).is_some_and(|section| section > 0);
        let mut import_pointer = b"__imp_".to_vec();
        import_pointer.extend_from_slice(&import_name);
        let import_pointer_defined =
            coff_symbol_section(member, &import_pointer).is_some_and(|section| section > 0);
        if !head_matches || !import_defined || !import_pointer_defined {
            runtime_imports_valid = false;
        }
        if import_name == abi_import && import_defined && import_pointer_defined {
            found_abi_import = true;
        }
    });
    if !archive_valid
        || !head_archive_valid
        || !import_archive_valid
        || !runtime_imports_valid
        || !found_abi_import
    {
        return Err(BackendError::Toolchain(format!(
            "runtime import library must contain a COFF import for {RUNTIME_DLL_NAME} and keld_rt_v1_abi_version: {}",
            path.display()
        )));
    }
    Ok(())
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
    validate_runtime_dll(&request.runtime_dll)?;
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
    validate_runtime_import_library(&request.runtime_import_library)?;
    Ok(())
}

fn scalar_type(ty: &IrType) -> Option<&'static str> {
    match ty {
        IrType::Bool => Some("i1"),
        IrType::Int => Some("i64"),
        IrType::Lifecycle => Some("{ i64, i32, i32 }"),
        IrType::Unit
        | IrType::Struct(_)
        | IrType::Entity(_)
        | IrType::Link { .. }
        | IrType::Text
        | IrType::List(_)
        | IrType::Optional(_) => None,
    }
}

fn is_managed_type(ty: &IrType) -> bool {
    matches!(
        ty,
        IrType::Struct(_)
            | IrType::Entity(_)
            | IrType::Link { .. }
            | IrType::Text
            | IrType::List(_)
            | IrType::Optional(_)
    )
}

fn owns_native_payload(ty: &IrType) -> bool {
    match ty {
        IrType::Struct(_) | IrType::Text | IrType::List(_) => true,
        IrType::Optional(inner) => owns_native_payload(inner),
        IrType::Unit
        | IrType::Bool
        | IrType::Int
        | IrType::Entity(_)
        | IrType::Link { .. }
        | IrType::Lifecycle => false,
    }
}

fn value_type(ty: &IrType) -> Option<&'static str> {
    scalar_type(ty).or_else(|| is_managed_type(ty).then_some(VALUE_IR_TYPE))
}

fn ir_fault_code(kind: IrFaultKind) -> u32 {
    match kind {
        IrFaultKind::Arithmetic => 1,
        IrFaultKind::DivisionByZero => 2,
        IrFaultKind::Shift => 3,
        IrFaultKind::Allocation => 4,
        IrFaultKind::Capacity => 5,
    }
}

fn llvm_compare(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "eq",
        CompareOp::NotEq => "ne",
        CompareOp::Less => "slt",
        CompareOp::LessEq => "sle",
        CompareOp::Greater => "sgt",
        CompareOp::GreaterEq => "sge",
    }
}

fn llvm_type_for_register(
    function: &keld_ir::Function,
    register: Register,
) -> Result<&'static str, BackendError> {
    function
        .register_types
        .get(register.0 as usize)
        .and_then(scalar_type)
        .ok_or_else(|| {
            BackendError::Unsupported(format!(
                "register %{} is not a supported scalar value",
                register.0
            ))
        })
}

fn register_name(register: Register) -> String {
    format!("%r{}", register.0)
}

fn managed_slot_name(function: &keld_ir::Function, register: Register) -> String {
    if function.parameters.contains(&register)
        && function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    {
        register_name(register)
    } else {
        format!("%slot{}", register.0)
    }
}

fn escape_llvm_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for byte in bytes {
        match byte {
            b' '..=b'!' | b'#'..=b'[' | b']'..=b'~' if *byte != b'\\' && *byte != b'"' => {
                escaped.push(char::from(*byte));
            }
            _ => {
                let _ = write!(escaped, "\\{byte:02X}");
            }
        }
    }
    escaped
}

struct ScalarLowerer<'module> {
    module: &'module Module,
    function: &'module keld_ir::Function,
    locations: &'module LocationTable,
    allocation_schedule: &'module AllocationSchedule,
    lines: Vec<String>,
    current_label: String,
    faults: BTreeSet<(u32, u32)>,
    text_literals: BTreeMap<String, Vec<u8>>,
    view_entities: BTreeMap<keld_ir::ViewId, Register>,
    block_exit_labels: BTreeMap<keld_ir::IrBlockId, String>,
    home_trackers: BTreeMap<u32, HomeTracker>,
    slots_emitted: bool,
}

#[derive(Clone, Debug)]
struct HomeTracker {
    ids: String,
    count: String,
    slots: String,
    capacity: u32,
    slot_count: u32,
}

impl<'module> ScalarLowerer<'module> {
    fn new(
        module: &'module Module,
        function: &'module keld_ir::Function,
        locations: &'module LocationTable,
        allocation_schedule: &'module AllocationSchedule,
    ) -> Self {
        Self {
            module,
            function,
            locations,
            allocation_schedule,
            lines: Vec::new(),
            current_label: String::new(),
            faults: BTreeSet::new(),
            text_literals: BTreeMap::new(),
            view_entities: BTreeMap::new(),
            block_exit_labels: function
                .blocks
                .iter()
                .map(|block| (block.id, format!("exit_bb{}", block.id.0)))
                .collect(),
            home_trackers: BTreeMap::new(),
            slots_emitted: false,
        }
    }

    fn value_name(&self, register: Register) -> String {
        self.function
            .register_types
            .get(register.0 as usize)
            .filter(|ty| is_managed_type(ty))
            .map_or_else(
                || register_name(register),
                |_: &IrType| managed_slot_name(self.function, register),
            )
    }

    fn value_type(&self, register: Register) -> Result<&'static str, BackendError> {
        self.function
            .register_types
            .get(register.0 as usize)
            .and_then(value_type)
            .ok_or_else(|| {
                BackendError::Unsupported(format!(
                    "register %{} has no native value representation",
                    register.0
                ))
            })
    }

    fn is_managed_register(&self, register: Register) -> bool {
        self.function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    }

    fn is_home_register(&self, register: Register) -> bool {
        matches!(
            self.function.register_storage.get(register.0 as usize),
            Some(keld_ir::RegisterStorage::Home { .. })
        )
    }

    fn owns_register(&self, register: Register) -> bool {
        self.function
            .register_types
            .get(register.0 as usize)
            .is_some_and(owns_native_payload)
    }

    fn tracker_for_register(&self, register: Register) -> Option<&HomeTracker> {
        let RegisterStorage::Home { scope, .. } =
            self.function.register_storage.get(register.0 as usize)?
        else {
            return None;
        };
        self.home_trackers.get(&scope.0)
    }

    fn is_tracked_home(&self, register: Register) -> bool {
        self.owns_register(register) && self.tracker_for_register(register).is_some()
    }

    fn emit_home_track(&mut self, register: Register) -> Result<(), BackendError> {
        if !self.is_tracked_home(register) {
            return Ok(());
        }
        let tracker = self
            .tracker_for_register(register)
            .cloned()
            .ok_or_else(|| {
                BackendError::Unsupported("managed Home tracker is missing".to_owned())
            })?;
        let status = format!("%home_track_status_{}", self.lines.len());
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_home_track(ptr {}, ptr {}, i32 {}, i32 {})",
            tracker.ids, tracker.count, register.0, tracker.capacity
        ));
        self.emit_runtime_status(&status, format!("home_track_cont_{}", self.lines.len()));
        Ok(())
    }

    fn emit_home_untrack(&mut self, register: Register) -> Result<(), BackendError> {
        if !self.is_tracked_home(register) {
            return Ok(());
        }
        let tracker = self
            .tracker_for_register(register)
            .cloned()
            .ok_or_else(|| {
                BackendError::Unsupported("managed Home tracker is missing".to_owned())
            })?;
        let status = format!("%home_untrack_status_{}", self.lines.len());
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_home_untrack(ptr {}, ptr {}, i32 {}, i32 {})",
            tracker.ids, tracker.count, register.0, tracker.capacity
        ));
        self.emit_runtime_status(&status, format!("home_untrack_cont_{}", self.lines.len()));
        Ok(())
    }

    fn emit_cleanup_scope(&mut self, scope: u32, span: Span) {
        let Some(tracker) = self.home_trackers.get(&scope).cloned() else {
            return;
        };
        let status = format!("%scope_cleanup_status_{}", self.lines.len());
        let location = self.locations.id_for(span);
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_cleanup_scope(ptr %context, ptr {}, ptr {}, ptr {}, i32 {}, i32 {}, i32 {location})",
            tracker.ids, tracker.count, tracker.slots, tracker.capacity, tracker.slot_count
        ));
        self.emit_runtime_status(&status, format!("scope_cleanup_cont_{}", self.lines.len()));
    }

    fn list_element_type(&self, list: Register) -> Result<&IrType, BackendError> {
        match self.function.register_types.get(list.0 as usize) {
            Some(IrType::List(element)) => Ok(element),
            _ => Err(BackendError::Unsupported(
                "List instruction receiver is not a List".to_owned(),
            )),
        }
    }

    fn definition_field_index(
        &self,
        definition: keld_ir::DefId,
        field: keld_ir::FieldId,
    ) -> Result<usize, BackendError> {
        let definition = self
            .module
            .definitions
            .iter()
            .find(|candidate| candidate.id == definition)
            .ok_or_else(|| BackendError::Unsupported("struct definition is missing".to_owned()))?;
        definition
            .fields
            .iter()
            .position(|(candidate, _)| *candidate == field)
            .ok_or_else(|| BackendError::Unsupported("struct field is missing".to_owned()))
    }

    fn definition_field_type(
        &self,
        definition: keld_ir::DefId,
        field: keld_ir::FieldId,
    ) -> Result<&IrType, BackendError> {
        let definition = self
            .module
            .definitions
            .iter()
            .find(|candidate| candidate.id == definition)
            .ok_or_else(|| BackendError::Unsupported("definition is missing".to_owned()))?;
        definition
            .fields
            .iter()
            .find_map(|(candidate, ty)| (*candidate == field).then_some(ty))
            .ok_or_else(|| BackendError::Unsupported("definition field is missing".to_owned()))
    }

    fn entity_register_definition(&self, entity: Register) -> Result<keld_ir::DefId, BackendError> {
        match self.function.register_types.get(entity.0 as usize) {
            Some(IrType::Entity(definition)) => Ok(*definition),
            _ => Err(BackendError::Unsupported(
                "entity instruction operand is not an Entity".to_owned(),
            )),
        }
    }

    fn lifecycle_pointer(&mut self, register: Register) -> String {
        let slot = format!("%lifecycle_argument_{}", self.lines.len());
        self.line(format!("{slot} = alloca {{ i64, i32, i32 }}"));
        self.line(format!(
            "store {{ i64, i32, i32 }} {}, ptr {slot}",
            register_name(register)
        ));
        slot
    }

    fn emit_value_argument(&mut self, register: Register) -> Result<(String, bool), BackendError> {
        if self.is_managed_register(register) {
            let owns = self
                .function
                .register_types
                .get(register.0 as usize)
                .is_some_and(owns_native_payload);
            return Ok((self.value_name(register), owns));
        }
        let ty = self.value_type(register)?;
        let slot = format!("%list_argument_{}.slot", self.lines.len());
        self.line(format!("{slot} = alloca {VALUE_IR_TYPE}"));
        match ty {
            "i64" => self.line(format!(
                "%list_argument_{}.value = insertvalue {VALUE_IR_TYPE} zeroinitializer, i64 {}, 2, 0",
                self.lines.len(),
                register_name(register)
            )),
            "i1" => {
                let widened = format!("%list_argument_{}.widened", self.lines.len());
                self.line(format!("{widened} = zext i1 {} to i64", register_name(register)));
                self.line(format!(
                    "%list_argument_{}.value = insertvalue {VALUE_IR_TYPE} zeroinitializer, i64 {widened}, 2, 0",
                    self.lines.len()
                ));
            }
            _ => {
                return Err(BackendError::Unsupported(
                    "List element has no scalar envelope representation".to_owned(),
                ));
            }
        }
        let value = format!(
            "%list_argument_{}.value",
            self.lines.len().saturating_sub(1)
        );
        self.line(format!("store {VALUE_IR_TYPE} {value}, ptr {slot}"));
        Ok((slot, false))
    }

    fn emit_value_result(
        &mut self,
        destination: Register,
        output: &str,
    ) -> Result<(), BackendError> {
        if self.is_managed_register(destination) {
            let value = format!("%list_result_{}", self.lines.len());
            self.line(format!("{value} = load {VALUE_IR_TYPE}, ptr {output}"));
            self.line(format!(
                "store {VALUE_IR_TYPE} {value}, ptr {}",
                self.value_name(destination)
            ));
            return Ok(());
        }
        let ty = self.value_type(destination)?;
        let value = format!("%list_result_{}", self.lines.len());
        self.line(format!("{value} = load {VALUE_IR_TYPE}, ptr {output}"));
        let word = format!("{value}.word");
        self.line(format!(
            "{word} = extractvalue {VALUE_IR_TYPE} {value}, 2, 0"
        ));
        match ty {
            "i64" => self.line(format!(
                "{} = add i64 0, {word}",
                register_name(destination)
            )),
            "i1" => self.line(format!(
                "{} = icmp ne i64 {word}, 0",
                register_name(destination)
            )),
            _ => {
                return Err(BackendError::Unsupported(
                    "List result has no scalar envelope representation".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn emit_place_pointer(
        &mut self,
        source: &keld_ir::ArgumentSource,
        span: Span,
    ) -> Result<String, BackendError> {
        if source.projections.is_empty() {
            return Ok(self.value_name(source.base));
        }
        let entity_root = self
            .function
            .register_types
            .get(source.base.0 as usize)
            .is_some_and(|ty| matches!(ty, IrType::Entity(_)));
        if !entity_root && !self.is_managed_register(source.base) {
            return Err(BackendError::Unsupported(
                "projected place root is not managed".to_owned(),
            ));
        }
        let count = source.projections.len();
        let steps = format!("%place_steps_{}", self.lines.len());
        self.line(format!("{steps} = alloca [{count} x {PLACE_STEP_IR_TYPE}]"));
        for (index, projection) in source.projections.iter().enumerate() {
            let pointer = format!("%place_step_{}_{}", self.lines.len(), index);
            self.line(format!(
                "{pointer} = getelementptr inbounds [{count} x {PLACE_STEP_IR_TYPE}], ptr {steps}, i64 0, i64 {index}"
            ));
            match projection {
                keld_ir::ArgumentProjection::Field(field) => self.line(format!(
                    "store {PLACE_STEP_IR_TYPE} {{ i32 0, i32 0, i64 {} }}, ptr {pointer}",
                    field.0
                )),
                keld_ir::ArgumentProjection::Index(index_register) => {
                    if !matches!(
                        self.function.register_types.get(index_register.0 as usize),
                        Some(IrType::Int)
                    ) {
                        return Err(BackendError::Unsupported(
                            "projected place index is not Int".to_owned(),
                        ));
                    }
                    let step_value = format!("%place_step_value_{}", self.lines.len());
                    let step_index = format!("%place_step_index_{}", self.lines.len());
                    self.line(format!(
                        "{step_value} = insertvalue {PLACE_STEP_IR_TYPE} zeroinitializer, i32 1, 0"
                    ));
                    self.line(format!(
                        "{step_index} = insertvalue {PLACE_STEP_IR_TYPE} {step_value}, i64 {}, 2",
                        register_name(*index_register)
                    ));
                    self.line(format!(
                        "store {PLACE_STEP_IR_TYPE} {step_index}, ptr {pointer}"
                    ));
                }
            }
        }
        let output = format!("%place_value_{}.slot", self.lines.len());
        let status = format!("%place_resolve_status_{}", self.lines.len());
        let location = self.locations.id_for(span);
        self.line(format!("{output} = alloca {VALUE_IR_TYPE}"));
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_place_resolve(ptr %context, ptr {}, i8 {}, ptr {steps}, i32 {count}, ptr {output}, i32 {location})",
            self.value_name(source.base),
            u8::from(entity_root),
        ));
        self.emit_runtime_status(&status, format!("place_resolve_cont_{}", self.lines.len()));
        Ok(output)
    }

    fn emit_receiver_list_pointer(
        &mut self,
        receiver: &keld_ir::Receiver,
        span: Span,
    ) -> Result<String, BackendError> {
        if let Some(source) = receiver.source.as_ref() {
            self.emit_place_pointer(source, span)
        } else {
            Ok(self.value_name(receiver.list))
        }
    }

    fn emit_place_replace(
        &mut self,
        destination: &keld_ir::ArgumentSource,
        source: Register,
        displaced: Register,
        span: Span,
        block: IrBlockId,
        instruction_index: u32,
    ) -> Result<(), BackendError> {
        if destination.projections.is_empty() {
            return self.emit_instruction(
                &Instruction::InstallHome {
                    destination: destination.base,
                    source,
                    displaced,
                    span,
                },
                block,
                instruction_index,
            );
        }
        let entity_root = self
            .function
            .register_types
            .get(destination.base.0 as usize)
            .is_some_and(|ty| matches!(ty, IrType::Entity(_)));
        let count = destination.projections.len();
        let steps = format!("%place_replace_steps_{}", self.lines.len());
        self.line(format!("{steps} = alloca [{count} x {PLACE_STEP_IR_TYPE}]"));
        for (index, projection) in destination.projections.iter().enumerate() {
            let pointer = format!("%place_replace_step_{}_{}", self.lines.len(), index);
            self.line(format!(
                "{pointer} = getelementptr inbounds [{count} x {PLACE_STEP_IR_TYPE}], ptr {steps}, i64 0, i64 {index}"
            ));
            match projection {
                keld_ir::ArgumentProjection::Field(field) => self.line(format!(
                    "store {PLACE_STEP_IR_TYPE} {{ i32 0, i32 0, i64 {} }}, ptr {pointer}",
                    field.0
                )),
                keld_ir::ArgumentProjection::Index(index_register) => {
                    if !matches!(
                        self.function.register_types.get(index_register.0 as usize),
                        Some(IrType::Int)
                    ) {
                        return Err(BackendError::Unsupported(
                            "projected place index is not Int".to_owned(),
                        ));
                    }
                    let step_value = format!("%place_replace_step_value_{}", self.lines.len());
                    let step_index = format!("%place_replace_step_index_{}", self.lines.len());
                    self.line(format!(
                        "{step_value} = insertvalue {PLACE_STEP_IR_TYPE} zeroinitializer, i32 1, 0"
                    ));
                    self.line(format!(
                        "{step_index} = insertvalue {PLACE_STEP_IR_TYPE} {step_value}, i64 {}, 2",
                        register_name(*index_register)
                    ));
                    self.line(format!(
                        "store {PLACE_STEP_IR_TYPE} {step_index}, ptr {pointer}"
                    ));
                }
            }
        }
        let output = format!("%place_replace_{}.slot", self.lines.len());
        let status = format!("%place_replace_status_{}", self.lines.len());
        let location = self.locations.id_for(span);
        let (value_pointer, managed) = self.emit_value_argument(source)?;
        self.line(format!("{output} = alloca {VALUE_IR_TYPE}"));
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_place_replace(ptr %context, ptr {}, i8 {}, ptr {steps}, i32 {count}, ptr {value_pointer}, ptr {output}, i8 {}, i32 {location})",
            self.value_name(destination.base),
            u8::from(entity_root),
            u8::from(managed),
        ));
        self.emit_runtime_status(&status, format!("place_replace_cont_{}", self.lines.len()));
        self.emit_value_result(displaced, &output)?;
        if managed {
            self.emit_home_untrack(source)?;
            self.line(format!(
                "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                self.value_name(source)
            ));
        }
        Ok(())
    }

    fn emit_list_read(
        &mut self,
        destination: Register,
        receiver: &keld_ir::Receiver,
        index: Register,
        span: Span,
        copy: bool,
    ) -> Result<(), BackendError> {
        let element = self.list_element_type(receiver.list)?;
        let managed = copy && owns_native_payload(element);
        let flags = u8::from(managed) | (u8::from(copy) << 1);
        let list_pointer = self.emit_receiver_list_pointer(receiver, span)?;
        let output = format!("%list_get_{}", self.lines.len());
        let status = format!("%list_get_status_{}", self.lines.len());
        let location = self.locations.id_for(span);
        self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_list_get(ptr %context, ptr {list_pointer}, i64 {}, ptr {output}.slot, i8 {}, i32 {location})",
            register_name(index),
            flags
        ));
        self.emit_runtime_status(&status, format!("list_get_cont_{}", self.lines.len()));
        self.emit_value_result(destination, &format!("{output}.slot"))
    }

    fn next_label(&self, prefix: &str) -> String {
        format!("{prefix}_{}", self.lines.len())
    }

    fn emit_runtime_status(&mut self, status: &str, continuation: String) {
        let is_ok = format!("{status}.ok");
        let dispatch = self.next_label("runtime_dispatch");
        let runtime_fault = self.next_label("runtime_fault");
        let context_status = self.next_label("runtime_fault_status");
        self.line(format!("{is_ok} = icmp eq i32 {status}, 0"));
        self.line(format!(
            "br i1 {is_ok}, label %{continuation}, label %{dispatch}"
        ));
        self.label(dispatch);
        let is_fault = format!("{status}.language_fault");
        self.line(format!("{is_fault} = icmp eq i32 {status}, 1"));
        self.line(format!(
            "br i1 {is_fault}, label %{runtime_fault}, label %internal_exit"
        ));
        self.label(runtime_fault);
        let copied = format!("%{context_status}");
        self.line(format!(
            "{copied} = call i32 @keld_rt_v1_context_fault_parts(ptr %context, ptr %out_kind, ptr %out_location)"
        ));
        let copied_ok = format!("%{context_status}.ok");
        self.line(format!("{copied_ok} = icmp eq i32 {copied}, 0"));
        self.line(format!(
            "br i1 {copied_ok}, label %fault_exit, label %internal_exit"
        ));
        self.label(continuation);
    }

    fn line(&mut self, value: impl Into<String>) {
        self.lines.push(format!("  {}", value.into()));
    }

    fn label(&mut self, value: impl Into<String>) {
        self.current_label = value.into();
        self.lines.push(format!("{}:", self.current_label));
    }

    fn continuation(&mut self, register: Register, suffix: &str) -> String {
        format!("cont_{}_{}_{}", self.current_label, register.0, suffix)
    }

    fn fault_label(&mut self, kind: IrFaultKind, span: Span) -> String {
        let key = (ir_fault_code(kind), self.locations.id_for(span));
        self.faults.insert(key);
        format!("fault_{}_{}", key.0, key.1)
    }

    fn emit_block(&mut self, block: &keld_ir::IrBlock) -> Result<(), BackendError> {
        self.label(format!("bb{}", block.id.0));
        if block.id == self.function.entry && !self.slots_emitted {
            self.emit_managed_slots();
        }
        let leading_phi = block
            .instructions
            .iter()
            .take_while(|instruction| matches!(instruction, Instruction::Phi { .. }))
            .count();
        if block
            .instructions
            .iter()
            .skip(leading_phi)
            .any(|instruction| matches!(instruction, Instruction::Phi { .. }))
        {
            return Err(BackendError::Unsupported(
                "Phi must precede non-Phi instructions in a block".to_owned(),
            ));
        }
        for (index, instruction) in block.instructions.iter().enumerate() {
            self.emit_instruction(
                instruction,
                block.id,
                u32::try_from(index).map_err(|_| {
                    BackendError::Unsupported(
                        "instruction index exceeds native allocation-site range".to_owned(),
                    )
                })?,
            )?;
        }
        let exit_label = self
            .block_exit_labels
            .get(&block.id)
            .cloned()
            .expect("every lowered block has a stable exit label");
        self.line(format!("br label %{exit_label}"));
        self.label(exit_label);
        self.emit_terminator(&block.terminator)
    }

    fn emit_managed_slots(&mut self) {
        self.slots_emitted = true;
        for (index, ty) in self.function.register_types.iter().enumerate() {
            let register = Register(
                u32::try_from(index).expect("register index exceeds native register range"),
            );
            if is_managed_type(ty) && !self.function.parameters.contains(&register) {
                let slot = managed_slot_name(self.function, register);
                self.line(format!("{slot} = alloca {VALUE_IR_TYPE}"));
                self.line(format!("store {VALUE_IR_TYPE} zeroinitializer, ptr {slot}"));
            }
        }

        let mut capacities = BTreeMap::<u32, u32>::new();
        for (index, storage) in self.function.register_storage.iter().enumerate() {
            let register = Register(
                u32::try_from(index).expect("register index exceeds native register range"),
            );
            if self.owns_register(register)
                && let RegisterStorage::Home { scope, .. } = storage
            {
                capacities
                    .entry(scope.0)
                    .and_modify(|capacity| *capacity = capacity.saturating_add(1))
                    .or_insert(1);
            }
        }
        let slot_count = u32::try_from(self.function.register_types.len())
            .expect("register count exceeds native ABI range");
        for (scope, capacity) in capacities {
            let ids = format!("%home_scope_{scope}_ids");
            let count = format!("%home_scope_{scope}_count");
            let slots = format!("%home_scope_{scope}_slots");
            self.line(format!("{ids} = alloca [{capacity} x i32]"));
            self.line(format!("{count} = alloca i32"));
            self.line(format!("store i32 0, ptr {count}"));
            self.line(format!("{slots} = alloca [{slot_count} x ptr]"));
            for index in 0..slot_count {
                let pointer = format!("%home_scope_{scope}_slot_{index}");
                self.line(format!(
                    "{pointer} = getelementptr inbounds [{slot_count} x ptr], ptr {slots}, i64 0, i64 {index}"
                ));
                self.line(format!("store ptr null, ptr {pointer}"));
            }
            for (index, storage) in self.function.register_storage.iter().enumerate() {
                let register = Register(
                    u32::try_from(index).expect("register index exceeds native register range"),
                );
                if self.owns_register(register)
                    && matches!(
                        storage,
                        RegisterStorage::Home { scope: home_scope, .. }
                            if home_scope.0 == scope
                    )
                {
                    let pointer = format!("%home_scope_{scope}_slot_{}", register.0);
                    self.line(format!(
                        "store ptr {}, ptr {pointer}",
                        self.value_name(register)
                    ));
                }
            }
            self.home_trackers.insert(
                scope,
                HomeTracker {
                    ids,
                    count,
                    slots,
                    capacity,
                    slot_count,
                },
            );
        }
    }

    fn instruction_allocates(&self, instruction: &Instruction) -> bool {
        match instruction {
            Instruction::ConstText { .. }
            | Instruction::TextConcat { .. }
            | Instruction::ConstructStruct { .. }
            | Instruction::BeginLifecycle { .. }
            | Instruction::AllocateEntity { .. }
            | Instruction::ListNew { .. }
            | Instruction::ListPush { .. }
            | Instruction::ListPushPlace { .. }
            | Instruction::ListReserve { .. }
            | Instruction::ListTryReserve { .. } => true,
            Instruction::Copy { dst, src, .. } => {
                self.owns_register(*src) && self.is_home_register(*dst)
            }
            Instruction::ReadStructField { dst, .. } | Instruction::ReadField { dst, .. } => {
                self.owns_register(*dst) && self.is_home_register(*dst)
            }
            Instruction::ListGet { dst, receiver, .. } => {
                self.list_element_type(receiver.list).is_ok_and(|element| {
                    self.owns_register(*dst)
                        && self.is_home_register(*dst)
                        && owns_native_payload(element)
                })
            }
            _ => false,
        }
    }

    fn emit_test_site(
        &mut self,
        block: IrBlockId,
        instruction_index: u32,
    ) -> Result<(), BackendError> {
        let base = self
            .allocation_schedule
            .base_id(self.function.id, block, instruction_index)
            .ok_or_else(|| {
                BackendError::Unsupported(
                    "allocation instruction is missing from deterministic site schedule".to_owned(),
                )
            })?;
        let stem = format!("test_site_{}", self.lines.len());
        let status = format!("%{stem}.status");
        let ok = format!("%{stem}.ok");
        let continuation = format!("{stem}.cont");
        self.line(format!(
            "{status} = call i32 @keld_rt_v1_test_site(ptr %context, i32 {base})"
        ));
        self.line(format!("{ok} = icmp eq i32 {status}, 0"));
        self.line(format!(
            "br i1 {ok}, label %{continuation}, label %internal_exit"
        ));
        self.label(continuation);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn emit_instruction(
        &mut self,
        instruction: &Instruction,
        block: IrBlockId,
        instruction_index: u32,
    ) -> Result<(), BackendError> {
        if self.instruction_allocates(instruction) {
            self.emit_test_site(block, instruction_index)?;
        }
        match instruction {
            Instruction::ConstInt { dst, value, .. } => {
                self.line(format!("{} = add i64 0, {value}", register_name(*dst)));
            }
            Instruction::ConstBool { dst, value, .. } => {
                let expected = i32::from(*value);
                self.line(format!(
                    "{} = icmp eq i1 {expected}, 1",
                    register_name(*dst)
                ));
            }
            Instruction::CheckedUnaryInt { dst, op, src, span } => {
                self.emit_unary(*dst, *op, *src, *span);
            }
            Instruction::CheckedBinaryInt {
                dst,
                op,
                lhs,
                rhs,
                span,
            } => self.emit_binary(*dst, *op, *lhs, *rhs, *span),
            Instruction::Not { dst, src, .. } => {
                self.line(format!(
                    "{} = xor i1 {}, true",
                    register_name(*dst),
                    register_name(*src)
                ));
            }
            Instruction::Compare {
                dst,
                op,
                lhs,
                rhs,
                span,
            } => {
                let identity_compare = matches!(
                    self.function.register_types.get(lhs.0 as usize),
                    Some(IrType::Entity(_) | IrType::Link { .. })
                ) && matches!(
                    self.function.register_types.get(rhs.0 as usize),
                    Some(IrType::Entity(_) | IrType::Link { .. })
                );
                if identity_compare {
                    if !matches!(op, CompareOp::Eq | CompareOp::NotEq) {
                        return Err(BackendError::Unsupported(
                            "identity comparison supports only equality".to_owned(),
                        ));
                    }
                    let lhs_value = format!("%identity_lhs_{}", self.lines.len());
                    let rhs_value = format!("%identity_rhs_{}", self.lines.len());
                    self.line(format!(
                        "{lhs_value} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*lhs)
                    ));
                    self.line(format!(
                        "{rhs_value} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*rhs)
                    ));
                    let mut equal = None;
                    for word_index in 0..3 {
                        let lhs_word =
                            format!("%identity_lhs_word_{}_{}", self.lines.len(), word_index);
                        let rhs_word =
                            format!("%identity_rhs_word_{}_{}", self.lines.len(), word_index);
                        let same = format!("%identity_same_{}_{}", self.lines.len(), word_index);
                        self.line(format!(
                            "{lhs_word} = extractvalue {VALUE_IR_TYPE} {lhs_value}, 2, {word_index}"
                        ));
                        self.line(format!(
                            "{rhs_word} = extractvalue {VALUE_IR_TYPE} {rhs_value}, 2, {word_index}"
                        ));
                        self.line(format!("{same} = icmp eq i64 {lhs_word}, {rhs_word}"));
                        equal = Some(match equal {
                            None => same,
                            Some(previous) => {
                                let combined = format!("%identity_equal_{}", self.lines.len());
                                self.line(format!("{combined} = and i1 {previous}, {same}"));
                                combined
                            }
                        });
                    }
                    let equal = equal.expect("identity has three fixed words");
                    if matches!(op, CompareOp::Eq) {
                        self.line(format!("{} = xor i1 {equal}, false", register_name(*dst)));
                    } else {
                        self.line(format!("{} = xor i1 {equal}, true", register_name(*dst)));
                    }
                    return Ok(());
                }
                if matches!(
                    self.function.register_types.get(lhs.0 as usize),
                    Some(IrType::Text)
                ) {
                    if !matches!(op, CompareOp::Eq | CompareOp::NotEq)
                        || !matches!(
                            self.function.register_types.get(rhs.0 as usize),
                            Some(IrType::Text)
                        )
                    {
                        return Err(BackendError::Unsupported(
                            "Text comparison supports only equality".to_owned(),
                        ));
                    }
                    let out = format!("%text_equal_{}", self.lines.len());
                    let status = format!("%text_equal_status_{}", self.lines.len());
                    let location = self.locations.id_for(*span);
                    self.line(format!("{out}.slot = alloca i8"));
                    self.line(format!(
                        "{status} = call i32 @keld_rt_v1_text_equal(ptr %context, ptr {}, ptr {}, ptr {out}.slot, i32 {location})",
                        self.value_name(*lhs),
                        self.value_name(*rhs),
                    ));
                    self.emit_runtime_status(
                        &status,
                        format!("text_equal_cont_{}", self.lines.len()),
                    );
                    self.line(format!("{out} = load i8, ptr {out}.slot"));
                    let value = if matches!(op, CompareOp::Eq) {
                        format!("{out}.bool")
                    } else {
                        format!("{out}.not")
                    };
                    if matches!(op, CompareOp::Eq) {
                        self.line(format!("{value} = icmp ne i8 {out}, 0"));
                    } else {
                        self.line(format!("{value} = icmp eq i8 {out}, 0"));
                    }
                    self.line(format!("{} = xor i1 {value}, false", register_name(*dst)));
                    return Ok(());
                }
                let ty = llvm_type_for_register(self.function, *lhs)?;
                if ty != llvm_type_for_register(self.function, *rhs)? {
                    return Err(BackendError::Unsupported(
                        "scalar comparison operands have different types".to_owned(),
                    ));
                }
                if matches!(
                    op,
                    CompareOp::Less | CompareOp::LessEq | CompareOp::Greater | CompareOp::GreaterEq
                ) && ty != "i64"
                {
                    return Err(BackendError::Unsupported(
                        "ordered comparison requires Int operands".to_owned(),
                    ));
                }
                self.line(format!(
                    "{} = icmp {} {} {}, {}",
                    register_name(*dst),
                    llvm_compare(*op),
                    ty,
                    register_name(*lhs),
                    register_name(*rhs)
                ));
            }
            Instruction::Phi { dst, inputs, .. } => {
                if self.is_managed_register(*dst) {
                    let pointers = inputs
                        .iter()
                        .map(|(block, register)| {
                            let label = self
                                .block_exit_labels
                                .get(block)
                                .cloned()
                                .unwrap_or_else(|| format!("bb{}", block.0));
                            format!("[ {}, %{label} ]", self.value_name(*register))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let pointer = format!("%phi_managed_ptr_{}", self.lines.len());
                    let value = format!("%phi_managed_value_{}", self.lines.len());
                    self.line(format!("{pointer} = phi ptr {pointers}"));
                    self.line(format!("{value} = load {VALUE_IR_TYPE}, ptr {pointer}"));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {value}, ptr {}",
                        self.value_name(*dst)
                    ));
                    self.emit_home_track(*dst)?;
                    // A managed Phi whose destination is an owned Home is a
                    // move from the selected predecessor Home, just like the
                    // interpreter's `take_register` during block entry.  The
                    // envelope is copied into the destination slot above, so
                    // clear every Home input afterward; only the selected
                    // input can be live, and clearing an unselected empty Home
                    // is harmless.  Loan Phis retain their source envelopes.
                    if self.is_home_register(*dst) {
                        for (_, source) in inputs {
                            if *source != *dst && self.is_home_register(*source) {
                                self.line(format!(
                                    "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                                    self.value_name(*source)
                                ));
                                self.emit_home_untrack(*source)?;
                            }
                        }
                    }
                    return Ok(());
                }
                let ty = llvm_type_for_register(self.function, *dst)?;
                let values = inputs
                    .iter()
                    .map(|(block, register)| {
                        let label = self
                            .block_exit_labels
                            .get(block)
                            .cloned()
                            .unwrap_or_else(|| format!("bb{}", block.0));
                        format!("[ {}, %{label} ]", register_name(*register))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.line(format!("{} = phi {ty} {values}", register_name(*dst)));
            }
            Instruction::ConstText { dst, value, span } => {
                if !matches!(
                    self.function.register_types.get(dst.0 as usize),
                    Some(IrType::Text)
                ) {
                    return Err(BackendError::Unsupported(
                        "ConstText destination is not Text".to_owned(),
                    ));
                }
                let global = format!(
                    "keld_text_{}_{}",
                    self.function.id.0,
                    self.text_literals.len()
                );
                self.text_literals
                    .insert(global.clone(), value.as_bytes().to_vec());
                let length = value.len();
                let array_length = length.saturating_add(1);
                let pointer = format!(
                    "getelementptr inbounds ([{array_length} x i8], ptr @{global}, i64 0, i64 0)"
                );
                let status = format!("%text_new_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_text_new(ptr %context, ptr {pointer}, i64 {length}, ptr {}, i32 {location})",
                    self.value_name(*dst),
                ));
                self.emit_runtime_status(&status, format!("text_new_cont_{}", self.lines.len()));
                self.emit_home_track(*dst)?;
            }
            Instruction::TextByteLength { dst, text, span } => {
                let output = format!("%text_length_{}", self.lines.len());
                let status = format!("%text_length_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca i64"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_text_byte_length(ptr %context, ptr {}, ptr {output}.slot, i32 {location})",
                    self.value_name(*text),
                ));
                self.emit_runtime_status(&status, format!("text_length_cont_{}", self.lines.len()));
                self.line(format!(
                    "{} = load i64, ptr {output}.slot",
                    register_name(*dst)
                ));
            }
            Instruction::TextIsEmpty { dst, text, span } => {
                let output = format!("%text_empty_{}", self.lines.len());
                let status = format!("%text_empty_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca i8"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_text_is_empty(ptr %context, ptr {}, ptr {output}.slot, i32 {location})",
                    self.value_name(*text),
                ));
                self.emit_runtime_status(&status, format!("text_empty_cont_{}", self.lines.len()));
                let loaded = format!("{output}.loaded");
                self.line(format!("{loaded} = load i8, ptr {output}.slot"));
                self.line(format!("{} = icmp ne i8 {loaded}, 0", register_name(*dst)));
            }
            Instruction::TextConcat {
                dst,
                lhs,
                rhs,
                span,
            } => {
                let status = format!("%text_concat_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_text_concat(ptr %context, ptr {}, ptr {}, ptr {}, i32 {location})",
                    self.value_name(*lhs),
                    self.value_name(*rhs),
                    self.value_name(*dst),
                ));
                self.emit_runtime_status(&status, format!("text_concat_cont_{}", self.lines.len()));
                self.emit_home_track(*dst)?;
            }
            Instruction::Copy { dst, src, span } => {
                if self.is_managed_register(*dst) && !self.is_home_register(*dst) {
                    let value = format!("%value_copy_loan_{}", self.lines.len());
                    self.line(format!(
                        "{value} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*src)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {value}, ptr {}",
                        self.value_name(*dst)
                    ));
                } else if self
                    .function
                    .register_types
                    .get(src.0 as usize)
                    .is_some_and(owns_native_payload)
                {
                    let status = format!("%value_copy_status_{}", self.lines.len());
                    let location = self.locations.id_for(*span);
                    self.line(format!(
                        "{status} = call i32 @keld_rt_v1_value_copy(ptr %context, ptr {}, ptr {}, i32 {location})",
                        self.value_name(*src),
                        self.value_name(*dst),
                    ));
                    self.emit_runtime_status(
                        &status,
                        format!("value_copy_cont_{}", self.lines.len()),
                    );
                    self.emit_home_track(*dst)?;
                } else if self.is_managed_register(*src) {
                    let value = format!("%value_copy_fixed_{}", self.lines.len());
                    self.line(format!(
                        "{value} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*src)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {value}, ptr {}",
                        self.value_name(*dst)
                    ));
                } else {
                    let ty = self.value_type(*src)?;
                    if ty == "i64" {
                        self.line(format!(
                            "{} = add i64 0, {}",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    } else if ty == "i1" {
                        self.line(format!(
                            "{} = xor i1 {}, false",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    } else {
                        self.line(format!(
                            "{} = select i1 true, {ty} {}, {ty} zeroinitializer",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    }
                }
            }
            Instruction::Take { dst, src, .. } => {
                if self.is_managed_register(*src) {
                    let loaded = format!("%take_{}_{}", src.0, self.lines.len());
                    self.line(format!(
                        "{loaded} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*src)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {loaded}, ptr {}",
                        self.value_name(*dst)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*src)
                    ));
                    self.emit_home_untrack(*src)?;
                    self.emit_home_track(*dst)?;
                } else {
                    let ty = self.value_type(*src)?;
                    self.line(format!(
                        "{} = select i1 true, {ty} {}, {ty} zeroinitializer",
                        register_name(*dst),
                        register_name(*src)
                    ));
                }
            }
            Instruction::InstallHome {
                destination,
                source,
                displaced,
                ..
            } => {
                if self.is_managed_register(*destination) {
                    let previous = format!("%install_previous_{}", self.lines.len());
                    let incoming = format!("%install_incoming_{}", self.lines.len());
                    self.line(format!(
                        "{previous} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*destination)
                    ));
                    self.line(format!(
                        "{incoming} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*source)
                    ));
                    self.emit_home_untrack(*destination)?;
                    self.emit_home_untrack(*source)?;
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {previous}, ptr {}",
                        self.value_name(*displaced)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {incoming}, ptr {}",
                        self.value_name(*destination)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*source)
                    ));
                    self.emit_home_track(*destination)?;
                } else {
                    let ty = self.value_type(*source)?;
                    let previous = format!("%install_previous_{}", self.lines.len());
                    self.line(format!(
                        "{previous} = select i1 true, {ty} {}, {ty} zeroinitializer",
                        register_name(*destination)
                    ));
                    self.line(format!(
                        "{} = select i1 true, {ty} {}, {ty} zeroinitializer",
                        register_name(*destination),
                        register_name(*source)
                    ));
                    self.line(format!(
                        "{} = select i1 true, {ty} {previous}, {ty} zeroinitializer",
                        register_name(*displaced),
                    ));
                }
            }
            Instruction::MoveHome {
                destination,
                source,
                ..
            } => {
                if self.is_managed_register(*destination) {
                    let incoming = format!("%move_incoming_{}", self.lines.len());
                    self.line(format!(
                        "{incoming} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*source)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {incoming}, ptr {}",
                        self.value_name(*destination)
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*source)
                    ));
                    self.emit_home_untrack(*source)?;
                    self.emit_home_track(*destination)?;
                } else {
                    let ty = self.value_type(*source)?;
                    self.line(format!(
                        "{} = select i1 true, {ty} {}, {ty} zeroinitializer",
                        register_name(*destination),
                        register_name(*source)
                    ));
                }
            }
            Instruction::DropHome { home, span }
            | Instruction::DropIfLive { home, span }
            | Instruction::DropSlot { slot: home, span } => {
                if self
                    .function
                    .register_types
                    .get(home.0 as usize)
                    .is_some_and(owns_native_payload)
                {
                    let status = format!("%value_drop_status_{}", self.lines.len());
                    let location = self.locations.id_for(*span);
                    self.line(format!(
                        "{status} = call i32 @keld_rt_v1_value_drop(ptr %context, ptr {}, i32 {location})",
                        self.value_name(*home),
                    ));
                    self.emit_runtime_status(
                        &status,
                        format!("value_drop_cont_{}", self.lines.len()),
                    );
                    self.emit_home_untrack(*home)?;
                }
            }
            Instruction::CleanupTrackedScope { scope, span } => {
                self.emit_cleanup_scope(scope.0, *span);
            }
            Instruction::ConstructStruct {
                dst,
                definition,
                fields,
                span,
            } => {
                let Some(IrType::Struct(expected_definition)) =
                    self.function.register_types.get(dst.0 as usize)
                else {
                    return Err(BackendError::Unsupported(
                        "struct construction destination is not a struct".to_owned(),
                    ));
                };
                if expected_definition != definition {
                    return Err(BackendError::Unsupported(
                        "struct construction definition does not match destination".to_owned(),
                    ));
                }
                let definition_info = self
                    .module
                    .definitions
                    .iter()
                    .find(|candidate| candidate.id == *definition)
                    .ok_or_else(|| {
                        BackendError::Unsupported("struct definition is missing".to_owned())
                    })?;
                let count = definition_info.fields.len();
                let fields_array = format!("%struct_fields_{}", self.lines.len());
                let managed_array = format!("%struct_managed_{}", self.lines.len());
                self.line(format!(
                    "{fields_array} = alloca [{count} x {VALUE_IR_TYPE}]"
                ));
                self.line(format!("{managed_array} = alloca [{count} x i8]"));
                let mut seen = BTreeSet::new();
                for (field, source) in fields {
                    let index = self.definition_field_index(*definition, *field)?;
                    if !seen.insert(index) {
                        return Err(BackendError::Unsupported(
                            "struct field is repeated".to_owned(),
                        ));
                    }
                    let (source_pointer, managed) = self.emit_value_argument(*source)?;
                    let field_pointer = format!("%struct_field_{}_{}", self.lines.len(), index);
                    let managed_pointer =
                        format!("%struct_managed_field_{}_{}", self.lines.len(), index);
                    self.line(format!(
                        "{field_pointer} = getelementptr inbounds [{count} x {VALUE_IR_TYPE}], ptr {fields_array}, i64 0, i64 {index}"
                    ));
                    self.line(format!("{managed_pointer} = getelementptr inbounds [{count} x i8], ptr {managed_array}, i64 0, i64 {index}"));
                    let value = format!("%struct_source_{}", self.lines.len());
                    self.line(format!(
                        "{value} = load {VALUE_IR_TYPE}, ptr {source_pointer}"
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {value}, ptr {field_pointer}"
                    ));
                    self.line(format!(
                        "store i8 {}, ptr {managed_pointer}",
                        u8::from(managed)
                    ));
                }
                if seen.len() != count {
                    return Err(BackendError::Unsupported(
                        "struct construction does not initialize every field".to_owned(),
                    ));
                }
                let status = format!("%struct_new_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_struct_new(ptr %context, i32 {}, ptr {fields_array}, ptr {managed_array}, i32 {count}, ptr {}, i32 {location})",
                    definition.0,
                    self.value_name(*dst)
                ));
                self.emit_runtime_status(&status, format!("struct_new_cont_{}", self.lines.len()));
                self.emit_home_track(*dst)?;
                for (_, source) in fields {
                    if self.owns_register(*source) {
                        self.line(format!(
                            "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                            self.value_name(*source)
                        ));
                        self.emit_home_untrack(*source)?;
                    }
                }
            }
            Instruction::ReadStructField {
                dst,
                base,
                field,
                span,
            } => {
                let Some(IrType::Struct(definition)) =
                    self.function.register_types.get(base.0 as usize)
                else {
                    return Err(BackendError::Unsupported(
                        "struct field base is not a struct".to_owned(),
                    ));
                };
                let index = self.definition_field_index(*definition, *field)?;
                let output = format!("%struct_read_{}", self.lines.len());
                let status = format!("%struct_read_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_struct_field(ptr %context, ptr {}, i32 {index}, ptr {output}.slot, i8 {}, i32 {location})",
                    self.value_name(*base),
                    u8::from(self.owns_register(*dst) && self.is_home_register(*dst))
                ));
                self.emit_runtime_status(&status, format!("struct_read_cont_{}", self.lines.len()));
                self.emit_value_result(*dst, &format!("{output}.slot"))?;
            }
            Instruction::ConstNoneLink { dst, entity, .. } => {
                if !matches!(
                    self.function.register_types.get(dst.0 as usize),
                    Some(IrType::Link { .. })
                ) {
                    return Err(BackendError::Unsupported(
                        "ConstNoneLink destination is not a Link".to_owned(),
                    ));
                }
                let value = format!("%none_link_{}", self.lines.len());
                let expected = format!("{value}.expected");
                self.line(format!(
                    "{value} = insertvalue {VALUE_IR_TYPE} zeroinitializer, i64 {}, 2, 0",
                    entity.0
                ));
                self.line(format!("{expected} = select i1 true, {VALUE_IR_TYPE} {value}, {VALUE_IR_TYPE} zeroinitializer"));
                self.line(format!(
                    "store {VALUE_IR_TYPE} {expected}, ptr {}",
                    self.value_name(*dst)
                ));
            }
            Instruction::BeginLifecycle { dst, parent, span } => {
                let parent_pointer = self.lifecycle_pointer(*parent);
                let output = format!("%lifecycle_result_{}", self.lines.len());
                let status = format!("%begin_lifecycle_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {{ i64, i32, i32 }}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_begin_lifecycle(ptr %context, ptr {parent_pointer}, ptr {output}.slot, i32 {location})"
                ));
                self.emit_runtime_status(
                    &status,
                    format!("begin_lifecycle_cont_{}", self.lines.len()),
                );
                self.line(format!(
                    "{} = load {{ i64, i32, i32 }}, ptr {output}.slot",
                    register_name(*dst)
                ));
            }
            Instruction::EndLifecycle { lifecycle, span } => {
                let lifecycle_pointer = self.lifecycle_pointer(*lifecycle);
                let status = format!("%end_lifecycle_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_end_lifecycle(ptr %context, ptr {lifecycle_pointer}, i32 {location})"
                ));
                self.emit_runtime_status(
                    &status,
                    format!("end_lifecycle_cont_{}", self.lines.len()),
                );
            }
            Instruction::AllocateEntity {
                dst,
                definition,
                fields,
                lifecycle,
                span,
            } => {
                let definition_info = self
                    .module
                    .definitions
                    .iter()
                    .find(|candidate| candidate.id == *definition)
                    .ok_or_else(|| {
                        BackendError::Unsupported("entity definition is missing".to_owned())
                    })?;
                let count = definition_info.fields.len();
                let fields_array = format!("%entity_fields_{}", self.lines.len());
                let managed_array = format!("%entity_managed_{}", self.lines.len());
                self.line(format!(
                    "{fields_array} = alloca [{count} x {VALUE_IR_TYPE}]"
                ));
                self.line(format!("{managed_array} = alloca [{count} x i8]"));
                let mut seen = BTreeSet::new();
                for (field, source) in fields {
                    let index = self.definition_field_index(*definition, *field)?;
                    if !seen.insert(index) {
                        return Err(BackendError::Unsupported(
                            "entity field is repeated".to_owned(),
                        ));
                    }
                    let (source_pointer, managed) = self.emit_value_argument(*source)?;
                    let field_pointer = format!("%entity_field_{}_{}", self.lines.len(), index);
                    let managed_pointer =
                        format!("%entity_managed_field_{}_{}", self.lines.len(), index);
                    self.line(format!(
                        "{field_pointer} = getelementptr inbounds [{count} x {VALUE_IR_TYPE}], ptr {fields_array}, i64 0, i64 {index}"
                    ));
                    self.line(format!(
                        "{managed_pointer} = getelementptr inbounds [{count} x i8], ptr {managed_array}, i64 0, i64 {index}"
                    ));
                    let value = format!("%entity_source_{}", self.lines.len());
                    self.line(format!(
                        "{value} = load {VALUE_IR_TYPE}, ptr {source_pointer}"
                    ));
                    self.line(format!(
                        "store {VALUE_IR_TYPE} {value}, ptr {field_pointer}"
                    ));
                    self.line(format!(
                        "store i8 {}, ptr {managed_pointer}",
                        u8::from(managed)
                    ));
                }
                if seen.len() != count {
                    return Err(BackendError::Unsupported(
                        "entity construction does not initialize every field".to_owned(),
                    ));
                }
                let lifecycle_pointer = self.lifecycle_pointer(*lifecycle);
                let status = format!("%allocate_entity_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_allocate_entity(ptr %context, i32 {}, ptr {fields_array}, ptr {managed_array}, i32 {count}, ptr {lifecycle_pointer}, ptr {}, i32 {location})",
                    definition.0,
                    self.value_name(*dst)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("allocate_entity_cont_{}", self.lines.len()),
                );
                self.emit_home_track(*dst)?;
                for (_, source) in fields {
                    if self.owns_register(*source) {
                        self.line(format!(
                            "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                            self.value_name(*source)
                        ));
                        self.emit_home_untrack(*source)?;
                    }
                }
            }
            Instruction::EntityToLink { dst, entity, span } => {
                let status = format!("%entity_to_link_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_entity_to_link(ptr %context, ptr {}, ptr {}, i32 {location})",
                    self.value_name(*entity),
                    self.value_name(*dst)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("entity_to_link_cont_{}", self.lines.len()),
                );
            }
            Instruction::OpenView { view, entity, .. } => {
                self.view_entities.insert(*view, *entity);
            }
            Instruction::ReadField {
                dst,
                view,
                field,
                span,
            } => {
                let entity = *self.view_entities.get(view).ok_or_else(|| {
                    BackendError::Unsupported("field read uses an unknown view".to_owned())
                })?;
                let definition = self.entity_register_definition(entity)?;
                let index = self.definition_field_index(definition, *field)?;
                let output = format!("%entity_read_{}", self.lines.len());
                let status = format!("%entity_read_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_entity_field(ptr %context, ptr {}, i32 {index}, ptr {output}.slot, i8 {}, i32 {location})",
                    self.value_name(entity),
                    u8::from(self.owns_register(*dst) && self.is_home_register(*dst))
                ));
                self.emit_runtime_status(&status, format!("entity_read_cont_{}", self.lines.len()));
                self.emit_value_result(*dst, &format!("{output}.slot"))?;
            }
            Instruction::WriteField {
                view,
                field,
                value,
                span,
            } => {
                let entity = *self.view_entities.get(view).ok_or_else(|| {
                    BackendError::Unsupported("field write uses an unknown view".to_owned())
                })?;
                let definition = self.entity_register_definition(entity)?;
                let field_type = self.definition_field_type(definition, *field)?;
                let managed = owns_native_payload(field_type);
                let (value_pointer, _) = self.emit_value_argument(*value)?;
                let output = format!("%entity_write_{}", self.lines.len());
                let status = format!("%entity_write_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_replace_field(ptr %context, ptr {}, i32 {}, ptr {value_pointer}, ptr {output}.slot, i8 {}, i32 {location})",
                    self.value_name(entity),
                    self.definition_field_index(definition, *field)?,
                    u8::from(managed)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("entity_write_cont_{}", self.lines.len()),
                );
                if managed {
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*value)
                    ));
                    let drop_status = format!("%entity_write_drop_status_{}", self.lines.len());
                    self.line(format!(
                        "{drop_status} = call i32 @keld_rt_v1_value_drop(ptr %context, ptr {output}.slot, i32 {location})"
                    ));
                    self.emit_runtime_status(
                        &drop_status,
                        format!("entity_write_drop_cont_{}", self.lines.len()),
                    );
                    self.emit_home_untrack(*value)?;
                }
            }
            Instruction::CloseView { view, .. } => {
                self.view_entities.remove(view);
            }
            Instruction::ReplaceField {
                view,
                field,
                source,
                displaced,
                span,
            } => {
                let entity = *self.view_entities.get(view).ok_or_else(|| {
                    BackendError::Unsupported("field replacement uses an unknown view".to_owned())
                })?;
                let definition = self.entity_register_definition(entity)?;
                let field_type = self.definition_field_type(definition, *field)?;
                let managed = owns_native_payload(field_type);
                let (value_pointer, _) = self.emit_value_argument(*source)?;
                let output = format!("%entity_replace_{}", self.lines.len());
                let status = format!("%entity_replace_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_replace_field(ptr %context, ptr {}, i32 {}, ptr {value_pointer}, ptr {output}.slot, i8 {}, i32 {location})",
                    self.value_name(entity),
                    self.definition_field_index(definition, *field)?,
                    u8::from(managed)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("entity_replace_cont_{}", self.lines.len()),
                );
                self.emit_value_result(*displaced, &format!("{output}.slot"))?;
                if managed {
                    self.emit_home_untrack(*source)?;
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*source)
                    ));
                }
            }
            Instruction::KeepEntity {
                entity,
                lifecycle,
                span,
            } => {
                let lifecycle_pointer = self.lifecycle_pointer(*lifecycle);
                let status = format!("%keep_entity_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_keep_entity(ptr %context, ptr {}, ptr {lifecycle_pointer}, i32 {location})",
                    self.value_name(*entity)
                ));
                self.emit_runtime_status(&status, format!("keep_entity_cont_{}", self.lines.len()));
            }
            Instruction::RetireEntity { entity, span } => {
                let status = format!("%retire_entity_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_retire_entity(ptr %context, ptr {}, i32 {location})",
                    self.value_name(*entity)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("retire_entity_cont_{}", self.lines.len()),
                );
                self.line(format!(
                    "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                    self.value_name(*entity)
                ));
            }
            Instruction::ListNew { dst, span } => {
                let status = format!("%list_new_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_new(ptr %context, ptr {}, i32 {location})",
                    self.value_name(*dst)
                ));
                self.emit_runtime_status(&status, format!("list_new_cont_{}", self.lines.len()));
                self.emit_home_track(*dst)?;
            }
            Instruction::ListLength { dst, list, span } => {
                let output = format!("%list_length_{}", self.lines.len());
                let status = format!("%list_length_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca i64"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_length(ptr %context, ptr {}, ptr {output}.slot, i32 {location})",
                    self.value_name(*list)
                ));
                self.emit_runtime_status(&status, format!("list_length_cont_{}", self.lines.len()));
                self.line(format!(
                    "{} = load i64, ptr {output}.slot",
                    register_name(*dst)
                ));
            }
            Instruction::ListPush { list, value, span } => {
                let element = self.list_element_type(*list)?;
                let managed = owns_native_payload(element);
                let (value_pointer, _) = self.emit_value_argument(*value)?;
                let status = format!("%list_push_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_push(ptr %context, ptr {}, ptr {value_pointer}, i8 {}, i32 {location})",
                    self.value_name(*list),
                    u8::from(managed)
                ));
                self.emit_runtime_status(&status, format!("list_push_cont_{}", self.lines.len()));
                if managed {
                    self.emit_home_untrack(*value)?;
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*value)
                    ));
                }
            }
            Instruction::ListPushPlace {
                list,
                source,
                value,
                span,
            } => {
                let list_pointer = self.emit_place_pointer(source, *span)?;
                let element = self.list_element_type(*list)?;
                let managed = owns_native_payload(element);
                let (value_pointer, _) = self.emit_value_argument(*value)?;
                let status = format!("%list_push_place_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_push(ptr %context, ptr {list_pointer}, ptr {value_pointer}, i8 {}, i32 {location})",
                    u8::from(managed)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_push_place_cont_{}", self.lines.len()),
                );
                if managed {
                    self.emit_home_untrack(*value)?;
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*value)
                    ));
                }
            }
            Instruction::ListRemove {
                dst,
                list,
                index,
                span,
            } => {
                let output = format!("%list_remove_{}", self.lines.len());
                let status = format!("%list_remove_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_remove(ptr %context, ptr {}, i64 {}, ptr {output}.slot, i32 {location})",
                    self.value_name(*list),
                    register_name(*index)
                ));
                self.emit_runtime_status(&status, format!("list_remove_cont_{}", self.lines.len()));
                self.emit_value_result(*dst, &format!("{output}.slot"))?;
                self.emit_home_track(*dst)?;
            }
            Instruction::ListRemovePlace {
                dst,
                source,
                index,
                span,
                ..
            } => {
                let list_pointer = self.emit_place_pointer(source, *span)?;
                let output = format!("%list_remove_place_{}", self.lines.len());
                let status = format!("%list_remove_place_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_remove(ptr %context, ptr {list_pointer}, i64 {}, ptr {output}.slot, i32 {location})",
                    register_name(*index)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_remove_place_cont_{}", self.lines.len()),
                );
                self.emit_value_result(*dst, &format!("{output}.slot"))?;
                self.emit_home_track(*dst)?;
            }
            Instruction::ListGet {
                dst,
                receiver,
                index,
                span,
            } => self.emit_list_read(*dst, receiver, *index, *span, true)?,
            Instruction::ListIndex {
                dst,
                receiver,
                index,
                span,
            } => self.emit_list_read(*dst, receiver, *index, *span, false)?,
            Instruction::ListReplace {
                receiver,
                index,
                value,
                displaced,
                span,
            } => {
                let element = self.list_element_type(receiver.list)?;
                let managed = owns_native_payload(element);
                let list_pointer = self.emit_receiver_list_pointer(receiver, *span)?;
                let (value_pointer, _) = self.emit_value_argument(*value)?;
                let output = format!("%list_replace_{}", self.lines.len());
                let status = format!("%list_replace_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_replace(ptr %context, ptr {list_pointer}, i64 {}, ptr {value_pointer}, ptr {output}.slot, i8 {}, i32 {location})",
                    register_name(*index),
                    u8::from(managed)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_replace_cont_{}", self.lines.len()),
                );
                self.emit_value_result(*displaced, &format!("{output}.slot"))?;
                if managed {
                    self.emit_home_untrack(*value)?;
                    self.line(format!(
                        "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                        self.value_name(*value)
                    ));
                }
            }
            Instruction::ListTryRemove {
                dst,
                receiver,
                index,
                span,
            } => {
                let list_pointer = self.emit_receiver_list_pointer(receiver, *span)?;
                let output = format!("%list_try_remove_{}", self.lines.len());
                let status = format!("%list_try_remove_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_try_remove(ptr %context, ptr {list_pointer}, i64 {}, ptr {output}.slot, i32 {location})",
                    register_name(*index)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_try_remove_cont_{}", self.lines.len()),
                );
                self.emit_value_result(*dst, &format!("{output}.slot"))?;
                self.emit_home_track(*dst)?;
            }
            Instruction::ListClear { receiver, span } => {
                let list_pointer = self.emit_receiver_list_pointer(receiver, *span)?;
                let status = format!("%list_clear_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_clear(ptr %context, ptr {list_pointer}, i32 {location})",
                ));
                self.emit_runtime_status(&status, format!("list_clear_cont_{}", self.lines.len()));
            }
            Instruction::ListReserve {
                receiver,
                additional,
                span,
            } => {
                let list_pointer = self.emit_receiver_list_pointer(receiver, *span)?;
                let status = format!("%list_reserve_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_reserve(ptr %context, ptr {list_pointer}, i64 {}, i32 {location})",
                    register_name(*additional)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_reserve_cont_{}", self.lines.len()),
                );
            }
            Instruction::ListTryReserve {
                dst,
                receiver,
                additional,
                span,
            } => {
                let list_pointer = self.emit_receiver_list_pointer(receiver, *span)?;
                let output = format!("%list_try_reserve_{}", self.lines.len());
                let status = format!("%list_try_reserve_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{output}.slot = alloca i8"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_list_try_reserve(ptr %context, ptr {list_pointer}, i64 {}, ptr {output}.slot, i32 {location})",
                    register_name(*additional)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("list_try_reserve_cont_{}", self.lines.len()),
                );
                let loaded = format!("{output}.loaded");
                self.line(format!("{loaded} = load i8, ptr {output}.slot"));
                self.line(format!("{} = icmp ne i8 {loaded}, 0", register_name(*dst)));
            }
            Instruction::Call {
                dst,
                function,
                arguments,
                argument_sources,
                current_lifecycle,
                span,
            } => self.emit_call(
                *dst,
                *function,
                arguments,
                argument_sources,
                *current_lifecycle,
                *span,
            )?,
            Instruction::ReplacePlace {
                destination,
                source,
                displaced,
                span,
            } => {
                self.emit_place_replace(
                    destination,
                    *source,
                    *displaced,
                    *span,
                    block,
                    instruction_index,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn emit_call(
        &mut self,
        dst: Option<Register>,
        function_id: FunctionId,
        arguments: &[(ParameterIndex, Register)],
        argument_sources: &[(ParameterIndex, Option<keld_ir::ArgumentSource>)],
        current_lifecycle: Register,
        span: Span,
    ) -> Result<(), BackendError> {
        let Some(callee) = self.module.functions.get(function_id.0 as usize) else {
            return Err(BackendError::Unsupported(format!(
                "call targets unknown function {}",
                function_id.0
            )));
        };
        let mut argument_map = BTreeMap::new();
        for (parameter, register) in arguments {
            if argument_map.insert(parameter.0, *register).is_some() {
                return Err(BackendError::Unsupported(
                    "scalar call contains duplicate parameter arguments".to_owned(),
                ));
            }
        }
        if argument_map.len() != callee.parameters.len() {
            return Err(BackendError::Unsupported(
                "scalar call argument count does not match callee".to_owned(),
            ));
        }
        let mut call_arguments = vec![
            "ptr %context".to_owned(),
            format!(
                "{} {}",
                llvm_type_for_register(self.function, current_lifecycle)?,
                register_name(current_lifecycle)
            ),
        ];
        for (index, parameter) in callee.parameters.iter().enumerate() {
            let parameter_index = u32::try_from(index).map_err(|_| {
                BackendError::Unsupported("callee parameter index exceeds u32".to_owned())
            })?;
            let Some(argument) = argument_map.get(&parameter_index) else {
                return Err(BackendError::Unsupported(
                    "scalar call is missing a parameter argument".to_owned(),
                ));
            };
            let expected_ir = callee
                .register_types
                .get(parameter.0 as usize)
                .ok_or_else(|| {
                    BackendError::Unsupported("callee parameter register is missing".to_owned())
                })?;
            let actual_ir = self
                .function
                .register_types
                .get(argument.0 as usize)
                .ok_or_else(|| {
                    BackendError::Unsupported("call argument register is missing".to_owned())
                })?;
            let expected = scalar_type(expected_ir).unwrap_or("ptr");
            let actual = scalar_type(actual_ir).unwrap_or("ptr");
            if expected != actual || is_managed_type(expected_ir) != is_managed_type(actual_ir) {
                return Err(BackendError::Unsupported(
                    "call argument type does not match callee".to_owned(),
                ));
            }
            let argument_source = argument_sources
                .iter()
                .find_map(|(parameter, source)| {
                    (parameter.0 == parameter_index).then_some(source.as_ref())
                })
                .flatten();
            let parameter_mode = callee.parameter_modes.get(index).copied().ok_or_else(|| {
                BackendError::Unsupported(
                    "callee parameter mode table is shorter than its parameters".to_owned(),
                )
            })?;
            let argument_value = if is_managed_type(actual_ir) {
                if parameter_mode == keld_ir::ParameterMode::Loan {
                    if let Some(source) = argument_source {
                        self.emit_place_pointer(source, span)?
                    } else {
                        self.value_name(*argument)
                    }
                } else {
                    // A consuming argument is already materialized in the
                    // validated argument register (a `take` lowers to a
                    // MoveHome before this call).  Passing the source place
                    // here would observe the now-empty original Home.
                    self.value_name(*argument)
                }
            } else {
                register_name(*argument)
            };
            call_arguments.push(format!("{expected} {argument_value}"));
        }
        let result_slot = match (&callee.return_type, dst) {
            (IrType::Unit, None) => None,
            (IrType::Unit, Some(_)) => {
                return Err(BackendError::Unsupported(
                    "Unit call unexpectedly has a result register".to_owned(),
                ));
            }
            (return_type, Some(_)) => {
                let ty = value_type(return_type).ok_or_else(|| {
                    BackendError::Unsupported("call return type is not representable".to_owned())
                })?;
                if is_managed_type(return_type) {
                    let destination = self.value_name(dst.ok_or_else(|| {
                        BackendError::Unsupported(
                            "managed call result is missing a destination".to_owned(),
                        )
                    })?);
                    call_arguments.push(format!("ptr {destination}"));
                    Some((destination, ty))
                } else {
                    let slot = format!("%call_result_{}_{}", function_id.0, self.lines.len());
                    self.line(format!("{slot} = alloca {ty}"));
                    call_arguments.push(format!("ptr {slot}"));
                    Some((slot, ty))
                }
            }
            (_, None) => {
                return Err(BackendError::Unsupported(
                    "non-Unit call is missing a result register".to_owned(),
                ));
            }
        };
        call_arguments.push("ptr %out_kind".to_owned());
        call_arguments.push("ptr %out_location".to_owned());
        let stem = format!("call_{}_{}", function_id.0, self.lines.len());
        let status = format!("%{stem}.status");
        self.line(format!(
            "{status} = call i32 @keld_fn_{}({})",
            function_id.0,
            call_arguments.join(", ")
        ));
        let is_ok = format!("%{stem}.is_ok");
        self.line(format!("{is_ok} = icmp eq i32 {status}, 0"));
        let ok_label = format!("{stem}.ok");
        let status_label = format!("{stem}.status_dispatch");
        let fault_label = format!("{stem}.fault");
        self.line(format!(
            "br i1 {is_ok}, label %{ok_label}, label %{status_label}"
        ));
        self.label(status_label);
        let is_fault = format!("%{stem}.is_fault");
        self.line(format!("{is_fault} = icmp eq i32 {status}, 1"));
        self.line(format!(
            "br i1 {is_fault}, label %{fault_label}, label %internal_exit"
        ));
        self.label(fault_label);
        self.line("br label %fault_exit");
        self.label(ok_label);
        for (index, _parameter) in callee.parameters.iter().enumerate() {
            let parameter_mode = callee.parameter_modes.get(index).copied().ok_or_else(|| {
                BackendError::Unsupported(
                    "callee parameter mode table is shorter than its parameters".to_owned(),
                )
            })?;
            if parameter_mode != keld_ir::ParameterMode::Take {
                continue;
            }
            let parameter_index = u32::try_from(index).expect("callee parameter index checked");
            let Some(argument) = argument_map.get(&parameter_index) else {
                continue;
            };
            if self.is_managed_register(*argument) {
                self.emit_home_untrack(*argument)?;
                self.line(format!(
                    "store {VALUE_IR_TYPE} zeroinitializer, ptr {}",
                    self.value_name(*argument)
                ));
            }
        }
        if let Some((slot, ty)) = result_slot
            && let Some(dst) = dst
            && !self.is_managed_register(dst)
        {
            self.line(format!("{} = load {ty}, ptr {slot}", register_name(dst)));
        } else if let Some(dst) = dst {
            self.emit_home_track(dst)?;
        }
        Ok(())
    }

    fn emit_unary(&mut self, dst: Register, op: IntUnaryOp, src: Register, span: Span) {
        let continuation = self.continuation(dst, "unary");
        let fault = self.fault_label(IrFaultKind::Arithmetic, span);
        match op {
            IntUnaryOp::Neg => {
                let pair = format!("{}.pair", register_name(dst));
                let result = register_name(dst);
                let overflow = format!("{result}.overflow");
                self.line(format!(
                    "{pair} = call {{ i64, i1 }} @llvm.ssub.with.overflow.i64(i64 0, i64 {})",
                    register_name(src)
                ));
                self.line(format!("{result} = extractvalue {{ i64, i1 }} {pair}, 0"));
                self.line(format!("{overflow} = extractvalue {{ i64, i1 }} {pair}, 1"));
                self.line(format!(
                    "br i1 {overflow}, label %{fault}, label %{continuation}"
                ));
                self.label(continuation);
            }
        }
    }

    fn emit_binary(
        &mut self,
        dst: Register,
        op: IntBinaryOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    ) {
        match op {
            IntBinaryOp::Add | IntBinaryOp::Sub | IntBinaryOp::Mul => {
                let intrinsic = match op {
                    IntBinaryOp::Add => "sadd",
                    IntBinaryOp::Sub => "ssub",
                    IntBinaryOp::Mul => "smul",
                    IntBinaryOp::Div | IntBinaryOp::Rem | IntBinaryOp::Shl | IntBinaryOp::Shr => {
                        unreachable!("matched arithmetic intrinsic")
                    }
                };
                let pair = format!("{}.pair", register_name(dst));
                let result = register_name(dst);
                let overflow = format!("{result}.overflow");
                let continuation = self.continuation(dst, "checked");
                let fault = self.fault_label(IrFaultKind::Arithmetic, span);
                self.line(format!(
                    "{pair} = call {{ i64, i1 }} @llvm.{intrinsic}.with.overflow.i64(i64 {}, i64 {})",
                    register_name(lhs),
                    register_name(rhs)
                ));
                self.line(format!("{result} = extractvalue {{ i64, i1 }} {pair}, 0"));
                self.line(format!("{overflow} = extractvalue {{ i64, i1 }} {pair}, 1"));
                self.line(format!(
                    "br i1 {overflow}, label %{fault}, label %{continuation}"
                ));
                self.label(continuation);
            }
            IntBinaryOp::Div | IntBinaryOp::Rem => {
                self.emit_division(dst, op, lhs, rhs, span);
            }
            IntBinaryOp::Shl | IntBinaryOp::Shr => {
                self.emit_shift(dst, op, lhs, rhs, span);
            }
        }
    }

    fn emit_division(
        &mut self,
        dst: Register,
        op: IntBinaryOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    ) {
        let result = register_name(dst);
        let label_stem = format!("r{}", dst.0);
        let zero = format!("{result}.zero");
        let nonzero = format!("{label_stem}.nonzero");
        let special = format!("{label_stem}.special");
        let normal = format!("{label_stem}.normal");
        let continuation = self.continuation(dst, "division");
        let division_fault = self.fault_label(IrFaultKind::DivisionByZero, span);
        self.line(format!("{zero} = icmp eq i64 {}, 0", register_name(rhs)));
        self.line(format!(
            "br i1 {zero}, label %{division_fault}, label %{nonzero}"
        ));
        self.label(nonzero.clone());
        let lhs_min = format!("{result}.lhs_min");
        let rhs_neg_one = format!("{result}.rhs_neg_one");
        let special_case = format!("{result}.special_case");
        self.line(format!(
            "{lhs_min} = icmp eq i64 {}, -9223372036854775808",
            register_name(lhs)
        ));
        self.line(format!(
            "{rhs_neg_one} = icmp eq i64 {}, -1",
            register_name(rhs)
        ));
        self.line(format!("{special_case} = and i1 {lhs_min}, {rhs_neg_one}"));
        if matches!(op, IntBinaryOp::Div) {
            let arithmetic_fault = self.fault_label(IrFaultKind::Arithmetic, span);
            self.line(format!(
                "br i1 {special_case}, label %{arithmetic_fault}, label %{normal}"
            ));
            self.label(normal);
            self.line(format!(
                "{result} = sdiv i64 {}, {}",
                register_name(lhs),
                register_name(rhs)
            ));
            self.line(format!("br label %{continuation}"));
        } else {
            self.line(format!(
                "br i1 {special_case}, label %{special}, label %{normal}"
            ));
            self.label(special.clone());
            let special_value = format!("{result}.special_value");
            self.line(format!("{special_value} = add i64 0, 0"));
            self.line(format!("br label %{continuation}"));
            self.label(normal.clone());
            let normal_value = format!("{result}.normal_value");
            self.line(format!(
                "{normal_value} = srem i64 {}, {}",
                register_name(lhs),
                register_name(rhs)
            ));
            self.line(format!("br label %{continuation}"));
            self.label(continuation.clone());
            self.line(format!(
                "{result} = phi i64 [ {special_value}, %{special} ], [ {normal_value}, %{normal} ]"
            ));
            return;
        }
        self.label(continuation);
    }

    fn emit_shift(
        &mut self,
        dst: Register,
        op: IntBinaryOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    ) {
        let result = register_name(dst);
        let label_stem = format!("r{}", dst.0);
        let negative = format!("{result}.negative");
        let too_large = format!("{result}.too_large");
        let invalid = format!("{result}.invalid");
        let valid = format!("{label_stem}.valid");
        let continuation = self.continuation(dst, "shift");
        let shift_fault = self.fault_label(IrFaultKind::Shift, span);
        self.line(format!(
            "{negative} = icmp slt i64 {}, 0",
            register_name(rhs)
        ));
        self.line(format!(
            "{too_large} = icmp sge i64 {}, 64",
            register_name(rhs)
        ));
        self.line(format!("{invalid} = or i1 {negative}, {too_large}"));
        self.line(format!(
            "br i1 {invalid}, label %{shift_fault}, label %{valid}"
        ));
        self.label(valid.clone());
        match op {
            IntBinaryOp::Shr => {
                self.line(format!(
                    "{result} = ashr i64 {}, {}",
                    register_name(lhs),
                    register_name(rhs)
                ));
                self.line(format!("br label %{continuation}"));
            }
            IntBinaryOp::Shl => {
                let amount = format!("{result}.amount");
                let lhs_wide = format!("{result}.lhs_wide");
                let one = format!("{result}.one");
                let power = format!("{result}.power");
                let product = format!("{result}.product");
                let low = format!("{result}.low");
                let high = format!("{result}.high");
                let fits = format!("{result}.fits");
                let wide_ok = format!("{label_stem}.wide_ok");
                let arithmetic_fault = self.fault_label(IrFaultKind::Arithmetic, span);
                self.line(format!(
                    "{amount} = sext i64 {} to i128",
                    register_name(rhs)
                ));
                self.line(format!(
                    "{lhs_wide} = sext i64 {} to i128",
                    register_name(lhs)
                ));
                self.line(format!("{one} = add i128 0, 1"));
                self.line(format!("{power} = shl i128 {one}, {amount}"));
                self.line(format!("{product} = mul i128 {lhs_wide}, {power}"));
                self.line(format!(
                    "{low} = icmp sge i128 {product}, -9223372036854775808"
                ));
                self.line(format!(
                    "{high} = icmp sle i128 {product}, 9223372036854775807"
                ));
                self.line(format!("{fits} = and i1 {low}, {high}"));
                self.line(format!(
                    "br i1 {fits}, label %{wide_ok}, label %{arithmetic_fault}"
                ));
                self.label(wide_ok);
                self.line(format!("{result} = trunc i128 {product} to i64"));
                self.line(format!("br label %{continuation}"));
            }
            IntBinaryOp::Add
            | IntBinaryOp::Sub
            | IntBinaryOp::Mul
            | IntBinaryOp::Div
            | IntBinaryOp::Rem => unreachable!("matched shift operation"),
        }
        self.label(continuation);
    }

    fn emit_terminator(&mut self, terminator: &Terminator) -> Result<(), BackendError> {
        match terminator {
            Terminator::Goto(target) => self.line(format!("br label %bb{}", target.0)),
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => self.line(format!(
                "br i1 {}, label %bb{}, label %bb{}",
                register_name(*condition),
                then_block.0,
                else_block.0
            )),
            Terminator::Return(Some(register)) => {
                let ty = self.value_type(*register)?;
                if self.function.return_type == IrType::Unit
                    || value_type(&self.function.return_type) != Some(ty)
                {
                    return Err(BackendError::Unsupported(
                        "native return type does not match the function".to_owned(),
                    ));
                }
                if self.is_managed_register(*register) {
                    let loaded = format!("%return_value_{}", self.lines.len());
                    self.line(format!(
                        "{loaded} = load {VALUE_IR_TYPE}, ptr {}",
                        self.value_name(*register)
                    ));
                    self.line(format!("store {VALUE_IR_TYPE} {loaded}, ptr %out_value"));
                } else {
                    self.line(format!(
                        "store {ty} {}, ptr %out_value",
                        register_name(*register)
                    ));
                }
                self.line("ret i32 0");
            }
            Terminator::Return(None) => {
                if self.function.return_type != IrType::Unit {
                    return Err(BackendError::Unsupported(
                        "non-Unit scalar function requires a return value".to_owned(),
                    ));
                }
                self.line("ret i32 0");
            }
            Terminator::Fault { kind, span } => {
                let label = self.fault_label(*kind, *span);
                self.line(format!("br label %{label}"));
            }
            Terminator::Unreachable => self.line("br label %internal_exit"),
            Terminator::ResolveLink {
                link,
                live_value,
                live,
                absent,
                span,
            } => {
                let entity = format!("%resolve_entity_{}", self.lines.len());
                let live_flag = format!("%resolve_live_{}", self.lines.len());
                let status = format!("%resolve_link_status_{}", self.lines.len());
                let location = self.locations.id_for(*span);
                self.line(format!("{entity}.slot = alloca {VALUE_IR_TYPE}"));
                self.line(format!("{live_flag}.slot = alloca i8"));
                self.line(format!(
                    "{status} = call i32 @keld_rt_v1_resolve_link(ptr %context, ptr {}, ptr {entity}.slot, ptr {live_flag}.slot, i32 {location})",
                    self.value_name(*link)
                ));
                self.emit_runtime_status(
                    &status,
                    format!("resolve_link_cont_{}", self.lines.len()),
                );
                let live_loaded = format!("{live_flag}.loaded");
                self.line(format!("{live_loaded} = load i8, ptr {live_flag}.slot"));
                let live_condition = format!("{live_flag}.condition");
                self.line(format!("{live_condition} = icmp ne i8 {live_loaded}, 0"));
                let entity_value = format!("%resolve_entity_value_{}", self.lines.len());
                self.line(format!(
                    "{entity_value} = load {VALUE_IR_TYPE}, ptr {entity}.slot"
                ));
                self.line(format!(
                    "store {VALUE_IR_TYPE} {entity_value}, ptr {}",
                    self.value_name(*live_value)
                ));
                self.line(format!(
                    "br i1 {live_condition}, label %bb{}, label %bb{}",
                    live.0, absent.0
                ));
            }
        }
        Ok(())
    }

    fn finish(mut self) -> (String, BTreeMap<String, Vec<u8>>) {
        let faults = self.faults.iter().copied().collect::<Vec<_>>();
        for (kind, location) in faults {
            self.lines.push(format!("fault_{kind}_{location}:"));
            self.line(format!("store i32 {kind}, ptr %out_kind"));
            self.line(format!("store i32 {location}, ptr %out_location"));
            self.line("br label %fault_exit");
        }
        self.lines.push("fault_exit:".to_owned());
        self.line("ret i32 1");
        self.lines.push("internal_exit:".to_owned());
        self.line("ret i32 2");
        (self.lines.join("\n"), self.text_literals)
    }
}

#[allow(clippy::too_many_lines)]
fn lower_scalar_ir(module: &Module, metadata: &SourceMetadata) -> Result<String, BackendError> {
    let Some(main) = module
        .functions
        .iter()
        .find(|function| function.id == module.main)
    else {
        return Err(BackendError::Unsupported(
            "module main function is missing".to_owned(),
        ));
    };
    if !main.parameters.is_empty() || main.return_type != IrType::Int {
        return Err(BackendError::Unsupported(
            "native scalar main must be parameterless and return Int".to_owned(),
        ));
    }
    for function in &module.functions {
        if function.parameter_modes.len() != function.parameters.len() {
            return Err(BackendError::Unsupported(format!(
                "function {} parameter mode table does not match its parameters",
                function.id.0
            )));
        }
        if function.return_type != IrType::Unit && value_type(&function.return_type).is_none() {
            return Err(BackendError::Unsupported(format!(
                "function {} has an unsupported return type",
                function.id.0
            )));
        }
        for parameter in &function.parameters {
            let Some(ty) = function.register_types.get(parameter.0 as usize) else {
                return Err(BackendError::Unsupported(
                    "function parameter register is missing".to_owned(),
                ));
            };
            if value_type(ty).is_none() {
                return Err(BackendError::Unsupported(
                    "native function parameters have no value representation".to_owned(),
                ));
            }
        }
        for (index, ty) in function.register_types.iter().enumerate() {
            if value_type(ty).is_none() && *ty != IrType::Unit {
                return Err(BackendError::Unsupported(format!(
                    "register {index} has a non-scalar type"
                )));
            }
        }
    }
    let locations = LocationTable::from_module(module, &metadata.source);
    let allocation_schedule = AllocationSchedule::from_module(module);
    let mut functions_ir = String::new();
    let mut text_literals = BTreeMap::new();
    for function in &module.functions {
        let mut lowerer = ScalarLowerer::new(module, function, &locations, &allocation_schedule);
        let Some(entry) = function
            .blocks
            .iter()
            .find(|block| block.id == function.entry)
        else {
            return Err(BackendError::Unsupported(
                "native scalar entry block is missing".to_owned(),
            ));
        };
        lowerer.emit_block(entry)?;
        for block in &function.blocks {
            if block.id != function.entry {
                lowerer.emit_block(block)?;
            }
        }
        let (body, literals) = lowerer.finish();
        text_literals.extend(literals);
        let lifecycle_type = llvm_type_for_register(function, function.current_lifecycle)?;
        let mut parameters = vec![
            format!("ptr %context"),
            format!(
                "{lifecycle_type} {}",
                register_name(function.current_lifecycle)
            ),
        ];
        for parameter in &function.parameters {
            let ty = function
                .register_types
                .get(parameter.0 as usize)
                .and_then(scalar_type)
                .map_or("ptr", |ty| ty);
            parameters.push(format!("{ty} {}", register_name(*parameter)));
        }
        if function.return_type != IrType::Unit {
            parameters.push("ptr %out_value".to_owned());
        }
        parameters.push("ptr %out_kind".to_owned());
        parameters.push("ptr %out_location".to_owned());
        let _ = writeln!(
            functions_ir,
            "define i32 @keld_fn_{}({}) {{\n{}\n}}\n",
            function.id.0,
            parameters.join(", "),
            body
        );
    }

    let path = metadata.path.to_string_lossy();
    let path_bytes = path.as_bytes();
    let path_length = path_bytes.len();
    let path_global = escape_llvm_bytes(path_bytes);
    let path_type_length = path_length.saturating_add(1);
    let path_pointer =
        format!("getelementptr inbounds ([{path_type_length} x i8], ptr @keld_path, i64 0, i64 0)");
    let main_name = format!("keld_fn_{}", main.id.0);
    let main_location = locations.id_for(main.span);
    let context_site = allocation_schedule
        .base_id(main.id, main.entry, 0)
        .unwrap_or(1);
    let mut literal_ir = String::new();
    for (name, bytes) in &text_literals {
        let array_length = bytes.len().saturating_add(1);
        let _ = writeln!(
            literal_ir,
            "@{name} = private constant [{array_length} x i8] c\"{}\\00\"",
            escape_llvm_bytes(bytes)
        );
    }
    let mut ir = format!(
        "target triple = \"{TARGET_TRIPLE}\"\n\n@keld_path = private constant [{path_type_length} x i8] c\"{path_global}\\00\"\n{literal_ir}\ndeclare {{ i64, i1 }} @llvm.sadd.with.overflow.i64(i64, i64)\ndeclare {{ i64, i1 }} @llvm.ssub.with.overflow.i64(i64, i64)\ndeclare {{ i64, i1 }} @llvm.smul.with.overflow.i64(i64, i64)\ndeclare ptr @keld_rt_v1_context_new()\ndeclare i32 @keld_rt_v1_context_destroy(ptr)\ndeclare i32 @keld_rt_v1_context_fault_parts(ptr, ptr, ptr)\ndeclare i32 @keld_rt_v1_value_copy(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_value_drop(ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_text_new(ptr, ptr, i64, ptr, i32)\ndeclare i32 @keld_rt_v1_text_byte_length(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_text_is_empty(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_text_equal(ptr, ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_text_concat(ptr, ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_print_int(i64)\ndeclare i32 @keld_rt_v1_print_fault(i32, ptr, i64, i32, i32)\n\n{functions_ir}\ndefine i32 @main() {{\nentry_main:\n  %out_value = alloca i64\n  %out_kind = alloca i32\n  %out_location = alloca i32\n  %context = call ptr @keld_rt_v1_context_new()\n  %context_ok = icmp ne ptr %context, null\n  br i1 %context_ok, label %context_ready, label %no_context\nno_context:\n  ret i32 70\ncontext_ready:\n  %status = call i32 @{main_name}(ptr %context, {{ i64, i32, i32 }} zeroinitializer, ptr %out_value, ptr %out_kind, ptr %out_location)\n  %ok = icmp eq i32 %status, 0\n  br i1 %ok, label %success, label %status_dispatch\nsuccess:\n  %result = load i64, ptr %out_value\n  %print_status = call i32 @keld_rt_v1_print_int(i64 %result)\n  %print_ok = icmp eq i32 %print_status, 0\n  br i1 %print_ok, label %destroy_success, label %destroy_internal\ndestroy_success:\n  %destroy_success_status = call i32 @keld_rt_v1_context_destroy(ptr %context)\n  %destroy_success_ok = icmp eq i32 %destroy_success_status, 0\n  br i1 %destroy_success_ok, label %done, label %internal_main\nstatus_dispatch:\n  %language_fault = icmp eq i32 %status, 1\n  br i1 %language_fault, label %fault_dispatch, label %destroy_internal\nfault_dispatch:\n  %out_kind_value = load i32, ptr %out_kind\n  %fault_location = load i32, ptr %out_location\n  switch i32 %fault_location, label %fault_unknown [\n",
    );
    ir = ir.replace(
        "declare ptr @keld_rt_v1_context_new()\n",
        "declare i32 @keld_rt_v1_abi_version()\ndeclare ptr @keld_rt_v1_context_new()\n",
    );
    let old_abi_bootstrap =
        "  %out_location = alloca i32\n  %context = call ptr @keld_rt_v1_context_new()\n";
    let new_abi_bootstrap = format!(
        "  %out_location = alloca i32\n  %runtime_abi_version = call i32 @keld_rt_v1_abi_version()\n  %runtime_abi_ok = icmp eq i32 %runtime_abi_version, {ABI_VERSION}\n  br i1 %runtime_abi_ok, label %context_bootstrap, label %internal_main\ncontext_bootstrap:\n  %context = call ptr @keld_rt_v1_context_new()\n"
    );
    ir = ir.replace(old_abi_bootstrap, &new_abi_bootstrap);
    let list_declarations = "declare i32 @keld_rt_v1_context_root_lifecycle(ptr, ptr)\ndeclare i32 @keld_rt_v1_begin_lifecycle(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_end_lifecycle(ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_list_new(ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_list_length(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_list_push(ptr, ptr, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_list_get(ptr, ptr, i64, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_list_remove(ptr, ptr, i64, ptr, i32)\ndeclare i32 @keld_rt_v1_list_replace(ptr, ptr, i64, ptr, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_list_try_remove(ptr, ptr, i64, ptr, i32)\ndeclare i32 @keld_rt_v1_list_clear(ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_list_reserve(ptr, ptr, i64, i32)\ndeclare i32 @keld_rt_v1_list_try_reserve(ptr, ptr, i64, ptr, i32)\ndeclare i32 @keld_rt_v1_struct_new(ptr, i32, ptr, ptr, i32, ptr, i32)\ndeclare i32 @keld_rt_v1_struct_field(ptr, ptr, i32, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_place_resolve(ptr, ptr, i8, ptr, i32, ptr, i32)\ndeclare i32 @keld_rt_v1_place_replace(ptr, ptr, i8, ptr, i32, ptr, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_home_track(ptr, ptr, i32, i32)\ndeclare i32 @keld_rt_v1_home_untrack(ptr, ptr, i32, i32)\ndeclare i32 @keld_rt_v1_cleanup_scope(ptr, ptr, ptr, ptr, i32, i32, i32)\ndeclare i32 @keld_rt_v1_allocate_entity(ptr, i32, ptr, ptr, i32, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_entity_to_link(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_resolve_link(ptr, ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_entity_field(ptr, ptr, i32, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_replace_field(ptr, ptr, i32, ptr, ptr, i8, i32)\ndeclare i32 @keld_rt_v1_keep_entity(ptr, ptr, ptr, i32)\ndeclare i32 @keld_rt_v1_retire_entity(ptr, ptr, i32)\n";
    ir = ir.replace(
        "declare i32 @keld_rt_v1_print_int(i64)\n",
        &format!("{list_declarations}declare i32 @keld_rt_v1_print_int(i64)\n"),
    );
    let old_context_bootstrap = format!(
        "  %context_ok = icmp ne ptr %context, null\n  br i1 %context_ok, label %context_ready, label %no_context\nno_context:\n  ret i32 70\ncontext_ready:\n  %status = call i32 @{main_name}(ptr %context, {{ i64, i32, i32 }} zeroinitializer, ptr %out_value, ptr %out_kind, ptr %out_location)"
    );
    let new_context_bootstrap = format!(
        "  %context_ok = icmp ne ptr %context, null\n  br i1 %context_ok, label %root_init, label %no_context\nno_context:\n  ret i32 70\nroot_init:\n  %root_lifecycle = alloca {{ i64, i32, i32 }}\n  %root_status = call i32 @keld_rt_v1_context_root_lifecycle(ptr %context, ptr %root_lifecycle)\n  %root_ok = icmp eq i32 %root_status, 0\n  br i1 %root_ok, label %context_ready, label %destroy_internal\ncontext_ready:\n  %root_value = load {{ i64, i32, i32 }}, ptr %root_lifecycle\n  %status = call i32 @{main_name}(ptr %context, {{ i64, i32, i32 }} %root_value, ptr %out_value, ptr %out_kind, ptr %out_location)"
    );
    let new_context_bootstrap = new_context_bootstrap.replace(
        "context_ready:\n  %root_value",
        "context_ready:\n  store i1 true, ptr %context_owned\n  %root_value",
    );
    ir = ir.replace(&old_context_bootstrap, &new_context_bootstrap);
    ir = ir.replace(
        "declare ptr @keld_rt_v1_context_new()\n",
        "declare ptr @keld_rt_v1_context_new()\ndeclare i32 @keld_rt_v1_context_new_at(i32, ptr, ptr, ptr)\n",
    );
    ir = ir.replace(
        "declare ptr @keld_rt_v1_context_new()\n",
        "declare ptr @keld_rt_v1_context_new()\ndeclare i32 @keld_rt_v1_test_site(ptr, i32)\n",
    );
    let old_context_call = "  %context = call ptr @keld_rt_v1_context_new()\n  %context_ok = icmp ne ptr %context, null\n  br i1 %context_ok, label %root_init, label %no_context\nno_context:\n  ret i32 70\n";
    let new_context_call = format!(
        "  %context_owned = alloca i1\n  store i1 false, ptr %context_owned\n  %context_slot = alloca ptr\n  %context_status = call i32 @keld_rt_v1_context_new_at(i32 {main_location}, ptr %context_slot, ptr %out_kind, ptr %out_location)\n  %context = load ptr, ptr %context_slot\n  %context_ok = icmp eq i32 %context_status, 0\n  br i1 %context_ok, label %root_init, label %context_init_status\ncontext_init_status:\n  %context_init_language = icmp eq i32 %context_status, 1\n  br i1 %context_init_language, label %fault_dispatch, label %internal_main\n"
    );
    ir = ir.replace(old_context_call, &new_context_call);
    let context_status_call = format!(
        "  %context_status = call i32 @keld_rt_v1_context_new_at(i32 {main_location}, ptr %context_slot, ptr %out_kind, ptr %out_location)"
    );
    let context_status_with_site = format!(
        "  %context_site_status = call i32 @keld_rt_v1_test_site(ptr null, i32 {context_site})\n  %context_site_ok = icmp eq i32 %context_site_status, 0\n  br i1 %context_site_ok, label %context_site_cont, label %internal_main\ncontext_site_cont:\n{context_status_call}"
    );
    ir = ir.replace(&context_status_call, &context_status_with_site);
    for location in locations.entries() {
        let _ = writeln!(
            ir,
            "    i32 {}, label %fault_location_{}",
            location.id, location.id
        );
    }
    ir.push_str("  ]\n");
    for location in locations.entries() {
        let _ = write!(
            ir,
            "fault_location_{}:\n  %fault_status_{} = call i32 @keld_rt_v1_print_fault(i32 %out_kind_value, ptr {}, i64 {}, i32 {}, i32 {})\n  %fault_print_ok_{} = icmp eq i32 %fault_status_{}, 0\n  br i1 %fault_print_ok_{}, label %fault_cleanup_dispatch_{}, label %destroy_internal\nfault_cleanup_dispatch_{}:\n  %fault_has_context_{} = load i1, ptr %context_owned\n  br i1 %fault_has_context_{}, label %destroy_fault, label %fault_done\n",
            location.id,
            location.id,
            path_pointer,
            path_length,
            location.line,
            location.column,
            location.id,
            location.id,
            location.id,
            location.id,
            location.id,
            location.id,
            location.id,
        );
    }
    ir.push_str(
        "fault_unknown:\n  br label %destroy_internal\ndestroy_fault:\n  %destroy_fault_status = call i32 @keld_rt_v1_context_destroy(ptr %context)\n  %destroy_fault_ok = icmp eq i32 %destroy_fault_status, 0\n  br i1 %destroy_fault_ok, label %fault_done, label %internal_main\nfault_done:\n  ret i32 2\ndone:\n  ret i32 0\ndestroy_internal:\n  %destroy_internal_status = call i32 @keld_rt_v1_context_destroy(ptr %context)\n  br label %internal_main\ninternal_main:\n  ret i32 70\n}\n",
    );
    Ok(ir)
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
    let generated_ir = lower_scalar_ir(module, metadata)?;
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
    keld_native_llvm::emit_ir(
        &object,
        module_name,
        &generated_ir,
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
