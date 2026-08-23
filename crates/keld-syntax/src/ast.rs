use crate::{SyntaxKind, SyntaxNode, TokenId};

pub trait AstNode<'a>: Copy {
    const KIND: SyntaxKind;

    fn cast(node: &'a SyntaxNode) -> Option<Self>;
    fn syntax(self) -> &'a SyntaxNode;
}

macro_rules! node_wrapper {
    ($name:ident, $kind:ident) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $name<'a> {
            node: &'a SyntaxNode,
        }

        impl<'a> $name<'a> {
            #[must_use]
            pub const fn syntax(self) -> &'a SyntaxNode {
                self.node
            }
        }

        impl<'a> AstNode<'a> for $name<'a> {
            const KIND: SyntaxKind = SyntaxKind::$kind;

            fn cast(node: &'a SyntaxNode) -> Option<Self> {
                (node.kind == Self::KIND).then_some(Self { node })
            }

            fn syntax(self) -> &'a SyntaxNode {
                self.node
            }
        }
    };
}

node_wrapper!(SourceFile, SourceFile);
node_wrapper!(ModuleDecl, ModuleDecl);
node_wrapper!(UseDecl, UseDecl);
node_wrapper!(StructDecl, StructDecl);
node_wrapper!(EntityDecl, EntityDecl);
node_wrapper!(EnumDecl, EnumDecl);
node_wrapper!(FunctionDecl, FunctionDecl);
node_wrapper!(ExternFunctionDecl, ExternFunctionDecl);
node_wrapper!(TypeParameterList, TypeParameterList);
node_wrapper!(TypeArgumentList, TypeArgumentList);
node_wrapper!(FieldBlock, FieldBlock);
node_wrapper!(FieldDecl, FieldDecl);
node_wrapper!(VariantBlock, VariantBlock);
node_wrapper!(VariantDecl, VariantDecl);
node_wrapper!(ParameterList, ParameterList);
node_wrapper!(Parameter, Parameter);
node_wrapper!(ReturnClause, ReturnClause);
node_wrapper!(RaisesClause, RaisesClause);
node_wrapper!(RetiresClause, RetiresClause);
node_wrapper!(RetirementTarget, RetirementTarget);
node_wrapper!(TypeRef, Type);
node_wrapper!(Path, Path);
node_wrapper!(Block, Block);
node_wrapper!(BindingStmt, BindingStmt);
node_wrapper!(AssignmentStmt, AssignmentStmt);
node_wrapper!(KeepStmt, KeepStmt);
node_wrapper!(RetireStmt, RetireStmt);
node_wrapper!(ReturnStmt, ReturnStmt);
node_wrapper!(BreakStmt, BreakStmt);
node_wrapper!(ContinueStmt, ContinueStmt);
node_wrapper!(ExprStmt, ExprStmt);
node_wrapper!(LifecycleStmt, LifecycleStmt);
node_wrapper!(WhenStmt, WhenStmt);
node_wrapper!(IfStmt, IfStmt);
node_wrapper!(WhileStmt, WhileStmt);
node_wrapper!(TryStmt, TryStmt);
node_wrapper!(HandleClause, HandleClause);
node_wrapper!(LogicalOrExpr, LogicalOrExpr);
node_wrapper!(LogicalAndExpr, LogicalAndExpr);
node_wrapper!(EqualityExpr, EqualityExpr);
node_wrapper!(ComparisonExpr, ComparisonExpr);
node_wrapper!(ShiftExpr, ShiftExpr);
node_wrapper!(AdditiveExpr, AdditiveExpr);
node_wrapper!(MultiplicativeExpr, MultiplicativeExpr);
node_wrapper!(UnaryExpr, UnaryExpr);
node_wrapper!(TakeExpr, TakeExpr);
node_wrapper!(CallExpr, CallExpr);
node_wrapper!(Argument, Argument);
node_wrapper!(FieldExpr, FieldExpr);
node_wrapper!(IndexExpr, IndexExpr);
node_wrapper!(ParenthesizedExpr, ParenthesizedExpr);
node_wrapper!(MatchExpr, MatchExpr);
node_wrapper!(MatchArm, MatchArm);
node_wrapper!(Pattern, Pattern);
node_wrapper!(LiteralExpr, LiteralExpr);
node_wrapper!(NameExpr, NameExpr);
node_wrapper!(Place, Place);
node_wrapper!(Name, Name);

