use keld_flow::FlowModule;
use keld_interpreter::{
    ExecutionResult, Interpreter, InterpreterError, InterpreterFailure, RuntimeFault,
};
use keld_lifecycle::VerifiedFlowModule;
use keld_semantics::TypedModule;
use keld_source::{Diagnostic, SourceId, SourceText, sort_diagnostics};
use keld_storage::VerifiedStorageModule;
use keld_syntax::ParsedFile;
use std::fmt;
use std::path::{Path, PathBuf};

pub struct Compilation {
    pub path: PathBuf,
    pub source: Option<SourceText>,
    pub parsed: Option<ParsedFile>,
    pub typed: Option<TypedModule>,
    pub flow: Option<FlowModule>,
    pub verified: Option<VerifiedFlowModule>,
    pub storage: Option<VerifiedStorageModule>,
    pub ir: Option<keld_ir::Module>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Compilation {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            source: None,
            parsed: None,
            typed: None,
            flow: None,
            verified: None,
            storage: None,
            ir: None,
            diagnostics: Vec::new(),
        }
    }

    fn stop_with(&mut self, mut diagnostics: Vec<Diagnostic>) {
        sort_diagnostics(&mut diagnostics);
        self.diagnostics = diagnostics;
    }
}

pub enum DriverFailure {
    Static(Box<Compilation>),
    Runtime {
        fault: RuntimeFault,
        path: PathBuf,
        source: SourceText,
    },
    Internal(InterpreterError),
}

impl fmt::Debug for DriverFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(compilation) => formatter
                .debug_struct("Static")
                .field("diagnostics", &compilation.diagnostics)
                .finish(),
            Self::Runtime { fault, path, .. } => formatter
                .debug_struct("Runtime")
                .field("fault", fault)
                .field("path", path)
                .finish(),
            Self::Internal(error) => formatter.debug_tuple("Internal").field(error).finish(),
        }
    }
}

impl fmt::Display for DriverFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(compilation) => write!(
                formatter,
                "compilation failed with {} diagnostics",
                compilation.diagnostics.len()
            ),
            Self::Runtime { fault, .. } => fault.fmt(formatter),
            Self::Internal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DriverFailure {}

#[must_use]
pub fn check_source(path: &Path, bytes: Vec<u8>) -> Compilation {
    compile_source(path, bytes)
}

#[must_use]
pub fn compile_source(path: &Path, bytes: Vec<u8>) -> Compilation {
    let mut compilation = Compilation::new(path);
    let source = match SourceText::from_bytes(SourceId(0), bytes) {
        Ok(source) => source,
        Err(diagnostic) => {
            compilation.stop_with(vec![diagnostic]);
            return compilation;
        }
    };
    let parsed = keld_syntax::parse(keld_syntax::lex(&source));
    if !parsed.diagnostics.is_empty() {
        compilation.stop_with(parsed.diagnostics.clone());
        compilation.source = Some(source);
        compilation.parsed = Some(parsed);
        return compilation;
    }
    let analysis = keld_semantics::analyze_parsed(&source, &parsed);
    compilation.source = Some(source);
    compilation.parsed = Some(parsed);
    if !analysis.diagnostics.is_empty() {
        compilation.stop_with(analysis.diagnostics);
        return compilation;
    }
    let Some(typed) = analysis.module else {
        return compilation;
    };
    let flow = keld_flow::lower(&typed);
    compilation.typed = Some(typed);
    compilation.flow = Some(flow.clone());
    let verification = keld_lifecycle::verify(flow);
    if !verification.diagnostics.is_empty() {
        compilation.stop_with(verification.diagnostics);
        return compilation;
    }
    let Some(verified) = verification.module else {
        return compilation;
    };
    compilation.verified = Some(verified);
    let Some(verified) = compilation.verified.as_ref() else {
        return compilation;
    };
    let storage = keld_storage::verify(verified.clone());
    if !storage.diagnostics.is_empty() {
        compilation.stop_with(storage.diagnostics);
        return compilation;
    }
    let Some(storage_module) = storage.module else {
        return compilation;
    };
    let ir = keld_ir::lower(&storage_module);
    compilation.storage = Some(storage_module);
    let diagnostics = keld_ir::validate(&ir);
    compilation.ir = Some(ir);
    if !diagnostics.is_empty() {
        compilation.stop_with(diagnostics);
    }
    compilation
}

/// Compiles and runs source with the bootstrap interpreter.
///
/// # Errors
///
/// Returns the first static diagnostic stage, a Keld runtime fault, or an
/// internal interpreter invariant error.
pub fn run_source(path: &Path, bytes: Vec<u8>) -> Result<ExecutionResult, DriverFailure> {
    let compilation = compile_source(path, bytes);
    if !compilation.diagnostics.is_empty() {
        return Err(DriverFailure::Static(Box::new(compilation)));
    }
    let source = compilation.source.clone().ok_or(DriverFailure::Internal(
        InterpreterError::InvalidState("compiled source text is missing"),
    ))?;
    let ir =
        compilation
            .ir
            .as_ref()
            .ok_or(DriverFailure::Internal(InterpreterError::InvalidState(
                "compiled executable IR is missing",
            )))?;
    let mut interpreter = Interpreter::new(ir).map_err(|failure| {
        map_interpreter_failure(failure, compilation.path.clone(), source.clone())
    })?;
    interpreter
        .run_main()
        .map_err(|failure| map_interpreter_failure(failure, compilation.path, source))
}

fn map_interpreter_failure(
    failure: InterpreterFailure,
    path: PathBuf,
    source: SourceText,
) -> DriverFailure {
    match failure {
        InterpreterFailure::Runtime(fault) => DriverFailure::Runtime {
            fault,
            path,
            source,
        },
        InterpreterFailure::Internal(error) => DriverFailure::Internal(error),
    }
}
