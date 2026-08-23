use crate::{FaultKind, Instruction, IrType, Module, Terminator, ViewMode};
use keld_numeric::{IntBinaryOp, IntUnaryOp};
use keld_semantics::CompareOp;
use keld_source::Span;
use std::fmt::Write;

pub(crate) fn dump(module: &Module) -> String {
    let mut output = String::new();
    for definition in &module.definitions {
        writeln!(
            output,
            "definition d{} {}",
            definition.id.0,
            match definition.kind {
                crate::IrDefinitionKind::Struct => "struct",
                crate::IrDefinitionKind::Entity => "entity",
            }
        )
        .expect("writing to String cannot fail");
        for (field, ty) in &definition.fields {
            writeln!(output, "  field f{} {}", field.0, type_name(ty))
                .expect("writing to String cannot fail");
        }
    }
    for function in &module.functions {
        write!(
            output,
            "function f{} entry b{} current r{} return {} parameters",
            function.id.0,
            function.entry.0,
            function.current_lifecycle.0,
            type_name(&function.return_type)
        )
        .expect("writing to String cannot fail");
        for parameter in &function.parameters {
            write!(output, " r{}", parameter.0).expect("writing to String cannot fail");
        }
        output.push('\n');
        for (index, ty) in function.register_types.iter().enumerate() {
            writeln!(output, "  register r{index} {}", type_name(ty))
                .expect("writing to String cannot fail");
        }
        for block in &function.blocks {
            writeln!(output, "  block b{}", block.id.0).expect("writing to String cannot fail");
            for instruction in &block.instructions {
                output.push_str("    ");
                write_instruction(&mut output, instruction);
                output.push('\n');
            }
            output.push_str("    ");
            write_terminator(&mut output, &block.terminator, function.span);
            output.push('\n');
        }
    }
    output
}

