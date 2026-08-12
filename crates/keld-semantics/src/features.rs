use keld_source::{Diagnostic, DiagnosticCode, SourceText};
use keld_syntax::{Keyword, Lexed, SyntaxKind, SyntaxNode, TokenKind};

const FEATURE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0004");

pub(crate) fn gate(root: &SyntaxNode, lexed: &Lexed, source: &SourceText) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut stack = root.child_nodes().rev().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        if let Some(feature) = unsupported_feature(node, lexed, source) {
            diagnostics.push(Diagnostic::error(
                FEATURE_DIAGNOSTIC,
                node.span,
                format!("`{feature}` is parsed but not supported by the bootstrap compiler"),
            ));
            continue;
        }
        stack.extend(node.child_nodes().rev());
    }
    diagnostics
}

fn unsupported_feature<'a>(
    node: &SyntaxNode,
    lexed: &Lexed,
    source: &'a SourceText,
) -> Option<&'a str> {
    let fixed = match node.kind {
        SyntaxKind::UseDecl => Some("use"),
        SyntaxKind::EnumDecl => Some("enum"),
        SyntaxKind::ExternFunctionDecl => Some("extern"),
        SyntaxKind::TypeParameterList | SyntaxKind::TypeArgumentList => Some("generic"),
        SyntaxKind::WhileStmt => Some("while"),
        SyntaxKind::BreakStmt => Some("break"),
        SyntaxKind::ContinueStmt => Some("continue"),
        SyntaxKind::MatchExpr => Some("match"),
        SyntaxKind::TakeExpr => Some("take"),
        SyntaxKind::RaisesClause => Some("raises"),
        SyntaxKind::TryStmt => Some("try"),
        SyntaxKind::LiteralExpr if has_direct_kind(node, lexed, TokenKind::String) => {
            Some("string")
        }
        SyntaxKind::BindingStmt
            if has_direct_kind(node, lexed, TokenKind::Keyword(Keyword::Var)) =>
        {
            Some("var")
        }
        SyntaxKind::Parameter
            if has_direct_kind(node, lexed, TokenKind::Keyword(Keyword::Take)) =>
        {
            Some("take")
        }
        SyntaxKind::ModuleDecl
            if has_direct_kind(node, lexed, TokenKind::Keyword(Keyword::Unsafe)) =>
        {
            Some("unsafe")
        }
        _ => None,
    };
    if fixed.is_some() {
        return fixed;
    }
    if node.kind == SyntaxKind::Type {
        let name = node
            .descendant_nodes()
            .find(|child| child.kind == SyntaxKind::Name)
            .and_then(|name| node_text(name, lexed, source));
        if matches!(name, Some("Text" | "List")) {
            return name;
        }
    }
    None
}

fn has_direct_kind(node: &SyntaxNode, lexed: &Lexed, expected: TokenKind) -> bool {
    node.direct_token_ids().any(|id| {
        usize::try_from(id.0)
            .ok()
            .and_then(|index| lexed.tokens.get(index))
            .is_some_and(|token| token.kind == expected)
    })
}

fn node_text<'a>(node: &SyntaxNode, lexed: &Lexed, source: &'a SourceText) -> Option<&'a str> {
    let id = node.token_ids().next()?;
    let index = usize::try_from(id.0).ok()?;
    let token = lexed.tokens.get(index)?;
    source.slice(token.span)
}
