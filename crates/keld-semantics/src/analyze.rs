use crate::features;
use crate::symbols::{FunctionSignature, ParameterSignature, RetirementSignature};
use crate::{
    DefId, Definition, DefinitionKind, FieldDefinition, FieldId, FunctionId, HirFunction, TypeId,
    TypeKind, TypeStore, TypedModule,
};
use keld_source::{Diagnostic, DiagnosticCode, SourceId, SourceText, Span, sort_diagnostics};
use keld_syntax::{
    Keyword, Lexed, ParsedFile, Punct, SyntaxKind, SyntaxNode, TokenKind, lex, parse,
};
use std::collections::{BTreeMap, BTreeSet};

const DUPLICATE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0101");
const DUPLICATE_MEMBER_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0102");
const UNKNOWN_NAME_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0103");
const TYPE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0106");
const ENTRY_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0109");
const LAYOUT_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0110");
const FEATURE_DIAGNOSTIC: DiagnosticCode = DiagnosticCode("KLD0004");

pub struct Analysis {
    pub module: Option<TypedModule>,
    pub diagnostics: Vec<Diagnostic>,
}

#[must_use]
pub fn analyze_text(text: &str) -> Analysis {
    match SourceText::from_str(SourceId(0), text) {
        Ok(source) => analyze(&source),
        Err(diagnostic) => Analysis {
            module: None,
            diagnostics: vec![diagnostic],
        },
    }
}

#[must_use]
pub fn analyze(source: &SourceText) -> Analysis {
    let parsed = parse(lex(source));
    if !parsed.diagnostics.is_empty() {
        return Analysis {
            module: None,
            diagnostics: parsed.diagnostics.clone(),
        };
    }

    let feature_diagnostics = features::gate(&parsed.root, &parsed.lexed, source);
    if !feature_diagnostics.is_empty() {
        return Analysis {
            module: None,
            diagnostics: feature_diagnostics,
        };
    }

    Analyzer::new(source, &parsed).run()
}

pub(crate) struct Analyzer<'source, 'syntax> {
    pub(crate) context: SyntaxContext<'source, 'syntax>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) types: TypeStore,
    pub(crate) definitions: Vec<Definition>,
    definition_nodes: Vec<&'syntax SyntaxNode>,
    pub(crate) definition_names: BTreeMap<String, DefId>,
    pub(crate) signatures: Vec<FunctionSignature<'syntax>>,
    pub(crate) function_names: BTreeMap<String, FunctionId>,
    pub(crate) functions: Vec<HirFunction>,
    pub(crate) main: FunctionId,
}

impl<'source, 'syntax> Analyzer<'source, 'syntax> {
    fn new(source: &'source SourceText, parsed: &'syntax ParsedFile) -> Self {
        Self {
            context: SyntaxContext {
                source,
                lexed: &parsed.lexed,
                root: &parsed.root,
            },
            diagnostics: Vec::new(),
            types: TypeStore::new(),
            definitions: Vec::new(),
            definition_nodes: Vec::new(),
            definition_names: BTreeMap::new(),
            signatures: Vec::new(),
            function_names: BTreeMap::new(),
            functions: Vec::new(),
            main: FunctionId(0),
        }
    }

    fn run(mut self) -> Analysis {
        self.collect_definition_headers();
        self.resolve_definition_fields();
        self.collect_function_signatures();
        self.validate_layouts();
        self.validate_entrypoint();
        self.check_function_bodies();
        self.validate_call_cycles();
        sort_diagnostics(&mut self.diagnostics);

        if self.diagnostics.is_empty() {
            Analysis {
                module: Some(TypedModule {
                    definitions: self.definitions,
                    functions: self.functions,
                    types: self.types,
                    main: self.main,
                }),
                diagnostics: Vec::new(),
            }
        } else {
            Analysis {
                module: None,
                diagnostics: self.diagnostics,
            }
        }
    }

    fn collect_definition_headers(&mut self) {
        for node in self.context.root_items() {
            let kind = match node.kind {
                SyntaxKind::StructDecl => DefinitionKind::Struct,
                SyntaxKind::EntityDecl => DefinitionKind::Entity,
                _ => continue,
            };
            let Some(name_node) = direct_child(node, SyntaxKind::Name) else {
                continue;
            };
            let Some(name) = self.context.node_text(name_node) else {
                continue;
            };
            let id = DefId(u32::try_from(self.definitions.len()).unwrap_or(u32::MAX));
            if self.definition_names.contains_key(name) {
                self.error(
                    DUPLICATE_DIAGNOSTIC,
                    name_node.span,
                    format!("duplicate definition `{name}`"),
                );
                continue;
            }
            self.definition_names.insert(name.to_owned(), id);
            self.definition_nodes.push(node);
            self.definitions.push(Definition {
                id,
                name: name.to_owned(),
                kind,
                fields: Vec::new(),
                span: node.span,
            });
        }
    }

