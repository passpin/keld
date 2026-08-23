use keld_source::{Diagnostic, DiagnosticCode, Span};
use std::collections::BTreeSet;

pub(crate) fn lifecycle_diagnostic(
    code: &'static str,
    span: Span,
    message: impl Into<String>,
    repair: impl Into<String>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(DiagnosticCode(code), span, message);
    diagnostic.help = Some(repair.into());
    diagnostic
}

#[derive(Default)]
pub(crate) struct DiagnosticSink {
    diagnostics: Vec<Diagnostic>,
    seen: BTreeSet<(&'static str, keld_source::SourceId, u32, u32)>,
}

impl DiagnosticSink {
    pub fn push(&mut self, diagnostic: Diagnostic) {
        let span = diagnostic.primary.span;
        let key = (
            diagnostic.code.0,
            span.source(),
            span.start().0,
            span.end().0,
        );
        if self.seen.insert(key) {
            self.diagnostics.push(diagnostic);
        }
    }

    pub fn finish(mut self) -> Vec<Diagnostic> {
        keld_source::sort_diagnostics(&mut self.diagnostics);
        self.diagnostics
    }
}
