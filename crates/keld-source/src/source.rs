use crate::{Diagnostic, DiagnosticCode, SourceId, Span};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SOURCE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0001");
const UTF8_BOM: char = '\u{feff}';

#[derive(Clone, Debug)]
pub struct SourceText {
    id: SourceId,
    text: Arc<str>,
    line_starts: Arc<[u32]>,
}

impl SourceText {
    /// Builds normalized source text from bytes.
    ///
    /// # Errors
    ///
    /// Returns `KLD0001` when the bytes are not UTF-8 or the normalized source
    /// cannot be represented by Keld byte positions.
    pub fn from_bytes(id: SourceId, bytes: Vec<u8>) -> Result<Self, Diagnostic> {
        let text = String::from_utf8(bytes)
            .map_err(|_| source_diagnostic(id, "source is not valid UTF-8"))?;
        Self::from_valid_utf8(id, &text)
    }

    /// Builds normalized source text from an already valid UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns `KLD0001` when the normalized source cannot be represented by
    /// Keld byte positions.
    pub fn from_str(id: SourceId, text: &str) -> Result<Self, Diagnostic> {
        Self::from_valid_utf8(id, text)
    }

    fn from_valid_utf8(id: SourceId, text: &str) -> Result<Self, Diagnostic> {
        let without_bom = text.strip_prefix(UTF8_BOM).unwrap_or(text);
        let normalized = without_bom.replace("\r\n", "\n");
        if normalized.len() > u32::MAX as usize {
            return Err(source_diagnostic(
                id,
                "normalized source exceeds the maximum byte length",
            ));
        }

        let mut line_starts = Vec::new();
        line_starts.push(0);
        for (index, byte) in normalized.bytes().enumerate() {
            if byte == b'\n' {
                let next = u32::try_from(index + 1)
                    .map_err(|_| source_diagnostic(id, "source line offset exceeds u32"))?;
                line_starts.push(next);
            }
        }

        Ok(Self {
            id,
            text: Arc::from(normalized),
            line_starts: Arc::from(line_starts),
        })
    }

    #[must_use]
    pub const fn id(&self) -> SourceId {
        self.id
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&str> {
        if span.source() != self.id {
            return None;
        }
        self.text
            .get(span.start().0 as usize..span.end().0 as usize)
    }

    #[must_use]
    pub fn line_col(&self, position: crate::BytePos) -> Option<(u32, u32)> {
        let position = position.0 as usize;
        if position > self.text.len() || !self.text.is_char_boundary(position) {
            return None;
        }

        let line_index = self
            .line_starts
            .partition_point(|start| *start as usize <= position)
            .checked_sub(1)?;
        let line_start = self.line_starts[line_index] as usize;
        let line = u32::try_from(line_index).ok()?.checked_add(1)?;
        let column = u32::try_from(self.text[line_start..position].chars().count())
            .ok()?
            .checked_add(1)?;
        Some((line, column))
    }
}

#[derive(Debug)]
struct SourceEntry {
    display_path: PathBuf,
    source: SourceText,
}

#[derive(Debug)]
pub struct SourceMap {
    next_id: Option<u32>,
    entries: Vec<SourceEntry>,
}

impl SourceMap {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_id: Some(0),
            entries: Vec::new(),
        }
    }

    /// Validates and publishes one source under a fresh identity.
    ///
    /// # Errors
    ///
    /// Returns `KLD0001` for invalid source bytes, exhausted source IDs, or
    /// source-map allocation failure. Failure never publishes a partial entry.
    pub fn add(&mut self, display_path: PathBuf, bytes: Vec<u8>) -> Result<SourceId, Diagnostic> {
        let raw_id = self
            .next_id
            .ok_or_else(|| source_diagnostic(SourceId(u32::MAX), "source ID space is exhausted"))?;
        let id = SourceId(raw_id);
        let source = SourceText::from_bytes(id, bytes)?;
        self.entries
            .try_reserve(1)
            .map_err(|_| source_diagnostic(id, "unable to reserve source-map storage"))?;

        self.entries.push(SourceEntry {
            display_path,
            source,
        });
        self.next_id = raw_id.checked_add(1);
        Ok(id)
    }

    #[must_use]
    pub fn source(&self, id: SourceId) -> Option<&SourceText> {
        self.entries
            .iter()
            .find(|entry| entry.source.id == id)
            .map(|entry| &entry.source)
    }

    #[must_use]
    pub fn display_path(&self, id: SourceId) -> Option<&Path> {
        self.entries
            .iter()
            .find(|entry| entry.source.id == id)
            .map(|entry| entry.display_path.as_path())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    const fn with_next_id(next_id: u32) -> Self {
        Self {
            next_id: Some(next_id),
            entries: Vec::new(),
        }
    }
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}

fn source_diagnostic(id: SourceId, message: &str) -> Diagnostic {
    Diagnostic::error(
        SOURCE_DIAGNOSTIC,
        Span::new(id, 0, 0).expect("zero-length source span is valid"),
        message,
    )
}

#[cfg(test)]
mod tests {
    use super::SourceMap;
    use crate::SourceId;
    use std::path::PathBuf;

    #[test]
    fn source_id_exhaustion_is_reported_without_publication() {
        let mut sources = SourceMap::with_next_id(u32::MAX);
        let last = sources
            .add(PathBuf::from("last.keld"), b"last".to_vec())
            .unwrap();
        let exhausted = sources.add(PathBuf::from("overflow.keld"), b"x".to_vec());

        assert_eq!(last, SourceId(u32::MAX));
        assert_eq!(exhausted.unwrap_err().code.0, "KLD0001");
        assert_eq!(sources.len(), 1);
    }
}
