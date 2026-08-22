from pathlib import Path

path = Path("crates/keld-semantics/src/features.rs")
text = path.read_text()

old = '''        if let Some(feature) = unsupported_feature(node, lexed, source) {\n            let mut diagnostic = Diagnostic::error(\n                FEATURE_DIAGNOSTIC,\n                node.span,\n                format!("`{feature}` is parsed but not supported by the bootstrap compiler"),\n            );\n'''
new = '''        if let Some(feature) = unsupported_feature(node, lexed, source) {\n            let span = if node.kind == SyntaxKind::MatchExpr {\n                direct_token_span(node, lexed, TokenKind::Keyword(Keyword::Match))\n                    .unwrap_or(node.span)\n            } else {\n                node.span\n            };\n            let mut diagnostic = Diagnostic::error(\n                FEATURE_DIAGNOSTIC,\n                span,\n                format!("`{feature}` is parsed but not supported by the bootstrap compiler"),\n            );\n'''
if text.count(old) != 1:
    raise RuntimeError("feature diagnostic construction changed")
text = text.replace(old, new, 1)

marker = '''fn has_direct_kind(node: &SyntaxNode, lexed: &Lexed, expected: TokenKind) -> bool {\n'''
helper = '''fn direct_token_span(\n    node: &SyntaxNode,\n    lexed: &Lexed,\n    expected: TokenKind,\n) -> Option<keld_source::Span> {\n    node.direct_token_ids().find_map(|id| {\n        usize::try_from(id.0)\n            .ok()\n            .and_then(|index| lexed.tokens.get(index))\n            .filter(|token| token.kind == expected)\n            .map(|token| token.span)\n    })\n}\n\n'''
if text.count(marker) != 1:
    raise RuntimeError("has_direct_kind insertion point changed")
text = text.replace(marker, helper + marker, 1)

path.write_text(text)
print("match feature diagnostic now points at the match keyword span")