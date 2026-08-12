use crate::{Lexed, TokenId};
use crate::{SyntaxKind, Token};
use keld_source::{Diagnostic, SourceText, Span, sort_diagnostics};

#[derive(Clone, Debug)]
pub enum SyntaxElement {
    Node(SyntaxNode),
    Token(TokenId),
}

#[derive(Clone, Debug)]
pub struct SyntaxNode {
    pub kind: SyntaxKind,
    pub span: Span,
    pub children: Vec<SyntaxElement>,
}

impl SyntaxNode {
    #[must_use]
    pub fn child_nodes(&self) -> impl DoubleEndedIterator<Item = &Self> {
        self.children.iter().filter_map(|element| match element {
            SyntaxElement::Node(node) => Some(node),
            SyntaxElement::Token(_) => None,
        })
    }

    #[must_use]
    pub fn descendant_nodes(&self) -> DescendantNodes<'_> {
        DescendantNodes { stack: vec![self] }
    }

    pub fn descendant_kinds(&self) -> impl Iterator<Item = &SyntaxKind> {
        self.descendant_nodes().map(|node| &node.kind)
    }

    #[must_use]
    pub fn token_ids(&self) -> TokenIds<'_> {
        TokenIds {
            stack: vec![self.children.iter()],
        }
    }

    pub fn direct_token_ids(&self) -> impl Iterator<Item = TokenId> + '_ {
        self.children.iter().filter_map(|element| match element {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(id) => Some(*id),
        })
    }
}

pub struct DescendantNodes<'a> {
    stack: Vec<&'a SyntaxNode>,
}

impl<'a> Iterator for DescendantNodes<'a> {
    type Item = &'a SyntaxNode;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.stack.extend(node.child_nodes().rev());
        Some(node)
    }
}

pub struct TokenIds<'a> {
    stack: Vec<std::slice::Iter<'a, SyntaxElement>>,
}

impl Iterator for TokenIds<'_> {
    type Item = TokenId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let element = self.stack.last_mut()?.next();
            match element {
                Some(SyntaxElement::Token(id)) => return Some(*id),
                Some(SyntaxElement::Node(node)) => self.stack.push(node.children.iter()),
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

pub struct ParsedFile {
    pub lexed: Lexed,
    pub root: SyntaxNode,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedFile {
    #[must_use]
    pub fn reconstruct(&self, source: &SourceText) -> String {
        let mut reconstructed = String::with_capacity(source.text().len());
        for id in self.root.token_ids() {
            let Some(token) = usize::try_from(id.0)
                .ok()
                .and_then(|index| self.lexed.tokens.get(index))
            else {
                continue;
            };
            if !token.synthetic
                && let Some(text) = source.slice(token.span)
            {
                reconstructed.push_str(text);
            }
        }
        reconstructed
    }

    #[must_use]
    pub fn ast(&self) -> crate::ast::SourceFile<'_> {
        crate::ast::SourceFile::from_root(&self.root)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Event {
    Start {
        kind: Option<SyntaxKind>,
        forward_parent: Option<usize>,
    },
    Token(TokenId),
    Finish,
    Error(Diagnostic),
    Tombstone,
}

pub(crate) fn build_tree(lexed: Lexed, mut events: Vec<Event>, fallback_span: Span) -> ParsedFile {
    let mut stack: Vec<(SyntaxKind, Vec<SyntaxElement>)> = Vec::new();
    let mut root = None;
    let mut parser_diagnostics = Vec::new();

    for index in 0..events.len() {
        let event = std::mem::replace(&mut events[index], Event::Tombstone);
        match event {
            Event::Start {
                kind,
                forward_parent,
            } => {
                let mut kinds = Vec::new();
                if let Some(kind) = kind {
                    kinds.push(kind);
                }
                let mut next = forward_parent.map(|distance| index + distance);
                while let Some(parent_index) = next {
                    let parent = std::mem::replace(&mut events[parent_index], Event::Tombstone);
                    if let Event::Start {
                        kind,
                        forward_parent,
                    } = parent
                    {
                        if let Some(kind) = kind {
                            kinds.push(kind);
                        }
                        next = forward_parent.map(|distance| parent_index + distance);
                    } else {
                        debug_assert!(false, "forward parent must point to a start event");
                        break;
                    }
                }
                for kind in kinds.into_iter().rev() {
                    stack.push((kind, Vec::new()));
                }
            }
            Event::Token(id) => {
                if let Some((_, children)) = stack.last_mut() {
                    children.push(SyntaxElement::Token(id));
                }
            }
            Event::Finish => {
                if let Some((kind, children)) = stack.pop() {
                    let span = node_span(&children, &lexed.tokens).unwrap_or(fallback_span);
                    let node = SyntaxNode {
                        kind,
                        span,
                        children,
                    };
                    if let Some((_, parent_children)) = stack.last_mut() {
                        parent_children.push(SyntaxElement::Node(node));
                    } else {
                        root = Some(node);
                    }
                }
            }
            Event::Error(diagnostic) => parser_diagnostics.push(diagnostic),
            Event::Tombstone => {}
        }
    }

    let root = root.unwrap_or_else(|| SyntaxNode {
        kind: SyntaxKind::SourceFile,
        span: fallback_span,
        children: Vec::new(),
    });
    let mut diagnostics = lexed.diagnostics.clone();
    diagnostics.extend(parser_diagnostics);
    sort_diagnostics(&mut diagnostics);
    ParsedFile {
        lexed,
        root,
        diagnostics,
    }
}

fn node_span(children: &[SyntaxElement], tokens: &[Token]) -> Option<Span> {
    let first = children
        .iter()
        .find_map(|element| element_span(element, tokens))?;
    let last = children
        .iter()
        .rev()
        .find_map(|element| element_span(element, tokens))?;
    first.cover(last)
}

fn element_span(element: &SyntaxElement, tokens: &[Token]) -> Option<Span> {
    match element {
        SyntaxElement::Node(node) => Some(node.span),
        SyntaxElement::Token(id) => usize::try_from(id.0)
            .ok()
            .and_then(|index| tokens.get(index))
            .map(|token| token.span),
    }
}