    fn resolve_definition_fields(&mut self) {
        let mut next_field = 0_u32;
        for index in 0..self.definition_nodes.len() {
            let node = self.definition_nodes[index];
            let field_nodes = node
                .descendant_nodes()
                .filter(|child| child.kind == SyntaxKind::FieldDecl)
                .collect::<Vec<_>>();
            let mut names = BTreeSet::new();
            let mut fields = Vec::new();
            for field_node in field_nodes {
                let Some(name_node) = direct_child(field_node, SyntaxKind::Name) else {
                    continue;
                };
                let Some(name) = self.context.node_text(name_node).map(str::to_owned) else {
                    continue;
                };
                if !names.insert(name.clone()) {
                    self.error(
                        DUPLICATE_MEMBER_DIAGNOSTIC,
                        name_node.span,
                        format!("duplicate field `{name}`"),
                    );
                    continue;
                }
                let Some(type_node) = direct_child(field_node, SyntaxKind::Type) else {
                    continue;
                };
                let ty = self.resolve_type(type_node);
                fields.push(FieldDefinition {
                    id: FieldId(next_field),
                    name,
                    ty,
                    span: field_node.span,
                });
                next_field = next_field.saturating_add(1);
            }
            self.definitions[index].fields = fields;
        }
    }

    fn collect_function_signatures(&mut self) {
        let function_nodes = self
            .context
            .root_items()
            .filter(|node| node.kind == SyntaxKind::FunctionDecl)
            .collect::<Vec<_>>();
        for node in function_nodes {
            let Some(name_node) = direct_child(node, SyntaxKind::Name) else {
                continue;
            };
            let Some(name) = self.context.node_text(name_node).map(str::to_owned) else {
                continue;
            };
            let id = FunctionId(u32::try_from(self.signatures.len()).unwrap_or(u32::MAX));
            if self.function_names.contains_key(&name) {
                self.error(
                    DUPLICATE_DIAGNOSTIC,
                    name_node.span,
                    format!("duplicate function `{name}`"),
                );
            } else {
                self.function_names.insert(name.clone(), id);
            }

            let mut parameter_names = BTreeSet::new();
            let mut parameters = Vec::new();
            if let Some(list) = direct_child(node, SyntaxKind::ParameterList) {
                for parameter in list
                    .child_nodes()
                    .filter(|child| child.kind == SyntaxKind::Parameter)
                {
                    let Some(parameter_name_node) = direct_child(parameter, SyntaxKind::Name)
                    else {
                        continue;
                    };
                    let Some(parameter_name) = self
                        .context
                        .node_text(parameter_name_node)
                        .map(str::to_owned)
                    else {
                        continue;
                    };
                    if !parameter_names.insert(parameter_name.clone()) {
                        self.error(
                            DUPLICATE_MEMBER_DIAGNOSTIC,
                            parameter_name_node.span,
                            format!("duplicate parameter `{parameter_name}`"),
                        );
                    }
                    let ty = direct_child(parameter, SyntaxKind::Type)
                        .map_or(TypeStore::ERROR, |type_node| self.resolve_type(type_node));
                    parameters.push(ParameterSignature {
                        name: parameter_name,
                        ty,
                    });
                }
            }

            let return_type = direct_child(node, SyntaxKind::ReturnClause)
                .and_then(|clause| direct_child(clause, SyntaxKind::Type))
                .map_or(TypeStore::UNIT, |type_node| self.resolve_type(type_node));
            let retirements = self.resolve_retirements(node);
            self.signatures.push(FunctionSignature {
                id,
                name,
                parameters,
                return_type,
                retirements,
                node,
            });
        }
    }