impl<'a> SourceFile<'a> {
    pub(crate) const fn from_root(node: &'a SyntaxNode) -> Self {
        Self { node }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    Struct(StructDecl<'a>),
    Entity(EntityDecl<'a>),
    Enum(EnumDecl<'a>),
    Function(FunctionDecl<'a>),
    ExternFunction(ExternFunctionDecl<'a>),
}

impl<'a> Item<'a> {
    fn cast(node: &'a SyntaxNode) -> Option<Self> {
        Some(match node.kind {
            SyntaxKind::StructDecl => Self::Struct(StructDecl { node }),
            SyntaxKind::EntityDecl => Self::Entity(EntityDecl { node }),
            SyntaxKind::EnumDecl => Self::Enum(EnumDecl { node }),
            SyntaxKind::FunctionDecl => Self::Function(FunctionDecl { node }),
            SyntaxKind::ExternFunctionDecl => Self::ExternFunction(ExternFunctionDecl { node }),
            _ => return None,
        })
    }

    #[must_use]
    pub const fn syntax(self) -> &'a SyntaxNode {
        match self {
            Self::Struct(value) => value.syntax(),
            Self::Entity(value) => value.syntax(),
            Self::Enum(value) => value.syntax(),
            Self::Function(value) => value.syntax(),
            Self::ExternFunction(value) => value.syntax(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Statement<'a> {
    Binding(BindingStmt<'a>),
    Assignment(AssignmentStmt<'a>),
    Keep(KeepStmt<'a>),
    Retire(RetireStmt<'a>),
    Return(ReturnStmt<'a>),
    Break(BreakStmt<'a>),
    Continue(ContinueStmt<'a>),
    Expression(ExprStmt<'a>),
    Lifecycle(LifecycleStmt<'a>),
    When(WhenStmt<'a>),
    If(IfStmt<'a>),
    While(WhileStmt<'a>),
    Try(TryStmt<'a>),
}

impl<'a> Statement<'a> {
    fn cast(node: &'a SyntaxNode) -> Option<Self> {
        Some(match node.kind {
            SyntaxKind::BindingStmt => Self::Binding(BindingStmt { node }),
            SyntaxKind::AssignmentStmt => Self::Assignment(AssignmentStmt { node }),
            SyntaxKind::KeepStmt => Self::Keep(KeepStmt { node }),
            SyntaxKind::RetireStmt => Self::Retire(RetireStmt { node }),
            SyntaxKind::ReturnStmt => Self::Return(ReturnStmt { node }),
            SyntaxKind::BreakStmt => Self::Break(BreakStmt { node }),
            SyntaxKind::ContinueStmt => Self::Continue(ContinueStmt { node }),
            SyntaxKind::ExprStmt => Self::Expression(ExprStmt { node }),
            SyntaxKind::LifecycleStmt => Self::Lifecycle(LifecycleStmt { node }),
            SyntaxKind::WhenStmt => Self::When(WhenStmt { node }),
            SyntaxKind::IfStmt => Self::If(IfStmt { node }),
            SyntaxKind::WhileStmt => Self::While(WhileStmt { node }),
            SyntaxKind::TryStmt => Self::Try(TryStmt { node }),
            _ => return None,
        })
    }

    #[must_use]
    pub const fn syntax(self) -> &'a SyntaxNode {
        match self {
            Self::Binding(value) => value.syntax(),
            Self::Assignment(value) => value.syntax(),
            Self::Keep(value) => value.syntax(),
            Self::Retire(value) => value.syntax(),
            Self::Return(value) => value.syntax(),
            Self::Break(value) => value.syntax(),
            Self::Continue(value) => value.syntax(),
            Self::Expression(value) => value.syntax(),
            Self::Lifecycle(value) => value.syntax(),
            Self::When(value) => value.syntax(),
            Self::If(value) => value.syntax(),
            Self::While(value) => value.syntax(),
            Self::Try(value) => value.syntax(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Expression<'a> {
    LogicalOr(LogicalOrExpr<'a>),
    LogicalAnd(LogicalAndExpr<'a>),
    Equality(EqualityExpr<'a>),
    Comparison(ComparisonExpr<'a>),
    Shift(ShiftExpr<'a>),
    Additive(AdditiveExpr<'a>),
    Multiplicative(MultiplicativeExpr<'a>),
    Unary(UnaryExpr<'a>),
    Take(TakeExpr<'a>),
    Call(CallExpr<'a>),
    Field(FieldExpr<'a>),
    Index(IndexExpr<'a>),
    Parenthesized(ParenthesizedExpr<'a>),
    Match(MatchExpr<'a>),
    Literal(LiteralExpr<'a>),
    Name(NameExpr<'a>),
}

impl<'a> Expression<'a> {
    #[must_use]
    pub fn cast(node: &'a SyntaxNode) -> Option<Self> {
        Some(match node.kind {
            SyntaxKind::LogicalOrExpr => Self::LogicalOr(LogicalOrExpr { node }),
            SyntaxKind::LogicalAndExpr => Self::LogicalAnd(LogicalAndExpr { node }),
            SyntaxKind::EqualityExpr => Self::Equality(EqualityExpr { node }),
            SyntaxKind::ComparisonExpr => Self::Comparison(ComparisonExpr { node }),
            SyntaxKind::ShiftExpr => Self::Shift(ShiftExpr { node }),
            SyntaxKind::AdditiveExpr => Self::Additive(AdditiveExpr { node }),
            SyntaxKind::MultiplicativeExpr => Self::Multiplicative(MultiplicativeExpr { node }),
            SyntaxKind::UnaryExpr => Self::Unary(UnaryExpr { node }),
            SyntaxKind::TakeExpr => Self::Take(TakeExpr { node }),
            SyntaxKind::CallExpr => Self::Call(CallExpr { node }),
            SyntaxKind::FieldExpr => Self::Field(FieldExpr { node }),
            SyntaxKind::IndexExpr => Self::Index(IndexExpr { node }),
            SyntaxKind::ParenthesizedExpr => Self::Parenthesized(ParenthesizedExpr { node }),
            SyntaxKind::MatchExpr => Self::Match(MatchExpr { node }),
            SyntaxKind::LiteralExpr => Self::Literal(LiteralExpr { node }),
            SyntaxKind::NameExpr => Self::Name(NameExpr { node }),
            _ => return None,
        })
    }
}

impl<'a> SourceFile<'a> {
    #[must_use]
    pub fn module(self) -> Option<ModuleDecl<'a>> {
        child(self.node)
    }

    pub fn uses(self) -> impl Iterator<Item = UseDecl<'a>> {
        children(self.node)
    }

    pub fn items(self) -> impl Iterator<Item = Item<'a>> {
        self.node.child_nodes().filter_map(Item::cast)
    }

    pub fn functions(self) -> impl Iterator<Item = FunctionDecl<'a>> {
        children(self.node)
    }
}

impl<'a> FunctionDecl<'a> {
    #[must_use]
    pub fn name(self) -> Name<'a> {
        child(self.node).unwrap_or(Name { node: self.node })
    }

    pub fn parameters(self) -> impl Iterator<Item = Parameter<'a>> {
        descendants(self.node)
    }

    #[must_use]
    pub fn return_type(self) -> Option<TypeRef<'a>> {
        self.node
            .child_nodes()
            .find(|node| node.kind == SyntaxKind::ReturnClause)
            .and_then(child)
    }

    pub fn retirement_targets(self) -> impl Iterator<Item = RetirementTarget<'a>> {
        descendants(self.node)
    }

    #[must_use]
    pub fn body(self) -> Block<'a> {
        child(self.node).unwrap_or(Block { node: self.node })
    }
}

impl<'a> Block<'a> {
    pub fn statements(self) -> impl Iterator<Item = Statement<'a>> {
        self.node.child_nodes().filter_map(Statement::cast)
    }
}

impl Name<'_> {
    #[must_use]
    pub fn token_id(self) -> TokenId {
        self.node.token_ids().next().unwrap_or(TokenId(u64::MAX))
    }
}

fn child<'a, T: AstNode<'a>>(node: &'a SyntaxNode) -> Option<T> {
    node.child_nodes().find_map(T::cast)
}

fn children<'a, T: AstNode<'a>>(node: &'a SyntaxNode) -> impl Iterator<Item = T> {
    node.child_nodes().filter_map(T::cast)
}

fn descendants<'a, T: AstNode<'a>>(node: &'a SyntaxNode) -> impl Iterator<Item = T> {
    node.descendant_nodes().skip(1).filter_map(T::cast)
}
