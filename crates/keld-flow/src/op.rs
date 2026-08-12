use crate::{AllocationSite, BlockId, LifecycleId, StorageScopeId, ValueId};
use keld_numeric::{IntBinaryOp, IntUnaryOp};
use keld_semantics::{CompareOp, DefId, FieldId, FunctionId, LocalId, ParameterIndex};
use keld_source::Span;

#[derive(Clone, Debug)]
pub enum ExitTarget {
    Goto(BlockId),
    Return(Option<ValueId>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Place {
    pub base: LocalId,
    pub fields: Vec<FieldId>,
}

#[derive(Clone, Debug)]
pub enum FlowOp {
    ConstInt {
        dst: ValueId,
        value: i64,
        span: Span,
    },
    ConstBool {
        dst: ValueId,
        value: bool,
        span: Span,
    },
    ConstText {
        dst: ValueId,
        value: String,
        span: Span,
    },
    ConstNoneLink {
        dst: ValueId,
        entity: DefId,
        span: Span,
    },
    BeginLifecycle {
        lifecycle: LifecycleId,
        parent: LifecycleId,
        span: Span,
    },
    BeginCall {
        call: u32,
        function: FunctionId,
        span: Span,
    },
    ReserveArgument {
        call: u32,
        parameter: ParameterIndex,
        value: ValueId,
        place: Option<Place>,
        span: Span,
    },
    CopyLocal {
        dst: ValueId,
        local: LocalId,
        span: Span,
    },
    StoreLocal {
        local: LocalId,
        value: ValueId,
        span: Span,
    },
    TakeLocal {
        dst: ValueId,
        local: LocalId,
        span: Span,
    },
    CopyStorage {
        dst: ValueId,
        source: ValueId,
        span: Span,
    },
    ListNew {
        dst: ValueId,
        span: Span,
    },
    ListLength {
        dst: ValueId,
        list: ValueId,
        span: Span,
    },
    ListPush {
        list: ValueId,
        value: ValueId,
        span: Span,
    },
    ListPushPlace {
        list: ValueId,
        place: Place,
        value: ValueId,
        span: Span,
    },
    ListLengthLocal {
        dst: ValueId,
        local: LocalId,
        span: Span,
    },
    ListPushLocal {
        local: LocalId,
        value: ValueId,
        span: Span,
    },
    ListRemove {
        dst: ValueId,
        list: ValueId,
        index: ValueId,
        span: Span,
    },
    ListRemovePlace {
        dst: ValueId,
        list: ValueId,
        place: Place,
        index: ValueId,
        span: Span,
    },
    ListRemoveLocal {
        dst: ValueId,
        local: LocalId,
        index: ValueId,
        span: Span,
    },
    TextByteLength {
        dst: ValueId,
        text: ValueId,
        span: Span,
    },
    TextIsEmpty {
        dst: ValueId,
        text: ValueId,
        span: Span,
    },
    TextConcat {
        dst: ValueId,
        lhs: ValueId,
        rhs: ValueId,
        span: Span,
    },
    UnaryInt {
        dst: ValueId,
        op: IntUnaryOp,
        value: ValueId,
        span: Span,
    },
    BinaryInt {
        dst: ValueId,
        op: IntBinaryOp,
        lhs: ValueId,
        rhs: ValueId,
        span: Span,
    },
    Not {
        dst: ValueId,
        value: ValueId,
        span: Span,
    },
    Compare {
        dst: ValueId,
        op: CompareOp,
        lhs: ValueId,
        rhs: ValueId,
        span: Span,
    },
    Phi {
        dst: ValueId,
        inputs: Vec<(BlockId, ValueId)>,
        span: Span,
    },
    ConstructStruct {
        dst: ValueId,
        definition: DefId,
        fields: Vec<(FieldId, ValueId)>,
        span: Span,
    },
    AllocateEntity {
        dst: ValueId,
        definition: DefId,
        fields: Vec<(FieldId, ValueId)>,
        lifecycle: LifecycleId,
        site: AllocationSite,
        span: Span,
    },
    EntityToLink {
        dst: ValueId,
        entity: ValueId,
        span: Span,
    },
    ReadStructField {
        dst: ValueId,
        base: ValueId,
        field: FieldId,
        span: Span,
    },
    ReadEntityField {
        dst: ValueId,
        entity: ValueId,
        field: FieldId,
        span: Span,
    },
    ReadUncheckedLinkField {
        dst: ValueId,
        link: ValueId,
        field: FieldId,
        span: Span,
    },
    WriteEntityField {
        entity: ValueId,
        field: FieldId,
        value: ValueId,
        span: Span,
    },
    Call {
        call: u32,
        dst: Option<ValueId>,
        function: FunctionId,
        arguments: Vec<(ParameterIndex, ValueId)>,
        argument_places: Vec<(ParameterIndex, Option<Place>)>,
        current_lifecycle: LifecycleId,
        span: Span,
    },
    Keep {
        entity: ValueId,
        target: LifecycleId,
        span: Span,
    },
    Retire {
        entity: ValueId,
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub enum Terminator {
    Goto(BlockId),
    Branch {
        condition: ValueId,
        then_block: BlockId,
        else_block: BlockId,
    },
    BranchIdentity {
        lhs: ValueId,
        rhs: ValueId,
        equal: BlockId,
        not_equal: BlockId,
    },
    ResolveLink {
        link: ValueId,
        bind_local: LocalId,
        live: BlockId,
        absent: BlockId,
        span: Span,
    },
    ExitScopes {
        storage_scopes: Vec<StorageScopeId>,
        lifecycles: Vec<LifecycleId>,
        next: ExitTarget,
    },
    Return(Option<ValueId>),
    Unreachable,
}
