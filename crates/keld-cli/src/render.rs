use crate::Compilation;
use keld_interpreter::{RuntimeFault, RuntimeFaultKind};
use keld_source::{Diagnostic, SourceText};
use std::fmt::Write;
use std::path::Path;

pub(crate) fn diagnostics(compilation: &Compilation) -> String {
    let mut output = String::new();
    for diagnostic in &compilation.diagnostics {
        write_diagnostic(
            &mut output,
            &compilation.path,
            compilation.source.as_ref(),
            diagnostic,
        );
    }
    output
}

fn write_diagnostic(
    output: &mut String,
    path: &Path,
    source: Option<&SourceText>,
    diagnostic: &Diagnostic,
) {
    let (line, column) = source
        .and_then(|source| source.line_col(diagnostic.primary.span.start()))
        .unwrap_or((1, 1));
    writeln!(
        output,
        "{}:{line}:{column}: error[{}]: {}",
        path.display(),
        diagnostic.code.0,
        diagnostic.primary.message
    )
    .expect("writing to String cannot fail");
    writeln!(output, "  --> {}", diagnostic.primary.message)
        .expect("writing to String cannot fail");
    for secondary in &diagnostic.secondary {
        writeln!(output, "  = note: {}", secondary.message).expect("writing to String cannot fail");
    }
    if let Some(help) = &diagnostic.help {
        writeln!(output, "  = help: {help}").expect("writing to String cannot fail");
    }
}

pub(crate) fn runtime_fault(path: &Path, source: &SourceText, fault: &RuntimeFault) -> String {
    let (line, column) = source.line_col(fault.span.start()).unwrap_or((1, 1));
    format!(
        "{}:{line}:{column}: runtime[{}]: {}\n",
        path.display(),
        runtime_name(fault.kind),
        runtime_message(fault.kind)
    )
}

const fn runtime_name(kind: RuntimeFaultKind) -> &'static str {
    match kind {
        RuntimeFaultKind::Arithmetic => "ArithmeticFault",
        RuntimeFaultKind::DivisionByZero => "DivisionByZeroFault",
        RuntimeFaultKind::Shift => "ShiftFault",
        RuntimeFaultKind::Allocation => "AllocationFault",
        RuntimeFaultKind::Capacity => "CapacityFault",
        RuntimeFaultKind::Bounds => "BoundsFault",
    }
}

const fn runtime_message(kind: RuntimeFaultKind) -> &'static str {
    match kind {
        RuntimeFaultKind::Arithmetic => "checked integer arithmetic overflow",
        RuntimeFaultKind::DivisionByZero => "integer division or remainder by zero",
        RuntimeFaultKind::Shift => "invalid integer shift amount",
        RuntimeFaultKind::Allocation => "runtime allocation failed",
        RuntimeFaultKind::Capacity => "requested list capacity is impossible",
        RuntimeFaultKind::Bounds => "list index is out of bounds",
    }
}
