use keld_source::{
    BytePos, Diagnostic, DiagnosticCode, SourceId, SourceMap, SourceText, Span, sort_diagnostics,
};
use std::path::{Path, PathBuf};

#[test]
fn source_normalizes_bom_and_crlf() {
    let source = SourceText::from_bytes(
        SourceId(7),
        b"\xEF\xBB\xBFlet x = 1\r\nreturn x\r\n".to_vec(),
    )
    .unwrap();
    assert_eq!(source.id(), SourceId(7));
    assert_eq!(source.text(), "let x = 1\nreturn x\n");
    let span = Span::new(SourceId(7), 10, 11).unwrap();
    assert_eq!(source.line_col(span.start()), Some((2, 1)));
}

#[test]
fn diagnostics_sort_by_source_then_span_then_code() {
    let mut items = vec![
        Diagnostic::error(
            DiagnosticCode("KLD0102"),
            Span::new(SourceId(0), 9, 10).unwrap(),
            "b",
        ),
        Diagnostic::error(
            DiagnosticCode("KLD0101"),
            Span::new(SourceId(0), 2, 3).unwrap(),
            "a",
        ),
    ];
    sort_diagnostics(&mut items);
    assert_eq!(items[0].code.0, "KLD0101");
}

#[test]
fn invalid_utf8_is_a_source_diagnostic() {
    let diagnostic = SourceText::from_bytes(SourceId(3), vec![0xff]).unwrap_err();
    assert_eq!(diagnostic.code.0, "KLD0001");
    assert_eq!(diagnostic.primary.span.source(), SourceId(3));
}

#[test]
fn line_columns_count_unicode_scalars_and_validate_boundaries() {
    let source = SourceText::from_str(SourceId(0), "a\tβ\n😀z").unwrap();
    assert_eq!(source.line_col(BytePos(0)), Some((1, 1)));
    assert_eq!(source.line_col(BytePos(1)), Some((1, 2)));
    assert_eq!(source.line_col(BytePos(2)), Some((1, 3)));
    assert_eq!(source.line_col(BytePos(4)), Some((1, 4)));
    assert_eq!(source.line_col(BytePos(5)), Some((2, 1)));
    assert_eq!(source.line_col(BytePos(9)), Some((2, 2)));
    assert_eq!(source.line_col(BytePos(10)), Some((2, 3)));
    assert_eq!(source.line_col(BytePos(3)), None);
    assert_eq!(source.line_col(BytePos(11)), None);
}

#[test]
fn spans_and_slices_reject_invalid_ranges() {
    let source = SourceText::from_str(SourceId(4), "aβc").unwrap();
    let beta = Span::new(SourceId(4), 1, 3).unwrap();
    assert_eq!(source.slice(beta), Some("β"));
    assert_eq!(source.slice(Span::new(SourceId(5), 1, 3).unwrap()), None);
    assert_eq!(source.slice(Span::new(SourceId(4), 2, 3).unwrap()), None);
    assert_eq!(Span::new(SourceId(4), 3, 2), None);
    assert_eq!(
        beta.cover(Span::new(SourceId(4), 0, 1).unwrap()),
        Span::new(SourceId(4), 0, 3)
    );
    assert_eq!(beta.cover(Span::new(SourceId(5), 0, 1).unwrap()), None);
}

#[test]
fn source_map_publishes_only_valid_sources_with_distinct_ids() {
    let mut sources = SourceMap::new();
    assert!(sources.is_empty());
    let first = sources
        .add(PathBuf::from("same.keld"), b"first\n".to_vec())
        .unwrap();
    let invalid = sources.add(PathBuf::from("bad.keld"), vec![0xff]);
    let second = sources
        .add(PathBuf::from("same.keld"), b"second\n".to_vec())
        .unwrap();

    assert_eq!(first, SourceId(0));
    assert_eq!(invalid.unwrap_err().code.0, "KLD0001");
    assert_eq!(second, SourceId(1));
    assert_eq!(sources.source(first).unwrap().text(), "first\n");
    assert_eq!(sources.display_path(second), Some(Path::new("same.keld")));
    assert_eq!(sources.len(), 2);
    assert!(!sources.is_empty());
}
