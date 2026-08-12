use crate::{ExitTarget, FlowModule, FlowOp, Terminator};
use keld_numeric::{IntBinaryOp, IntUnaryOp};
use keld_semantics::CompareOp;
use std::fmt::Write;

pub(crate) fn dump(module: &FlowModule) -> String {
    let mut output = String::new();
    for function in &module.functions {
        writeln!(
            output,
            "function f{} {} entry b{}",
            function.id.0, function.name, function.entry.0
        )
        .expect("writing to String cannot fail");
        for block in &function.blocks {
            writeln!(output, "  block b{}", block.id.0).expect("writing to String cannot fail");
            for operation in &block.operations {
                write!(output, "    ").expect("writing to String cannot fail");
                write_operation(&mut output, operation);
                output.push('\n');
            }
            write!(output, "    ").expect("writing to String cannot fail");
            write_terminator(&mut output, &block.terminator);
            output.push('\n');
        }
    }
    output
}

fn write_operation(output: &mut String, operation: &FlowOp) {
    match operation {
        FlowOp::ConstInt { dst, value, .. } => {
            write_line(output, format_args!("v{} = int {value}", dst.0));
        }
        FlowOp::ConstBool { dst, value, .. } => {
            write_line(output, format_args!("v{} = bool {value}", dst.0));
        }
        FlowOp::ConstNoneLink { dst, entity, .. } => {
            write_line(output, format_args!("v{} = none d{}", dst.0, entity.0));
        }
        FlowOp::BeginLifecycle {
            lifecycle, parent, ..
        } => write_line(
            output,
            format_args!("begin lifecycle l{} parent l{}", lifecycle.0, parent.0),
        ),
        FlowOp::CopyLocal { dst, local, .. } => {
            write_line(output, format_args!("v{} = local n{}", dst.0, local.0));
        }
        FlowOp::StoreLocal { local, value, .. } => {
            write_line(output, format_args!("local n{} = v{}", local.0, value.0));
        }
        FlowOp::UnaryInt { dst, op, value, .. } => write_line(
            output,
            format_args!("v{} = {} v{}", dst.0, unary_name(*op), value.0),
        ),
        FlowOp::BinaryInt {
            dst, op, lhs, rhs, ..
        } => write_line(
            output,
            format_args!("v{} = {} v{} v{}", dst.0, binary_name(*op), lhs.0, rhs.0),
        ),
        FlowOp::Not { dst, value, .. } => {
            write_line(output, format_args!("v{} = not v{}", dst.0, value.0));
        }
        FlowOp::Compare {
            dst, op, lhs, rhs, ..
        } => write_line(
            output,
            format_args!(
                "v{} = compare.{} v{} v{}",
                dst.0,
                compare_name(*op),
                lhs.0,
                rhs.0
            ),
        ),
        _ => write_effect_operation(output, operation),
    }
}

