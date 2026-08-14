//! Safe Native-1 lowering boundary.

#![forbid(unsafe_code)]

use keld_ir::{
    CompareOp, FaultKind as IrFaultKind, FunctionId, Instruction, IntBinaryOp, IntUnaryOp, IrType,
    Module, ParameterIndex, Register, Terminator, validate,
};
use keld_native_abi::{RUNTIME_DLL_NAME, RUNTIME_IMPORT_LIBRARY_NAME};
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

fn escape_llvm_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for byte in bytes {
        match byte {
            b' '..=b'!' | b'#'..=b'[' | b']'..=b'~' => escaped.push(char::from(*byte)),
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
    lines: Vec<String>,
    current_label: String,
    faults: BTreeSet<(u32, u32)>,
}

impl<'module> ScalarLowerer<'module> {
    fn new(
        module: &'module Module,
        function: &'module keld_ir::Function,
        locations: &'module LocationTable,
    ) -> Self {
        Self {
            module,
            function,
            locations,
            lines: Vec::new(),
            current_label: String::new(),
            faults: BTreeSet::new(),
        }
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
        let mut saw_non_phi = false;
        for instruction in &block.instructions {
            if matches!(instruction, Instruction::Phi { .. }) {
                if saw_non_phi {
                    return Err(BackendError::Unsupported(
                        "Phi must precede non-Phi instructions in a block".to_owned(),
                    ));
                }
            } else {
                saw_non_phi = true;
            }
            self.emit_instruction(instruction)?;
        }
        self.emit_terminator(&block.terminator)
    }

    #[allow(clippy::too_many_lines)]
    fn emit_instruction(&mut self, instruction: &Instruction) -> Result<(), BackendError> {
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
                dst, op, lhs, rhs, ..
            } => {
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
                let ty = llvm_type_for_register(self.function, *dst)?;
                let values = inputs
                    .iter()
                    .map(|(block, register)| {
                        format!("[ {}, %bb{} ]", register_name(*register), block.0)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.line(format!("{} = phi {ty} {values}", register_name(*dst)));
            }
            Instruction::Call {
                dst,
                function,
                arguments,
                argument_sources,
                current_lifecycle,
                ..
            } => self.emit_call(
                *dst,
                *function,
                arguments,
                argument_sources,
                *current_lifecycle,
            )?,
            Instruction::ConstText { .. }
            | Instruction::ConstNoneLink { .. }
            | Instruction::Copy { .. }
            | Instruction::Take { .. }
            | Instruction::InstallHome { .. }
            | Instruction::MoveHome { .. }
            | Instruction::DropHome { .. }
            | Instruction::DropIfLive { .. }
            | Instruction::DropSlot { .. }
            | Instruction::CleanupTrackedScope { .. }
            | Instruction::ReplacePlace { .. }
            | Instruction::ReplaceField { .. }
            | Instruction::ListNew { .. }
            | Instruction::ListLength { .. }
            | Instruction::ListPush { .. }
            | Instruction::ListPushPlace { .. }
            | Instruction::ListRemove { .. }
            | Instruction::ListRemovePlace { .. }
            | Instruction::ListIndex { .. }
            | Instruction::ListGet { .. }
            | Instruction::ListReplace { .. }
            | Instruction::ListTryRemove { .. }
            | Instruction::ListClear { .. }
            | Instruction::ListReserve { .. }
            | Instruction::ListTryReserve { .. }
            | Instruction::TextByteLength { .. }
            | Instruction::TextIsEmpty { .. }
            | Instruction::TextConcat { .. }
            | Instruction::ConstructStruct { .. }
            | Instruction::ReadStructField { .. }
            | Instruction::BeginLifecycle { .. }
            | Instruction::EndLifecycle { .. }
            | Instruction::AllocateEntity { .. }
            | Instruction::EntityToLink { .. }
            | Instruction::OpenView { .. }
            | Instruction::ReadField { .. }
            | Instruction::WriteField { .. }
            | Instruction::CloseView { .. }
            | Instruction::KeepEntity { .. }
            | Instruction::RetireEntity { .. } => {
                return Err(BackendError::Unsupported(
                    "native scalar lowering encountered a managed or call instruction".to_owned(),
                ));
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
    ) -> Result<(), BackendError> {
        let Some(callee) = self.module.functions.get(function_id.0 as usize) else {
            return Err(BackendError::Unsupported(format!(
                "call targets unknown function {}",
                function_id.0
            )));
        };
        if argument_sources.iter().any(|(_, source)| source.is_some()) {
            return Err(BackendError::Unsupported(
                "scalar calls cannot lower projected argument sources".to_owned(),
            ));
        }
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
            let expected = llvm_type_for_register(callee, *parameter)?;
            let actual = llvm_type_for_register(self.function, *argument)?;
            if expected != actual {
                return Err(BackendError::Unsupported(
                    "scalar call argument type does not match callee".to_owned(),
                ));
            }
            call_arguments.push(format!("{expected} {}", register_name(*argument)));
        }
        let result_slot = match (&callee.return_type, dst) {
            (IrType::Unit, None) => None,
            (IrType::Unit, Some(_)) => {
                return Err(BackendError::Unsupported(
                    "Unit call unexpectedly has a result register".to_owned(),
                ));
            }
            (return_type, Some(_)) => {
                let ty = scalar_type(return_type).ok_or_else(|| {
                    BackendError::Unsupported(
                        "scalar call return type is not representable".to_owned(),
                    )
                })?;
                let slot = format!("%call_result_{}_{}", function_id.0, self.lines.len());
                self.line(format!("{slot} = alloca {ty}"));
                call_arguments.push(format!("ptr {slot}"));
                Some((slot, ty))
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
        if let Some((slot, ty)) = result_slot
            && let Some(dst) = dst
        {
            self.line(format!("{} = load {ty}, ptr {slot}", register_name(dst)));
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
                let ty = llvm_type_for_register(self.function, *register)?;
                if matches!(self.function.return_type, IrType::Unit | IrType::Lifecycle)
                    || scalar_type(&self.function.return_type) != Some(ty)
                {
                    return Err(BackendError::Unsupported(
                        "native scalar return type does not match the function".to_owned(),
                    ));
                }
                self.line(format!(
                    "store {ty} {}, ptr %out_value",
                    register_name(*register)
                ));
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
            Terminator::ResolveLink { .. } => {
                return Err(BackendError::Unsupported(
                    "native scalar lowering cannot resolve links".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn finish(mut self) -> String {
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
        self.lines.join("\n")
    }
}

#[allow(clippy::too_many_lines)]
fn lower_scalar_ir(module: &Module, metadata: &SourceMetadata) -> Result<String, BackendError> {
    if !module.definitions.is_empty() {
        return Err(BackendError::Unsupported(
            "native scalar lowering requires definition-free functions".to_owned(),
        ));
    }
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
        if function.return_type != IrType::Unit && scalar_type(&function.return_type).is_none() {
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
            if scalar_type(ty).is_none() {
                return Err(BackendError::Unsupported(
                    "scalar function parameters must be Int, Bool, or Lifecycle".to_owned(),
                ));
            }
        }
        for (index, ty) in function.register_types.iter().enumerate() {
            if scalar_type(ty).is_none() && *ty != IrType::Lifecycle {
                return Err(BackendError::Unsupported(format!(
                    "register {index} has a non-scalar type"
                )));
            }
        }
    }
    let locations = LocationTable::from_module(module, &metadata.source);
    let mut functions_ir = String::new();
    for function in &module.functions {
        let mut lowerer = ScalarLowerer::new(module, function, &locations);
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
        let body = lowerer.finish();
        let lifecycle_type = llvm_type_for_register(function, function.current_lifecycle)?;
        let mut parameters = vec![
            format!("ptr %context"),
            format!(
                "{lifecycle_type} {}",
                register_name(function.current_lifecycle)
            ),
        ];
        for parameter in &function.parameters {
            let ty = llvm_type_for_register(function, *parameter)?;
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
    let mut ir = format!(
        "target triple = \"{TARGET_TRIPLE}\"\n\n@keld_path = private constant [{path_type_length} x i8] c\"{path_global}\\00\"\n\ndeclare {{ i64, i1 }} @llvm.sadd.with.overflow.i64(i64, i64)\ndeclare {{ i64, i1 }} @llvm.ssub.with.overflow.i64(i64, i64)\ndeclare {{ i64, i1 }} @llvm.smul.with.overflow.i64(i64, i64)\ndeclare i32 @keld_rt_v1_print_int(i64)\ndeclare i32 @keld_rt_v1_print_fault(i32, ptr, i64, i32, i32)\n\n{functions_ir}\ndefine i32 @main() {{\nentry_main:\n  %out_value = alloca i64\n  %out_kind = alloca i32\n  %out_location = alloca i32\n  %status = call i32 @{main_name}(ptr null, {{ i64, i32, i32 }} zeroinitializer, ptr %out_value, ptr %out_kind, ptr %out_location)\n  %ok = icmp eq i32 %status, 0\n  br i1 %ok, label %success, label %status_dispatch\nsuccess:\n  %result = load i64, ptr %out_value\n  %print_status = call i32 @keld_rt_v1_print_int(i64 %result)\n  %print_ok = icmp eq i32 %print_status, 0\n  br i1 %print_ok, label %done, label %internal_main\nstatus_dispatch:\n  %language_fault = icmp eq i32 %status, 1\n  br i1 %language_fault, label %fault_dispatch, label %internal_main\nfault_dispatch:\n  %out_kind_value = load i32, ptr %out_kind\n  %fault_location = load i32, ptr %out_location\n  switch i32 %fault_location, label %fault_unknown [\n",
    );
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
            "fault_location_{}:\n  %fault_status_{} = call i32 @keld_rt_v1_print_fault(i32 %out_kind_value, ptr {}, i64 {}, i32 {}, i32 {})\n  %fault_print_ok_{} = icmp eq i32 %fault_status_{}, 0\n  br i1 %fault_print_ok_{}, label %fault_done, label %internal_main\n",
            location.id,
            location.id,
            path_pointer,
            path_length,
            location.line,
            location.column,
            location.id,
            location.id,
            location.id,
        );
    }
    ir.push_str(
        "fault_unknown:\n  br label %internal_main\nfault_done:\n  ret i32 2\ndone:\n  ret i32 0\ninternal_main:\n  ret i32 70\n}\n",
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