#[allow(clippy::too_many_lines)]
fn write_instruction(output: &mut String, instruction: &Instruction) {
    match instruction {
        Instruction::ConstInt { dst, value, span } => {
            write!(output, "r{} = int {value} ", dst.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ConstBool { dst, value, span } => {
            write!(output, "r{} = bool {value} ", dst.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ConstText { dst, value, span } => {
            write!(output, "r{} = text {:?} ", dst.0, value)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ConstNoneLink { dst, entity, span } => {
            write!(output, "r{} = none d{} ", dst.0, entity.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::Copy { dst, src, span } => {
            write!(output, "r{} = copy r{} ", dst.0, src.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::Take { dst, src, span } => {
            write!(output, "r{} = take r{} ", dst.0, src.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListNew { dst, span } => {
            write!(output, "r{} = list-new ", dst.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListLength { dst, list, span } => {
            write!(output, "r{} = list-length r{} ", dst.0, list.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListPush { list, value, span } => {
            write!(output, "list-push r{} r{} ", list.0, value.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListPushPlace {
            list,
            source,
            value,
            span,
        } => {
            write!(
                output,
                "list-push-place r{} base r{} projections{} r{} ",
                list.0,
                source.base.0,
                source.projections.len(),
                value.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListRemove {
            dst,
            list,
            index,
            span,
        } => {
            write!(output, "r{} = list-remove r{} r{} ", dst.0, list.0, index.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListRemovePlace {
            dst,
            list,
            source,
            index,
            span,
        } => {
            write!(
                output,
                "r{} = list-remove-place r{} base r{} projections{} r{} ",
                dst.0,
                list.0,
                source.base.0,
                source.projections.len(),
                index.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListIndex {
            dst,
            receiver,
            index,
            span,
        } => {
            write!(
                output,
                "r{} = list-index r{} r{} ",
                dst.0, receiver.list.0, index.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListGet {
            dst,
            receiver,
            index,
            span,
        } => {
            write!(
                output,
                "r{} = list-get r{} r{} ",
                dst.0, receiver.list.0, index.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListReplace {
            receiver,
            index,
            value,
            displaced,
            span,
        } => {
            write!(
                output,
                "list-replace r{} r{} <- r{} displaced r{} ",
                receiver.list.0, index.0, value.0, displaced.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListTryRemove {
            dst,
            receiver,
            index,
            span,
        } => {
            write!(
                output,
                "r{} = list-try-remove r{} r{} ",
                dst.0, receiver.list.0, index.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListClear { receiver, span } => {
            write!(output, "list-clear r{} ", receiver.list.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListReserve {
            receiver,
            additional,
            span,
        } => {
            write!(
                output,
                "list-reserve r{} r{} ",
                receiver.list.0, additional.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ListTryReserve {
            dst,
            receiver,
            additional,
            span,
        } => {
            write!(
                output,
                "r{} = list-try-reserve r{} r{} ",
                dst.0, receiver.list.0, additional.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::TextByteLength { dst, text, span } => {
            write!(output, "r{} = text-byte-length r{} ", dst.0, text.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::TextIsEmpty { dst, text, span } => {
            write!(output, "r{} = text-is-empty r{} ", dst.0, text.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::TextConcat {
            dst,
            lhs,
            rhs,
            span,
        } => {
            write!(output, "r{} = text-concat r{} r{} ", dst.0, lhs.0, rhs.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::CheckedUnaryInt { dst, op, src, span } => {
            write!(
                output,
                "r{} = checked.{} r{} ",
                dst.0,
                unary_name(*op),
                src.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::CheckedBinaryInt {
            dst,
            op,
            lhs,
            rhs,
            span,
        } => {
            write!(
                output,
                "r{} = checked.{} r{} r{} ",
                dst.0,
                binary_name(*op),
                lhs.0,
                rhs.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::Not { dst, src, span } => {
            write!(output, "r{} = not r{} ", dst.0, src.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::Compare {
            dst,
            op,
            lhs,
            rhs,
            span,
        } => {
            write!(
                output,
                "r{} = compare.{} r{} r{} ",
                dst.0,
                compare_name(*op),
                lhs.0,
                rhs.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        _ => write_composite_instruction(output, instruction),
    }
}

fn write_composite_instruction(output: &mut String, instruction: &Instruction) {
    match instruction {
        Instruction::Phi { dst, inputs, span } => {
            write!(output, "r{} = phi", dst.0).expect("writing to String cannot fail");
            for (block, register) in inputs {
                write!(output, " b{}:r{}", block.0, register.0)
                    .expect("writing to String cannot fail");
            }
            output.push(' ');
            write_span(output, *span);
        }
        Instruction::ConstructStruct {
            dst,
            definition,
            fields,
            span,
        } => write_fields(output, "struct", dst.0, definition.0, fields, *span),
        Instruction::ReadStructField {
            dst,
            base,
            field,
            span,
        } => {
            write!(
                output,
                "r{} = struct-field r{} f{} ",
                dst.0, base.0, field.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::BeginLifecycle { dst, parent, span } => {
            write!(output, "r{} = begin-lifecycle r{} ", dst.0, parent.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::EndLifecycle { lifecycle, span } => {
            write!(output, "end-lifecycle r{} ", lifecycle.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::AllocateEntity {
            dst,
            definition,
            fields,
            lifecycle,
            span,
        } => {
            write_fields(output, "entity", dst.0, definition.0, fields, *span);
            write!(output, " lifecycle r{}", lifecycle.0).expect("writing to String cannot fail");
        }
        Instruction::EntityToLink { dst, entity, span } => {
            write!(output, "r{} = link r{} ", dst.0, entity.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        _ => write_effect_instruction(output, instruction),
    }
}

#[allow(clippy::too_many_lines)]
fn write_effect_instruction(output: &mut String, instruction: &Instruction) {
    match instruction {
        Instruction::OpenView {
            view,
            entity,
            mode,
            span,
        } => {
            write!(
                output,
                "open-view v{} r{} {} ",
                view.0,
                entity.0,
                match mode {
                    ViewMode::Read => "read",
                    ViewMode::Edit => "edit",
                }
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ReadField {
            dst,
            view,
            field,
            span,
        } => {
            write!(output, "r{} = view-field v{} f{} ", dst.0, view.0, field.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::WriteField {
            view,
            field,
            value,
            span,
        } => {
            write!(
                output,
                "view-field v{} f{} = r{} ",
                view.0, field.0, value.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::CloseView { view, span } => {
            write!(output, "close-view v{} ", view.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::KeepEntity {
            entity,
            lifecycle,
            span,
        } => {
            write!(output, "keep r{} in r{} ", entity.0, lifecycle.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::RetireEntity { entity, span } => {
            write!(output, "retire r{} ", entity.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::Call {
            dst,
            function,
            arguments,
            current_lifecycle,
            span,
            ..
        } => {
            if let Some(dst) = dst {
                write!(output, "r{} = ", dst.0).expect("writing to String cannot fail");
            }
            write!(output, "call f{}", function.0).expect("writing to String cannot fail");
            for (parameter, register) in arguments {
                write!(output, " p{}:r{}", parameter.0, register.0)
                    .expect("writing to String cannot fail");
            }
            write!(output, " lifecycle r{} ", current_lifecycle.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::InstallHome {
            destination,
            source,
            displaced,
            span,
        } => {
            write!(
                output,
                "install-home r{} <- r{} displaced r{} ",
                destination.0, source.0, displaced.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::MoveHome {
            destination,
            source,
            span,
        } => {
            write!(output, "move-home r{} <- r{} ", destination.0, source.0)
                .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::DropHome { home, span } => {
            write!(output, "drop-home r{} ", home.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::DropIfLive { home, span } => {
            write!(output, "drop-if-live r{} ", home.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::DropSlot { slot, span } => {
            write!(output, "drop-slot r{} ", slot.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::CleanupTrackedScope { scope, span } => {
            write!(output, "cleanup-scope s{} ", scope.0).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ReplacePlace {
            destination,
            source,
            displaced,
            span,
        } => {
            write!(
                output,
                "replace-place base r{} projections{} <- r{} displaced r{} ",
                destination.base.0,
                destination.projections.len(),
                source.0,
                displaced.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ReplaceField {
            view,
            field,
            source,
            displaced,
            span,
        } => {
            write!(
                output,
                "replace-field v{} f{} <- r{} displaced r{} ",
                view.0, field.0, source.0, displaced.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Instruction::ConstInt { .. }
        | Instruction::ConstBool { .. }
        | Instruction::ConstText { .. }
        | Instruction::ConstNoneLink { .. }
        | Instruction::Copy { .. }
        | Instruction::Take { .. }
        | Instruction::ListNew { .. }
        | Instruction::ListLength { .. }
        | Instruction::ListPush { .. }
        | Instruction::ListPushPlace { .. }
        | Instruction::ListRemove { .. }
        | Instruction::ListRemovePlace { .. }
        | Instruction::ListIndex { .. }
        | Instruction::ListGet { .. }
        | Instruction::ListReplace { .. }
        | Instruction::ListTryRemove { .. }
        | Instruction::ListClear { .. }
        | Instruction::ListReserve { .. }
        | Instruction::ListTryReserve { .. }
        | Instruction::TextByteLength { .. }
        | Instruction::TextIsEmpty { .. }
        | Instruction::TextConcat { .. }
        | Instruction::CheckedUnaryInt { .. }
        | Instruction::CheckedBinaryInt { .. }
        | Instruction::Not { .. }
        | Instruction::Compare { .. }
        | Instruction::Phi { .. }
        | Instruction::ConstructStruct { .. }
        | Instruction::ReadStructField { .. }
        | Instruction::BeginLifecycle { .. }
        | Instruction::EndLifecycle { .. }
        | Instruction::AllocateEntity { .. }
        | Instruction::EntityToLink { .. } => {
            unreachable!("value instruction handled by write_instruction")
        }
    }
}

fn write_terminator(output: &mut String, terminator: &Terminator, fallback: Span) {
    match terminator {
        Terminator::Goto(block) => {
            write!(output, "goto b{} ", block.0).expect("writing to String cannot fail");
            write_span(output, fallback);
        }
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => {
            write!(
                output,
                "branch r{} then b{} else b{}",
                condition.0, then_block.0, else_block.0
            )
            .expect("writing to String cannot fail");
            output.push(' ');
            write_span(output, fallback);
        }
        Terminator::ResolveLink {
            link,
            live_value,
            live,
            absent,
            span,
        } => {
            write!(
                output,
                "resolve r{} into r{} live b{} absent b{} ",
                link.0, live_value.0, live.0, absent.0
            )
            .expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Terminator::Return(value) => {
            output.push_str("return");
            if let Some(value) = value {
                write!(output, " r{}", value.0).expect("writing to String cannot fail");
            }
            output.push(' ');
            write_span(output, fallback);
        }
        Terminator::Fault { kind, span } => {
            write!(output, "fault {} ", fault_name(*kind)).expect("writing to String cannot fail");
            write_span(output, *span);
        }
        Terminator::Unreachable => {
            output.push_str("unreachable ");
            write_span(output, fallback);
        }
    }
}

fn write_fields(
    output: &mut String,
    operation: &str,
    destination: u32,
    definition: u32,
    fields: &[(keld_semantics::FieldId, crate::Register)],
    span: Span,
) {
    write!(output, "r{destination} = {operation} d{definition}")
        .expect("writing to String cannot fail");
    for (field, register) in fields {
        write!(output, " f{}:r{}", field.0, register.0).expect("writing to String cannot fail");
    }
    output.push(' ');
    write_span(output, span);
}

fn write_span(output: &mut String, span: Span) {
    write!(
        output,
        "@s{}:{}..{}",
        span.source().0,
        span.start().0,
        span.end().0
    )
    .expect("writing to String cannot fail");
}

fn type_name(ty: &IrType) -> String {
    match ty {
        IrType::Unit => "Unit".to_owned(),
        IrType::Bool => "Bool".to_owned(),
        IrType::Int => "Int".to_owned(),
        IrType::Struct(definition) => format!("Struct[d{}]", definition.0),
        IrType::Entity(definition) => format!("Entity[d{}]", definition.0),
        IrType::Link { entity, optional } => {
            format!("Link[d{}{}]", entity.0, if *optional { "?" } else { "" })
        }
        IrType::Text => "Text".to_owned(),
        IrType::List(element) => format!("List[{}]", type_name(element)),
        IrType::Optional(element) => format!("Optional[{}]", type_name(element)),
        IrType::Lifecycle => "Lifecycle".to_owned(),
    }
}

const fn unary_name(operator: IntUnaryOp) -> &'static str {
    match operator {
        IntUnaryOp::Neg => "neg",
    }
}

const fn binary_name(operator: IntBinaryOp) -> &'static str {
    match operator {
        IntBinaryOp::Add => "add",
        IntBinaryOp::Sub => "sub",
        IntBinaryOp::Mul => "mul",
        IntBinaryOp::Div => "div",
        IntBinaryOp::Rem => "rem",
        IntBinaryOp::Shl => "shl",
        IntBinaryOp::Shr => "shr",
    }
}

const fn compare_name(operator: CompareOp) -> &'static str {
    match operator {
        CompareOp::Eq => "eq",
        CompareOp::NotEq => "ne",
        CompareOp::Less => "lt",
        CompareOp::LessEq => "le",
        CompareOp::Greater => "gt",
        CompareOp::GreaterEq => "ge",
    }
}

const fn fault_name(kind: FaultKind) -> &'static str {
    match kind {
        FaultKind::Arithmetic => "arithmetic",
        FaultKind::DivisionByZero => "division-by-zero",
        FaultKind::Shift => "shift",
        FaultKind::Allocation => "allocation",
        FaultKind::Capacity => "capacity",
    }
}
