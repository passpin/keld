use crate::{IrBlockId, Register, ViewId};
use keld_numeric::{IntBinaryOp, IntUnaryOp};
use keld_semantics::{CompareOp, DefId, FieldId, FunctionId, ParameterIndex};
use keld_source::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewMode {
    Read,
    Edit,
}

#[derive(Clone, Debug)]
pub enum Instruction {
    ConstInt {
        dst: Register,
        value: i64,
        span: Span,
    },
    ConstBool {
        dst: Register,
        value: bool,
        span: Span,
    },
    ConstText {
        dst: Register,
        value: String,
        span: Span,
    },
    ConstNoneLink {
        dst: Register,
        entity: DefId,
        span: Span,
    },
    Copy {
        dst: Register,
        src: Register,
        span: Span,
    },
    Take {
        dst: Register,
        src: Register,
        span: Span,
    },
    InstallHome {
        destination: Register,
        source: Register,
        displaced: Register,
        span: Span,
    },
    MoveHome {
        destination: Register,
        source: Register,
        span: Span,
    },
    DropHome {
        home: Register,
        span: Span,
    },
    DropIfLive {
        home: Register,
        span: Span,
    },
    DropSlot {
        slot: Register,
        span: Span,
    },
    CleanupTrackedScope {
        scope: keld_flow::StorageScopeId,
        span: Span,
    },
    ReplacePlace {
        destination: crate::ArgumentSource,
        source: Register,
        displaced: Register,
        span: Span,
    },
    ReplaceField {
        view: ViewId,
        field: FieldId,
        source: Register,
        displaced: Register,
        span: Span,
    },
    ListNew {
        dst: Register,
        span: Span,
    },
    ListLength {
        dst: Register,
        list: Register,
        span: Span,
    },
    ListPush {
        list: Register,
        value: Register,
        span: Span,
    },
    ListPushPlace {
        list: Register,
        source: crate::ArgumentSource,
        value: Register,
        span: Span,
    },
    ListRemove {
        dst: Register,
        list: Register,
        index: Register,
        span: Span,
    },
    ListRemovePlace {
        dst: Register,
        list: Register,
        source: crate::ArgumentSource,
        index: Register,
        span: Span,
    },
    ListIndex {
        dst: Register,
        receiver: crate::Receiver,
        index: Register,
        span: Span,
    },
    ListGet {
        dst: Register,
        receiver: crate::Receiver,
        index: Register,
        span: Span,
    },
    ListReplace {
        receiver: crate::Receiver,
        index: Register,
        value: Register,
        displaced: Register,
        span: Span,
    },
    ListTryRemove {
        dst: Register,
        receiver: crate::Receiver,
        index: Register,
        span: Span,
    },
    ListClear {
        receiver: crate::Receiver,
        span: Span,
    },
    ListReserve {
        receiver: crate::Receiver,
        additional: Register,
        span: Span,
    },
    ListTryReserve {
        dst: Register,
        receiver: crate::Receiver,
        additional: Register,
        span: Span,
    },
    TextByteLength {
        dst: Register,
        text: Register,
        span: Span,
    },
    TextIsEmpty {
        dst: Register,
        text: Register,
        span: Span,
    },
    TextConcat {
        dst: Register,
        lhs: Register,
        rhs: Register,
        span: Span,
    },
    CheckedUnaryInt {
        dst: Register,
        op: IntUnaryOp,
        src: Register,
        span: Span,
    },
    CheckedBinaryInt {
        dst: Register,
        op: IntBinaryOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    },
    Not {
        dst: Register,
        src: Register,
        span: Span,
    },
    Compare {
        dst: Register,
        op: CompareOp,
        lhs: Register,
        rhs: Register,
        span: Span,
    },
    Phi {
        dst: Register,
        inputs: Vec<(IrBlockId, Register)>,
        span: Span,
    },
    ConstructStruct {
        dst: Register,
        definition: DefId,
        fields: Vec<(FieldId, Register)>,
        span: Span,
    },
    ReadStructField {
        dst: Register,
        base: Register,
        field: FieldId,
        span: Span,
    },
    BeginLifecycle {
        dst: Register,
        parent: Register,
        span: Span,
    },
    EndLifecycle {
        lifecycle: Register,
        span: Span,
    },
    AllocateEntity {
        dst: Register,
        definition: DefId,
        fields: Vec<(FieldId, Register)>,
        lifecycle: Register,
        span: Span,
    },
    EntityToLink {
        dst: Register,
        entity: Register,
        span: Span,
    },
    OpenView {
        view: ViewId,
        entity: Register,
        mode: ViewMode,
        span: Span,
    },
    ReadField {
        dst: Register,
        view: ViewId,
        field: FieldId,
        span: Span,
    },
    WriteField {
        view: ViewId,
        field: FieldId,
        value: Register,
        span: Span,
    },
    CloseView {
        view: ViewId,
        span: Span,
    },
    KeepEntity {
        entity: Register,
        lifecycle: Register,
        span: Span,
    },
    RetireEntity {
        entity: Register,
        span: Span,
    },
    Call {
        dst: Option<Register>,
        function: FunctionId,
        arguments: Vec<(ParameterIndex, Register)>,
        argument_sources: Vec<(ParameterIndex, Option<crate::ArgumentSource>)>,
        current_lifecycle: Register,
        span: Span,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultKind {
    Arithmetic,
    DivisionByZero,
    Shift,
    Allocation,
    Capacity,
}

#[derive(Clone, Debug)]
pub enum Terminator {
    Goto(IrBlockId),
    Branch {
        condition: Register,
        then_block: IrBlockId,
        else_block: IrBlockId,
    },
    ResolveLink {
        link: Register,
        live_value: Register,
        live: IrBlockId,
        absent: IrBlockId,
        span: Span,
    },
    Return(Option<Register>),
    Fault {
        kind: FaultKind,
        span: Span,
    },
    Unreachable,
}