fn write_effect_operation(output: &mut String, operation: &FlowOp) {
    match operation {
        FlowOp::Phi { dst, inputs, .. } => {
            write!(output, "v{} = phi", dst.0).expect("writing to String cannot fail");
            for (block, value) in inputs {
                write!(output, " b{}:v{}", block.0, value.0)
                    .expect("writing to String cannot fail");
            }
        }
        FlowOp::ConstructStruct {
            dst,
            definition,
            fields,
            ..
        } => write_fields(output, "struct", dst.0, definition.0, fields),
        FlowOp::AllocateEntity {
            dst,
            definition,
            fields,
            lifecycle,
            site,
            ..
        } => {
            write_fields(output, "entity", dst.0, definition.0, fields);
            write!(output, " lifecycle l{} site a{}", lifecycle.0, site.0)
                .expect("writing to String cannot fail");
        }
        FlowOp::EntityToLink { dst, entity, .. } => {
            write_line(output, format_args!("v{} = link v{}", dst.0, entity.0));
        }
        FlowOp::ReadStructField {
            dst, base, field, ..
        } => write_line(
            output,
            format_args!("v{} = struct-field v{} f{}", dst.0, base.0, field.0),
        ),
        FlowOp::ReadEntityField {
            dst, entity, field, ..
        } => write_line(
            output,
            format_args!("v{} = entity-field v{} f{}", dst.0, entity.0, field.0),
        ),
        FlowOp::ReadUncheckedLinkField {
            dst, link, field, ..
        } => write_line(
            output,
            format_args!("v{} = unchecked-link-field v{} f{}", dst.0, link.0, field.0),
        ),
        FlowOp::WriteEntityField {
            entity,
            field,
            value,
            ..
        } => write_line(
            output,
            format_args!("entity-field v{} f{} = v{}", entity.0, field.0, value.0),
        ),
        FlowOp::Call {
            dst,
            function,
            arguments,
            current_lifecycle,
            ..
        } => {
            if let Some(dst) = dst {
                write!(output, "v{} = ", dst.0).expect("writing to String cannot fail");
            }
            write!(output, "call f{}", function.0).expect("writing to String cannot fail");
            for (parameter, value) in arguments {
                write!(output, " p{}:v{}", parameter.0, value.0)
                    .expect("writing to String cannot fail");
            }
            write!(output, " lifecycle l{}", current_lifecycle.0)
                .expect("writing to String cannot fail");
        }
        FlowOp::Keep { entity, target, .. } => {
            write_line(output, format_args!("keep v{} in l{}", entity.0, target.0));
        }
        FlowOp::Retire { entity, .. } => {
            write_line(output, format_args!("retire v{}", entity.0));
        }
        FlowOp::ConstInt { .. }
        | FlowOp::ConstBool { .. }
        | FlowOp::ConstNoneLink { .. }
        | FlowOp::BeginLifecycle { .. }
        | FlowOp::CopyLocal { .. }
        | FlowOp::StoreLocal { .. }
        | FlowOp::UnaryInt { .. }
        | FlowOp::BinaryInt { .. }
        | FlowOp::Not { .. }
        | FlowOp::Compare { .. } => {
            unreachable!("primitive operations are formatted by write_operation")
        }
    }
}

fn write_terminator(output: &mut String, terminator: &Terminator) {
    match terminator {
        Terminator::Goto(block) => write_line(output, format_args!("goto b{}", block.0)),
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => write_line(
            output,
            format_args!(
                "branch v{} then b{} else b{}",
                condition.0, then_block.0, else_block.0
            ),
        ),
        Terminator::BranchIdentity {
            lhs,
            rhs,
            equal,
            not_equal,
        } => write_line(
            output,
            format_args!(
                "branch-identity v{} v{} equal b{} not-equal b{}",
                lhs.0, rhs.0, equal.0, not_equal.0
            ),
        ),
        Terminator::ResolveLink {
            link,
            bind_local,
            live,
            absent,
            ..
        } => write_line(
            output,
            format_args!(
                "resolve v{} bind n{} live b{} absent b{}",
                link.0, bind_local.0, live.0, absent.0
            ),
        ),
        Terminator::ExitScopes { lifecycles, next } => {
            output.push_str("exit");
            for lifecycle in lifecycles {
                write!(output, " l{}", lifecycle.0).expect("writing to String cannot fail");
            }
            match next {
                ExitTarget::Goto(block) => {
                    write!(output, " goto b{}", block.0).expect("writing to String cannot fail");
                }
                ExitTarget::Return(value) => {
                    output.push_str(" return");
                    if let Some(value) = value {
                        write!(output, " v{}", value.0).expect("writing to String cannot fail");
                    }
                }
            }
        }
        Terminator::Return(value) => {
            output.push_str("return");
            if let Some(value) = value {
                write!(output, " v{}", value.0).expect("writing to String cannot fail");
            }
        }
        Terminator::Unreachable => output.push_str("unreachable"),
    }
}

fn write_fields(
    output: &mut String,
    operation: &str,
    destination: u32,
    definition: u32,
    fields: &[(keld_semantics::FieldId, crate::ValueId)],
) {
    write!(output, "v{destination} = {operation} d{definition}")
        .expect("writing to String cannot fail");
    for (field, value) in fields {
        write!(output, " f{}:v{}", field.0, value.0).expect("writing to String cannot fail");
    }
}

fn write_line(output: &mut String, arguments: std::fmt::Arguments<'_>) {
    output
        .write_fmt(arguments)
        .expect("writing to String cannot fail");
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
