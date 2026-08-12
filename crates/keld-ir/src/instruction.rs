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
