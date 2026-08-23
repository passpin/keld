use crate::{
    BindingMutability, DefId, FieldId, FunctionId, HirLifecycleId, LocalId, ParameterIndex,
    ParameterMode, TypeId, TypeStore,
};
use keld_numeric::{IntBinaryOp, IntUnaryOp};
use keld_source::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionKind {
    Struct,
    Entity,
}

#[derive(Clone, Debug)]
pub struct TypedModule {
    pub definitions: Vec<Definition>,
    pub functions: Vec<HirFunction>,
    pub types: TypeStore,
    pub main: FunctionId,
}

#[derive(Clone, Debug)]
pub struct Definition {
    pub id: DefId,
    pub name: String,
    pub kind: DefinitionKind,
    pub fields: Vec<FieldDefinition>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldDefinition {
    pub id: FieldId,
    pub name: String,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionEffects {
    pub retires: Vec<LocalId>,
    pub retires_any: Vec<DefId>,
    pub retires_spans: Vec<Span>,
    pub retires_any_spans: Vec<Span>,
}

#[derive(Clone, Debug)]
pub struct HirFunction {
    pub id: FunctionId,
    pub name: String,
    pub parameters: Vec<(LocalId, TypeId)>,
    pub parameter_modes: Vec<ParameterMode>,
    pub parameter_names: Vec<String>,
    pub return_type: TypeId,
    pub effects: FunctionEffects,
    pub body: HirBlock,
    pub span: Span,
    pub local_types: Vec<TypeId>,
    pub local_mutability: Vec<BindingMutability>,
}

#[derive(Clone, Debug)]
pub struct HirBlock {
    pub statements: Vec<HirStmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct HirPlace {
    pub base: LocalId,
    pub projections: Vec<HirProjection>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum HirProjection {
    Field(FieldId),
    Index(HirExpr),
}

#[derive(Clone, Debug)]
pub struct HirIf {
    pub condition: HirExpr,
    pub then_block: HirBlock,
    pub else_block: Option<HirBlock>,
}

#[derive(Clone, Debug)]
pub struct HirWhile {
    pub condition: HirExpr,
    pub body: HirBlock,
}

#[derive(Clone, Debug)]
pub struct HirWhen {
    pub link: HirExpr,
    pub binding: LocalId,
    pub live: HirBlock,
    pub absent: Option<HirBlock>,
}

#[derive(Clone, Debug)]
pub struct HirLifecycle {
    pub id: HirLifecycleId,
    pub name: String,
    pub body: HirBlock,
}

#[derive(Clone, Debug)]
pub struct HirExpr {
    pub ty: TypeId,
    pub span: Span,
    pub kind: HirExprKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HirUnaryOp {
    Int(IntUnaryOp),
    Not,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HirBinaryOp {
    Int(IntBinaryOp),
    TextConcat,
    And,
    Or,
    Compare(CompareOp),
}

#[derive(Clone, Debug)]
pub enum HirExprKind {
    Int(i64),
    Bool(bool),
    TextLiteral(String),
    TextByteLength(Box<HirExpr>),
    TextIsEmpty(Box<HirExpr>),
    None,
    Local(LocalId),
    Take(HirPlace),
    Copy(Box<HirExpr>),
    ListNew,
    ListLength(Box<HirExpr>),
    ListIndex {
        list: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    ListGet {
        list: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    ListPush {
        list: Box<HirExpr>,
        value: Box<HirExpr>,
    },
    ListRemove {
        list: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    ListTryRemove {
        list: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    ListClear(Box<HirExpr>),
    ListReserve {
        list: Box<HirExpr>,
        additional: Box<HirExpr>,
    },
    ListTryReserve {
        list: Box<HirExpr>,
        additional: Box<HirExpr>,
    },
    Unary {
        op: HirUnaryOp,
        value: Box<HirExpr>,
    },
    Binary {
        op: HirBinaryOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },
    Field {
        base: Box<HirExpr>,
        field: FieldId,
    },
    UncheckedLinkField {
        link: Box<HirExpr>,
        field: FieldId,
    },
    Call {
        function: FunctionId,
        arguments: Vec<(ParameterIndex, HirExpr)>,
    },
    ConstructStruct {
        definition: DefId,
        fields: Vec<(FieldId, HirExpr)>,
    },
    ConstructEntity {
        definition: DefId,
        fields: Vec<(FieldId, HirExpr)>,
    },
    EntityToLink(Box<HirExpr>),
}

#[derive(Clone, Debug)]
pub enum HirStmtKind {
    Let {
        local: LocalId,
        initializer: HirExpr,
    },
    Var {
        local: LocalId,
        initializer: Option<HirExpr>,
    },
    Assign {
        target: HirPlace,
        value: HirExpr,
    },
    CompoundAssign {
        target: HirPlace,
        op: IntBinaryOp,
        value: HirExpr,
    },
    Expr(HirExpr),
    If(HirIf),
    While(HirWhile),
    Break,
    Continue,
    When(HirWhen),
    Lifecycle(HirLifecycle),
    Keep {
        entity: HirExpr,
        lifecycle: HirLifecycleId,
    },
    Retire(HirExpr),
    Return(Option<HirExpr>),
}