    fn resolve_retirements(&mut self, function: &'syntax SyntaxNode) -> Vec<RetirementSignature> {
        let mut retirements = Vec::new();
        let Some(clause) = direct_child(function, SyntaxKind::RetiresClause) else {
            return retirements;
        };
        for target in clause
            .child_nodes()
            .filter(|child| child.kind == SyntaxKind::RetirementTarget)
        {
            if self.context.has_direct_keyword(target, Keyword::Any) {
                let Some(type_node) = direct_child(target, SyntaxKind::Type) else {
                    continue;
                };
                let ty = self.resolve_type(type_node);
                if let TypeKind::EntityRef(definition) = self.types.kind(ty) {
                    retirements.push(RetirementSignature::Any(*definition, target.span));
                } else {
                    self.error(
                        TYPE_DIAGNOSTIC,
                        target.span,
                        "`retires any` requires an entity type".to_owned(),
                    );
                }
            } else if let Some(name_node) = direct_child(target, SyntaxKind::Name)
                && let Some(name) = self.context.node_text(name_node)
            {
                retirements.push(RetirementSignature::Parameter(name.to_owned(), target.span));
            }
        }
        retirements
    }

    pub(crate) fn resolve_type(&mut self, node: &SyntaxNode) -> TypeId {
        let linked = self.context.has_direct_keyword(node, Keyword::Link);
        let optional = self.context.has_direct_punct(node, Punct::Question);
        let Some(path) = direct_child(node, SyntaxKind::Path) else {
            return TypeStore::ERROR;
        };
        let Some(name_node) = direct_child(path, SyntaxKind::Name) else {
            return TypeStore::ERROR;
        };
        let Some(name) = self.context.node_text(name_node).map(str::to_owned) else {
            return TypeStore::ERROR;
        };

        if linked {
            let Some(definition) = self.definition_names.get(&name).copied() else {
                self.error(
                    UNKNOWN_NAME_DIAGNOSTIC,
                    name_node.span,
                    format!("unknown entity type `{name}`"),
                );
                return TypeStore::ERROR;
            };
            if self.definitions[definition.0 as usize].kind != DefinitionKind::Entity {
                self.error(
                    TYPE_DIAGNOSTIC,
                    name_node.span,
                    format!("link target `{name}` is not an entity"),
                );
                return TypeStore::ERROR;
            }
            return self.types.intern(TypeKind::Link {
                entity: definition,
                optional,
            });
        }

        if optional {
            self.error(
                FEATURE_DIAGNOSTIC,
                node.span,
                "optional non-link types are not supported by the bootstrap compiler".to_owned(),
            );
        }
        match name.as_str() {
            "Unit" => TypeStore::UNIT,
            "Bool" => TypeStore::BOOL,
            "Int" => TypeStore::INT,
            _ => {
                let Some(definition) = self.definition_names.get(&name).copied() else {
                    self.error(
                        UNKNOWN_NAME_DIAGNOSTIC,
                        name_node.span,
                        format!("unknown type `{name}`"),
                    );
                    return TypeStore::ERROR;
                };
                match self.definitions[definition.0 as usize].kind {
                    DefinitionKind::Struct => self.types.intern(TypeKind::Struct(definition)),
                    DefinitionKind::Entity => self.types.intern(TypeKind::EntityRef(definition)),
                }
            }
        }
    }

