use keld_source::{Diagnostic, Span};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeFaultKind {
    Arithmetic,
    DivisionByZero,
    Shift,
    Allocation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeFault {
    pub kind: RuntimeFaultKind,
    pub span: Span,
}

impl fmt::Display for RuntimeFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} at byte {}", self.kind, self.span.start().0)
    }
}

impl std::error::Error for RuntimeFault {}

#[derive(Debug)]
pub enum InterpreterError {
    InvalidIr(Vec<Diagnostic>),
    InvalidState(&'static str),
    Store(keld_runtime::StoreError),
}

impl fmt::Display for InterpreterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIr(diagnostics) => {
                write!(
                    formatter,
                    "executable IR failed validation ({} diagnostics)",
                    diagnostics.len()
                )
            }
            Self::InvalidState(message) => formatter.write_str(message),
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InterpreterError {}

#[derive(Debug)]
pub enum InterpreterFailure {
    Runtime(RuntimeFault),
    Internal(InterpreterError),
}

impl fmt::Display for InterpreterFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(fault) => fault.fmt(formatter),
            Self::Internal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InterpreterFailure {}