    fn validate_layouts(&mut self) {
        let adjacency = self
            .definitions
            .iter()
            .map(|definition| {
                definition
                    .fields
                    .iter()
                    .filter_map(|field| match self.types.kind(field.ty) {
                        TypeKind::Struct(target) => Some(target.0 as usize),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        for start in 0..adjacency.len() {
            if layout_is_invalid(start, &adjacency) {
                self.error(
                    LAYOUT_DIAGNOSTIC,
                    self.definitions[start].span,
                    format!(
                        "by-value layout for `{}` is cyclic or exceeds 256 definitions",
                        self.definitions[start].name
                    ),
                );
            }
        }
    }

    fn validate_entrypoint(&mut self) {
        let mains = self
            .signatures
            .iter()
            .filter(|signature| signature.name == "main")
            .map(|signature| signature.id)
            .collect::<Vec<_>>();
        if mains.len() != 1 {
            let span = self.signatures.first().map_or_else(
                || self.context.empty_span(),
                |signature| signature.node.span,
            );
            self.error(
                ENTRY_DIAGNOSTIC,
                span,
                "program requires exactly one `main` function".to_owned(),
            );
            return;
        }
        self.main = mains[0];
        let signature = &self.signatures[self.main.0 as usize];
        if !signature.parameters.is_empty()
            || signature.return_type != TypeStore::INT
            || !signature.retirements.is_empty()
        {
            self.error(
                ENTRY_DIAGNOSTIC,
                signature.node.span,
                "`main` must have no parameters, return Int, and declare no retirement effects"
                    .to_owned(),
            );
        }
    }

    fn validate_call_cycles(&mut self) {
        let graph = self
            .functions
            .iter()
            .map(crate::check::called_functions)
            .collect::<Vec<_>>();
        for start in 0..graph.len() {
            if reaches(start, start, &graph) {
                self.error(
                    FEATURE_DIAGNOSTIC,
                    self.functions[start].span,
                    format!(
                        "recursive function `{}` is not supported by the bootstrap compiler",
                        self.functions[start].name
                    ),
                );
            }
        }
    }

    pub(crate) fn error(&mut self, code: DiagnosticCode, span: Span, message: String) {
        self.diagnostics
            .push(Diagnostic::error(code, span, message));
    }
}

pub(crate) struct SyntaxContext<'source, 'syntax> {
    pub(crate) source: &'source SourceText,
    pub(crate) lexed: &'syntax Lexed,
    root: &'syntax SyntaxNode,
}

impl<'syntax> SyntaxContext<'_, 'syntax> {
    pub(crate) fn root_items(&self) -> impl Iterator<Item = &'syntax SyntaxNode> + use<'syntax> {
        self.root.child_nodes()
    }

    pub(crate) fn node_text<'a>(&'a self, node: &SyntaxNode) -> Option<&'a str> {
        let token = node.token_ids().find_map(|id| {
            let index = usize::try_from(id.0).ok()?;
            let token = self.lexed.tokens.get(index)?;
            (!is_trivia(token.kind)).then_some(token)
        })?;
        self.source.slice(token.span)
    }

    pub(crate) fn direct_token_kind(&self, node: &SyntaxNode) -> Option<TokenKind> {
        node.direct_token_ids().find_map(|id| {
            let index = usize::try_from(id.0).ok()?;
            let token = self.lexed.tokens.get(index)?;
            (!is_trivia(token.kind)).then_some(token.kind)
        })
    }

    pub(crate) fn has_direct_keyword(&self, node: &SyntaxNode, keyword: Keyword) -> bool {
        self.has_direct_kind(node, TokenKind::Keyword(keyword))
    }

    pub(crate) fn has_direct_punct(&self, node: &SyntaxNode, punct: Punct) -> bool {
        self.has_direct_kind(node, TokenKind::Punct(punct))
    }

    pub(crate) fn has_direct_kind(&self, node: &SyntaxNode, kind: TokenKind) -> bool {
        node.direct_token_ids().any(|id| {
            usize::try_from(id.0)
                .ok()
                .and_then(|index| self.lexed.tokens.get(index))
                .is_some_and(|token| token.kind == kind)
        })
    }

    pub(crate) fn empty_span(&self) -> Span {
        self.lexed.tokens.last().map_or_else(
            || Span::new(self.source.id(), 0, 0).expect("empty source span is valid"),
            |token| token.span,
        )
    }
}

pub(crate) fn direct_child(node: &SyntaxNode, kind: SyntaxKind) -> Option<&SyntaxNode> {
    node.child_nodes().find(|child| child.kind == kind)
}

fn layout_is_invalid(start: usize, adjacency: &[Vec<usize>]) -> bool {
    let mut on_path = vec![false; adjacency.len()];
    let mut stack = vec![(start, 0_usize)];
    on_path[start] = true;
    loop {
        if stack.len() > 256 {
            return true;
        }
        let Some((node, next_edge)) = stack.last_mut() else {
            break;
        };
        if *next_edge >= adjacency[*node].len() {
            on_path[*node] = false;
            stack.pop();
            continue;
        }
        let target = adjacency[*node][*next_edge];
        *next_edge += 1;
        if on_path[target] {
            return true;
        }
        on_path[target] = true;
        stack.push((target, 0));
    }
    false
}

fn reaches(start: usize, target: usize, graph: &[Vec<FunctionId>]) -> bool {
    let mut seen = vec![false; graph.len()];
    let mut stack = graph[start]
        .iter()
        .map(|function| function.0 as usize)
        .collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if node >= graph.len() || seen[node] {
            continue;
        }
        seen[node] = true;
        stack.extend(graph[node].iter().map(|function| function.0 as usize));
    }
    false
}

fn is_trivia(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
    )
}
