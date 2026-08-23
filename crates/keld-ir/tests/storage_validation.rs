use keld_flow::StorageScopeId;
use keld_ir::{
    ArgumentProjection, Instruction, IrType, Module, RegisterStorage, Terminator, lower, validate,
};
use keld_source::Span;
use keld_storage::verify_text_for_test;

fn lower_ok(source: &str) -> Module {
    let verification = verify_text_for_test(source);
    let verified = verification
        .module
        .unwrap_or_else(|| panic!("source must verify: {:#?}", verification.diagnostics));
    lower(&verified)
}

fn assert_ir_error(module: &Module, message: &str) {
    assert_ir_diagnostic(module, "KLD9006", message);
}

fn assert_ir_diagnostic(module: &Module, code: &str, message: &str) {
    let diagnostics = validate(module);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == code
                && diagnostic.primary.message.contains(message)),
        "{diagnostics:#?}"
    );
}

fn remove_last_cleanup_of_main(module: &mut Module) {
    let main = module.main.0 as usize;
    let block = module.functions[main]
        .blocks
        .iter_mut()
        .find(|block| {
            block
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DropHome { .. }))
        })
        .expect("compiler-produced cleanup block exists");
    let index = block
        .instructions
        .iter()
        .rposition(|instruction| matches!(instruction, Instruction::DropHome { .. }))
        .expect("compiler-produced cleanup instruction exists");
    block.instructions.remove(index);
}

fn insert_drop_after_move(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(index) = block
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, Instruction::MoveHome { .. }))
            {
                let Instruction::MoveHome {
                    source: home, span, ..
                } = block.instructions[index]
                else {
                    unreachable!()
                };
                block
                    .instructions
                    .insert(index + 1, Instruction::DropHome { home, span });
                return;
            }
        }
    }
    panic!("compiler-produced move instruction exists");
}

fn replace_first_loan_read_with_move(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(index) = block
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, Instruction::Copy { .. }))
            {
                let Instruction::Copy { dst, src, span } = block.instructions[index] else {
                    unreachable!()
                };
                block.instructions[index] = Instruction::MoveHome {
                    destination: dst,
                    source: src,
                    span,
                };
                return;
            }
        }
    }
    panic!("compiler-produced loan read exists");
}

fn replace_first_loan_read_with_take(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(index) = block
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, Instruction::Copy { .. }))
            {
                let Instruction::Copy { dst, src, span } = block.instructions[index] else {
                    unreachable!()
                };
                block.instructions[index] = Instruction::Take { dst, src, span };
                return;
            }
        }
    }
    panic!("compiler-produced loan read exists");
}

fn replace_first_take_destination_with_trivial(module: &mut Module) {
    let function = &mut module.functions[module.main.0 as usize];
    let source = keld_ir::Register(
        u32::try_from(function.register_types.len()).expect("register count fits"),
    );
    let destination = keld_ir::Register(
        u32::try_from(function.register_types.len() + 1).expect("register count fits"),
    );
    let home = function
        .register_storage
        .iter()
        .find(|storage| matches!(storage, RegisterStorage::Home { .. }))
        .cloned()
        .expect("main has a managed home");
    function.register_types.extend([IrType::Text, IrType::Text]);
    function
        .register_storage
        .extend([home, RegisterStorage::Trivial]);
    let span = function.span;
    let entry = function.entry;
    let block = function
        .blocks
        .iter_mut()
        .find(|block| block.id == entry)
        .expect("main entry block exists");
    block.instructions.splice(
        0..0,
        [
            Instruction::ConstText {
                dst: source,
                value: "owned".to_owned(),
                span,
            },
            Instruction::Take {
                dst: destination,
                src: source,
                span,
            },
        ],
    );
}

fn replace_first_projected_call_index_with_lifecycle(module: &mut Module) {
    for function in &mut module.functions {
        let lifecycle = function.current_lifecycle;
        for block in &mut function.blocks {
            if let Some(Instruction::Call {
                argument_sources, ..
            }) = block
                .instructions
                .iter_mut()
                .find(|instruction| matches!(instruction, Instruction::Call { .. }))
            {
                let source = argument_sources
                    .iter_mut()
                    .find_map(|(_, source)| source.as_mut())
                    .expect("projected call source exists");
                source.projections[0] = ArgumentProjection::Index(lifecycle);
                return;
            }
        }
    }
    panic!("compiler-produced projected call exists");
}

fn replace_first_projected_call_base_with_lifecycle(module: &mut Module) {
    for function in &mut module.functions {
        let lifecycle = function.current_lifecycle;
        for block in &mut function.blocks {
            if let Some(Instruction::Call {
                argument_sources, ..
            }) = block
                .instructions
                .iter_mut()
                .find(|instruction| matches!(instruction, Instruction::Call { .. }))
            {
                let source = argument_sources
                    .iter_mut()
                    .find_map(|(_, source)| source.as_mut())
                    .expect("projected call source exists");
                source.base = lifecycle;
                source.projections.clear();
                return;
            }
        }
    }
    panic!("compiler-produced projected call exists");
}

fn replace_first_struct_read_destination_with_trivial(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::ReadStructField { dst, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::ReadStructField { .. }))
            {
                function.register_storage[dst.0 as usize] = RegisterStorage::Trivial;
                return;
            }
        }
    }
    panic!("compiler-produced struct field read exists");
}

fn replace_first_entity_read_destination_with_trivial(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::ReadField { dst, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::ReadField { .. }))
            {
                function.register_storage[dst.0 as usize] = RegisterStorage::Trivial;
                return;
            }
        }
    }
    panic!("compiler-produced entity field read exists");
}

fn replace_first_list_push_value_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(value) = block
                .instructions
                .iter()
                .find_map(|instruction| match instruction {
                    Instruction::ListPush { value, .. }
                    | Instruction::ListPushPlace { value, .. } => Some(*value),
                    _ => None,
                })
                && matches!(
                    function.register_storage.get(value.0 as usize),
                    Some(RegisterStorage::Home { .. })
                )
            {
                function.register_storage[value.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced List push with an owned value exists");
}

fn replace_first_list_replace_value_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::ListReplace { value, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::ListReplace { .. }))
                && matches!(
                    function.register_storage.get(value.0 as usize),
                    Some(RegisterStorage::Home { .. })
                )
            {
                function.register_storage[value.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced List replacement with an owned value exists");
}

fn replace_first_constructed_field_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::ConstructStruct { fields, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::ConstructStruct { .. }))
                && let Some((_, value)) = fields.iter().find(|(_, value)| {
                    matches!(
                        function.register_storage.get(value.0 as usize),
                        Some(RegisterStorage::Home { .. })
                    )
                })
            {
                function.register_storage[value.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced aggregate with an owned field exists");
}

fn replace_first_allocated_field_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::AllocateEntity { fields, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::AllocateEntity { .. }))
                && let Some((_, value)) = fields.iter().find(|(_, value)| {
                    matches!(
                        function.register_storage.get(value.0 as usize),
                        Some(RegisterStorage::Home { .. })
                    )
                })
            {
                function.register_storage[value.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced entity aggregate with an owned field exists");
}

fn replace_first_field_write_value_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(value) = block
                .instructions
                .iter()
                .find_map(|instruction| match instruction {
                    Instruction::WriteField { value, .. } => Some(*value),
                    Instruction::ReplaceField { source, .. } => Some(*source),
                    _ => None,
                })
                && matches!(
                    function.register_storage.get(value.0 as usize),
                    Some(RegisterStorage::Home { .. })
                )
            {
                function.register_storage[value.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced field write with an owned value exists");
}

fn force_managed_entity_write_instruction(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            for instruction in &mut block.instructions {
                match instruction.clone() {
                    Instruction::WriteField { .. } => return,
                    Instruction::ReplaceField {
                        view,
                        field,
                        source,
                        span,
                        ..
                    } => {
                        *instruction = Instruction::WriteField {
                            view,
                            field,
                            value: source,
                            span,
                        };
                        return;
                    }
                    _ => {}
                }
            }
        }
    }
    panic!("compiler-produced managed entity field update exists");
}

fn mismatch_entity_field_replacement_type(module: &mut Module, source_mismatch: bool) {
    for function in &mut module.functions {
        for block_index in 0..function.blocks.len() {
            for instruction_index in 0..function.blocks[block_index].instructions.len() {
                let instruction =
                    function.blocks[block_index].instructions[instruction_index].clone();
                let (view, field, source, existing_displaced, span) = match instruction {
                    Instruction::WriteField {
                        view,
                        field,
                        value,
                        span,
                    } => (view, field, value, None, span),
                    Instruction::ReplaceField {
                        view,
                        field,
                        source,
                        displaced,
                        span,
                    } => (view, field, source, Some(displaced), span),
                    _ => continue,
                };
                let displaced = existing_displaced.unwrap_or_else(|| {
                    let register = keld_ir::Register(
                        u32::try_from(function.register_types.len())
                            .expect("register count fits in u32"),
                    );
                    function.register_types.push(IrType::Text);
                    function.register_storage.push(RegisterStorage::DropSlot);
                    register
                });
                if source_mismatch {
                    function.register_types[source.0 as usize] = IrType::Int;
                } else {
                    function.register_types[displaced.0 as usize] = IrType::Int;
                }
                function.blocks[block_index].instructions[instruction_index] =
                    Instruction::ReplaceField {
                        view,
                        field,
                        source,
                        displaced,
                        span,
                    };
                return;
            }
        }
    }
    panic!("compiler-produced managed entity field update exists");
}

fn replace_first_place_replacement_source_with_loan(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(Instruction::ReplacePlace { source, .. }) = block
                .instructions
                .iter()
                .find(|instruction| matches!(instruction, Instruction::ReplacePlace { .. }))
                && matches!(
                    function.register_storage.get(source.0 as usize),
                    Some(RegisterStorage::Home { .. })
                )
            {
                function.register_storage[source.0 as usize] = RegisterStorage::Loan;
                return;
            }
        }
    }
    panic!("compiler-produced place replacement with an owned source exists");
}

fn replace_first_consuming_call_argument_with_loan(module: &mut Module) {
    for function_index in 0..module.functions.len() {
        let argument = module.functions[function_index]
            .blocks
            .iter()
            .flat_map(|block| block.instructions.iter())
            .find_map(|instruction| {
                let Instruction::Call {
                    function: callee,
                    arguments,
                    ..
                } = instruction
                else {
                    return None;
                };
                arguments.iter().find_map(|(parameter, argument)| {
                    (module.functions[callee.0 as usize]
                        .parameter_modes
                        .get(parameter.0 as usize)
                        == Some(&keld_semantics::ParameterMode::Take)
                        && matches!(
                            module.functions[function_index]
                                .register_storage
                                .get(argument.0 as usize),
                            Some(RegisterStorage::Home { .. })
                        ))
                    .then_some(*argument)
                })
            });
        if let Some(argument) = argument {
            module.functions[function_index].register_storage[argument.0 as usize] =
                RegisterStorage::Loan;
            return;
        }
    }
    panic!("compiler-produced consuming call with an owned argument exists");
}

fn empty_first_managed_return(module: &mut Module) {
    for function in &mut module.functions {
        if function.return_type != IrType::Text {
            continue;
        }
        for block in &mut function.blocks {
            if let Terminator::Return(Some(value)) = block.terminator {
                block.instructions.push(Instruction::DropHome {
                    home: value,
                    span: function.span,
                });
                return;
            }
        }
    }
    panic!("compiler-produced managed return exists");
}

fn make_maybe_live_managed_return(module: &mut Module) {
    for function in &mut module.functions {
        if function.id == module.main {
            continue;
        }
        let Some((candidate, scope)) =
            function
                .register_types
                .iter()
                .enumerate()
                .find_map(|(index, ty)| {
                    if *ty != IrType::Text {
                        return None;
                    }
                    match function.register_storage.get(index) {
                        Some(RegisterStorage::Home {
                            scope,
                            conditional: true,
                        }) => Some((
                            keld_ir::Register(u32::try_from(index).expect("register index fits")),
                            *scope,
                        )),
                        _ => None,
                    }
                })
        else {
            continue;
        };
        function.return_type = IrType::Text;
        for block in &mut function.blocks {
            block.instructions.retain(|instruction| {
                !matches!(
                    instruction,
                    Instruction::DropIfLive { home, .. } if *home == candidate
                ) && !matches!(
                    instruction,
                    Instruction::CleanupTrackedScope { scope: current, .. } if *current == scope
                )
            });
            if matches!(block.terminator, Terminator::Return(Some(_))) {
                block.terminator = Terminator::Return(Some(candidate));
                return;
            }
        }
    }
    panic!("compiler-produced MaybeLive managed home exists");
}

fn rewrite_cleanup_scope(module: &mut Module, scope: StorageScopeId) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            for instruction in &mut block.instructions {
                if let Instruction::CleanupTrackedScope { scope: current, .. } = instruction {
                    *current = scope;
                    return;
                }
            }
        }
    }
    panic!("compiler-produced tracked cleanup exists");
}

fn remove_first_drop_slot(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(index) = block
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, Instruction::DropSlot { .. }))
            {
                block.instructions.remove(index);
                return;
            }
        }
    }
    panic!("compiler-produced drop slot exists");
}

fn remove_drop_after_list_replace(module: &mut Module) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            if let Some(index) = block
                .instructions
                .iter()
                .position(|instruction| matches!(instruction, Instruction::ListReplace { .. }))
            {
                assert!(matches!(
                    block.instructions.get(index + 1),
                    Some(Instruction::DropSlot { .. })
                ));
                block.instructions.remove(index + 1);
                return;
            }
        }
    }
    panic!("compiler-produced list replacement exists");
}

fn drop_home_before(
    module: &mut Module,
    select: impl Fn(&Instruction) -> Option<(keld_ir::Register, Span)>,
) {
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let Some((index, home, span)) =
                block
                    .instructions
                    .iter()
                    .enumerate()
                    .find_map(|(index, instruction)| {
                        let (home, span) = select(instruction)?;
                        matches!(
                            function.register_storage.get(home.0 as usize),
                            Some(RegisterStorage::Home { .. })
                        )
                        .then_some((index, home, span))
                    })
            else {
                continue;
            };
            block
                .instructions
                .insert(index, Instruction::DropHome { home, span });
            return;
        }
    }
    panic!("compiler-produced managed operand exists");
}

fn drop_one_managed_phi_input(module: &mut Module) {
    for function in &mut module.functions {
        let Some((destination, inputs, span)) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(|instruction| match instruction {
                Instruction::Phi { dst, inputs, span } => Some((*dst, inputs.clone(), *span)),
                _ => None,
            })
        else {
            continue;
        };
        let storage = RegisterStorage::Home {
            scope: StorageScopeId(0),
            conditional: false,
        };
        function.register_types[destination.0 as usize] = IrType::Text;
        function.register_storage[destination.0 as usize] = storage.clone();
        for (_, input) in &inputs {
            function.register_types[input.0 as usize] = IrType::Text;
            function.register_storage[input.0 as usize] = storage.clone();
        }
        let (predecessor, input) = inputs[0];
        let block = &mut function.blocks[predecessor.0 as usize];
        block
            .instructions
            .push(Instruction::DropHome { home: input, span });
        return;
    }
    panic!("compiler-produced Phi exists");
}

#[test]
fn managed_home_cannot_return_live_without_cleanup() {
    let mut module = lower_ok("fn main() -> Int { let value: Text = \"Keld\"; return 0; }\n");
    remove_last_cleanup_of_main(&mut module);
    assert_ir_error(&module, "live managed home at return");
}

#[test]
fn moved_home_cannot_be_dropped_twice() {
    let mut module = lower_ok(
        "fn consume(take value: Text) { return }\nfn main() -> Int { let value: Text = \"Keld\"; consume(take value); return 0; }\n",
    );
    insert_drop_after_move(&mut module);
    assert_ir_error(&module, "drop of empty home");
}

#[test]
fn a_loan_register_cannot_be_moved_or_dropped() {
    let mut module = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"Keld\") }\n",
    );
    replace_first_loan_read_with_move(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn a_loan_register_cannot_be_taken() {
    let mut module = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"Keld\") }\n",
    );
    replace_first_loan_read_with_take(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn managed_take_destination_must_be_a_home() {
    let mut module = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length }\nfn main() -> Int { return inspect(\"Keld\") }\n",
    );
    replace_first_take_destination_with_trivial(&mut module);
    assert_ir_error(&module, "managed take destination must be a Home");
}

#[test]
fn managed_struct_field_read_destination_must_be_a_loan() {
    let mut module = lower_ok(
        "struct Holder { value: Text }
fn main() -> Int { let holder = Holder(value: \"Keld\"); return holder.value.byte_length }
",
    );
    replace_first_struct_read_destination_with_trivial(&mut module);
    assert_ir_error(&module, "managed field read destination must be a Loan");
}

#[test]
fn managed_entity_field_read_destination_must_be_a_loan() {
    let mut module = lower_ok(
        "entity Holder { value: Text }
fn main() -> Int { lifecycle level { let holder = Holder(value: \"Keld\"); return holder.value.byte_length } }
",
    );
    replace_first_entity_read_destination_with_trivial(&mut module);
    assert_ir_error(&module, "managed field read destination must be a Loan");
}

#[test]
fn list_push_must_not_consume_a_loan_register() {
    let mut module = lower_ok(
        "fn main() -> Int { let value: Text = \"Keld\"; let items: List[Text] = List(); items.push(take value); return 0 }
",
    );
    replace_first_list_push_value_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn list_replacement_must_not_consume_a_loan_register() {
    let mut module = lower_ok(
        "fn main() -> Int { let old: Text = \"old\"; let items: List[Text] = List(); items.push(take old); let new: Text = \"new\"; items[0] = take new; return 0 }
",
    );
    replace_first_list_replace_value_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn aggregate_construction_must_not_consume_a_loan_field() {
    let mut module = lower_ok(
        "struct Holder { value: Text }
fn main() -> Int { let holder = Holder(value: \"Keld\"); return holder.value.byte_length }
",
    );
    replace_first_constructed_field_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn entity_construction_must_not_consume_a_loan_field() {
    let mut module = lower_ok(
        "entity Holder { value: Text }
fn main() -> Int { lifecycle level { let value: Text = \"Keld\"; let holder = Holder(value: take value); return holder.value.byte_length } }
",
    );
    replace_first_allocated_field_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn entity_field_write_must_not_consume_a_loan_register() {
    let mut module = lower_ok(
        "entity Holder { value: Text }
fn main() -> Int { lifecycle level { let holder = Holder(value: \"old\"); holder.value = \"new\"; return 0 } }
",
    );
    replace_first_field_write_value_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn managed_entity_fields_must_not_use_write_field_ir() {
    let mut module = lower_ok(
        "entity Holder { value: Text }\nfn main() -> Int { lifecycle level { let holder = Holder(value: \"old\"); holder.value = \"new\"; return 0; } }\n",
    );
    force_managed_entity_write_instruction(&mut module);

    assert_ir_error(&module, "managed entity field requires ReplaceField");
}

#[test]
fn entity_field_replacement_registers_must_match_the_selected_field() {
    let source = "entity Holder { value: Text }\nfn main() -> Int { lifecycle level { let holder = Holder(value: \"old\"); holder.value = \"new\"; return 0; } }\n";
    let mut wrong_source = lower_ok(source);
    mismatch_entity_field_replacement_type(&mut wrong_source, true);
    assert_ir_error(
        &wrong_source,
        "field replacement source type does not match the selected field",
    );

    let mut wrong_displaced = lower_ok(source);
    mismatch_entity_field_replacement_type(&mut wrong_displaced, false);
    assert_ir_error(
        &wrong_displaced,
        "field replacement displaced type does not match the selected field",
    );
}

#[test]
fn struct_field_replacement_must_not_consume_a_loan_register() {
    let mut module = lower_ok(
        "struct Holder { value: Text }
fn main() -> Int { var holder = Holder(value: \"old\"); let new: Text = \"new\"; holder.value = take new; return holder.value.byte_length }
",
    );
    replace_first_place_replacement_source_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn consuming_call_must_not_consume_a_loan_register() {
    let mut module = lower_ok(
        "fn consume(take value: Text) { return }
fn main() -> Int { let value: Text = \"Keld\"; consume(take value); return 0 }
",
    );
    replace_first_consuming_call_argument_with_loan(&mut module);
    assert_ir_error(&module, "loan register used as an owned source");
}

#[test]
fn projected_call_source_index_must_be_an_int() {
    let mut module = lower_ok(
        "fn inspect(items: List[Int]) -> Int { return items.length }\nfn main() -> Int { let matrix: List[List[Int]] = List(); return inspect(matrix[0]) }\n",
    );
    replace_first_projected_call_index_with_lifecycle(&mut module);
    assert_ir_diagnostic(
        &module,
        "KLD9002",
        "register type does not match the operation",
    );
}

#[test]
fn projected_call_source_must_resolve_to_the_argument_type() {
    let mut module = lower_ok(
        "fn inspect(items: List[Int]) -> Int { return items.length }\nfn main() -> Int { let matrix: List[List[Int]] = List(); return inspect(matrix[0]) }\n",
    );
    replace_first_projected_call_base_with_lifecycle(&mut module);
    assert_ir_diagnostic(
        &module,
        "KLD9006",
        "call argument source does not resolve to the parameter type",
    );
}

#[test]
fn managed_return_register_must_be_live() {
    let mut module =
        lower_ok("fn make() -> Text { return \"ok\" }\nfn main() -> Int { return 0 }\n");
    empty_first_managed_return(&mut module);
    assert_ir_error(&module, "returned managed home must be live");
}

#[test]
fn maybe_live_managed_return_register_must_be_live() {
    let mut module = lower_ok(
        "fn make(flag: Bool) -> Int { var value: Text; if flag { value = \"yes\"; }; return 0; }
fn main() -> Int { return 0; }
",
    );
    make_maybe_live_managed_return(&mut module);
    assert_ir_error(&module, "returned managed home must be live");
}

#[test]
fn tracked_scope_cleanup_must_name_the_register_scope() {
    let mut module = lower_ok(
        "fn main() -> Int { var a: Text; var b: Text; if true { a = \"a\"; b = \"b\"; } else { b = \"b\"; a = \"a\"; }; return 0; }\n",
    );
    rewrite_cleanup_scope(&mut module, StorageScopeId(u32::MAX));
    assert_ir_error(&module, "unknown cleanup scope");
}

#[test]
fn displaced_values_must_be_dropped_after_replacement() {
    let mut module =
        lower_ok("fn main() -> Int { var value: Text = \"old\"; value = \"new\"; return 0; }\n");
    remove_first_drop_slot(&mut module);
    assert_ir_error(&module, "live displaced value at return");
}

#[test]
fn list_get_lowers_to_a_valid_optional_result() {
    let module = lower_ok(
        "fn main() -> Int { let items: List[Int] = List(); let maybe = items.get(0); return 0; }\n",
    );
    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
}

#[test]
fn indexed_replacement_requires_displaced_cleanup() {
    let mut module = lower_ok(
        "fn main() -> Int { let items: List[Text] = List(); items.push(\"old\"); items[0] = \"new\"; return 0; }\n",
    );
    remove_drop_after_list_replace(&mut module);
    assert_ir_error(&module, "live displaced value at return");
}

#[test]
fn text_read_requires_a_live_managed_home() {
    let mut module = lower_ok("fn main() -> Int { return \"Keld\".byte_length; }\n");
    drop_home_before(&mut module, |instruction| match instruction {
        Instruction::TextByteLength { text, span, .. } => Some((*text, *span)),
        _ => None,
    });

    assert_ir_error(&module, "managed operand must be a live Home");
}

#[test]
fn text_compare_and_concat_require_live_managed_homes() {
    let mut compare = lower_ok("fn main() -> Int { if \"a\" == \"b\" { return 1; }; return 0; }\n");
    drop_home_before(&mut compare, |instruction| match instruction {
        Instruction::Compare { lhs, span, .. } => Some((*lhs, *span)),
        _ => None,
    });
    assert_ir_error(&compare, "managed operand must be a live Home");

    let mut concat = lower_ok("fn main() -> Int { let value = \"a\" + \"b\"; return 0; }\n");
    drop_home_before(&mut concat, |instruction| match instruction {
        Instruction::TextConcat { lhs, span, .. } => Some((*lhs, *span)),
        _ => None,
    });
    assert_ir_error(&concat, "managed operand must be a live Home");
}

#[test]
fn list_read_and_structural_operations_require_a_live_home() {
    let mut read = lower_ok(
        "fn make() -> List[Int] { return List(); }\nfn main() -> Int { return make().length; }\n",
    );
    drop_home_before(&mut read, |instruction| match instruction {
        Instruction::ListLength { list, span, .. } => Some((*list, *span)),
        _ => None,
    });
    assert_ir_error(&read, "managed operand must be a live Home");

    let mut structural = lower_ok(
        "fn make() -> List[Int] { return List(); }\nfn main() -> Int { make().reserve(1); return 0; }\n",
    );
    drop_home_before(&mut structural, |instruction| match instruction {
        Instruction::ListReserve { receiver, span, .. } => Some((receiver.list, *span)),
        _ => None,
    });
    assert_ir_error(&structural, "managed operand must be a live Home");
}

#[test]
fn calls_require_live_managed_arguments_and_projected_bases() {
    let mut argument = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length; }\nfn main() -> Int { return inspect(\"Keld\"); }\n",
    );
    drop_home_before(&mut argument, |instruction| match instruction {
        Instruction::Call {
            arguments, span, ..
        } => arguments.first().map(|(_, argument)| (*argument, *span)),
        _ => None,
    });
    assert_ir_error(&argument, "managed operand must be a live Home");

    let mut projected = lower_ok(
        "fn inspect(items: List[Int]) -> Int { return items.length; }\nfn main() -> Int { let matrix: List[List[Int]] = List(); return inspect(matrix[0]); }\n",
    );
    drop_home_before(&mut projected, |instruction| match instruction {
        Instruction::Call {
            argument_sources,
            span,
            ..
        } => argument_sources
            .iter()
            .find_map(|(_, source)| source.as_ref().map(|source| (source.base, *span))),
        _ => None,
    });
    assert_ir_error(&projected, "managed operand must be a live Home");
}

#[test]
fn managed_phi_inputs_must_be_live_on_each_predecessor() {
    let mut module = lower_ok("fn main() -> Int { let value = false && true; return 0; }\n");
    drop_one_managed_phi_input(&mut module);

    assert_ir_error(&module, "managed Phi input must be a live Home");
}

#[test]
fn managed_return_is_also_an_exhaustive_live_use() {
    let mut module =
        lower_ok("fn make() -> Text { return \"ok\"; }\nfn main() -> Int { return 0; }\n");
    empty_first_managed_return(&mut module);

    assert_ir_error(&module, "managed operand must be a live Home");
}

#[test]
fn conditional_drop_accepts_a_maybe_live_home() {
    let module = lower_ok(
        "fn maybe(flag: Bool) -> Int { var value: Text; if flag { value = \"yes\"; }; return 0; }\nfn main() -> Int { return maybe(true); }\n",
    );
    assert!(module.functions.iter().any(|function| {
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction, Instruction::DropIfLive { .. }))
    }));
    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
}

fn rewrite_first_output_role(
    module: &mut Module,
    select: impl Fn(&Instruction) -> Option<keld_ir::Register>,
    role: RegisterStorage,
) {
    for function in &mut module.functions {
        if let Some(register) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(&select)
        {
            function.register_storage[register.0 as usize] = role;
            return;
        }
    }
    panic!("compiler-produced instruction output exists");
}

#[test]
fn const_text_destination_must_be_an_owned_home() {
    let mut module = lower_ok("fn main() -> Int { let value: Text = \"Keld\"; return 0; }\n");
    rewrite_first_output_role(
        &mut module,
        |instruction| match instruction {
            Instruction::ConstText { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Trivial,
    );

    assert_ir_error(&module, "owned instruction result must be a Home");
}

#[test]
fn managed_construction_results_must_not_be_loans() {
    let mut list = lower_ok("fn main() -> Int { let items: List[Int] = List(); return 0; }\n");
    rewrite_first_output_role(
        &mut list,
        |instruction| match instruction {
            Instruction::ListNew { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Loan,
    );
    assert_ir_error(&list, "owned instruction result must be a Home");

    let mut text = lower_ok("fn main() -> Int { let value = \"a\" + \"b\"; return 0; }\n");
    rewrite_first_output_role(
        &mut text,
        |instruction| match instruction {
            Instruction::TextConcat { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Loan,
    );
    assert_ir_error(&text, "owned instruction result must be a Home");
}

#[test]
fn managed_field_and_list_reads_must_produce_loans() {
    let mut field = lower_ok(
        "struct Holder { value: Text }\nfn main() -> Int { let holder = Holder(value: \"Keld\"); return holder.value.byte_length; }\n",
    );
    rewrite_first_output_role(
        &mut field,
        |instruction| match instruction {
            Instruction::ReadStructField { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Home {
            scope: StorageScopeId(0),
            conditional: false,
        },
    );
    assert_ir_error(&field, "loan instruction result must be a Loan");

    let mut list = lower_ok(
        "fn size(items: List[Text]) -> Int { return items[0].byte_length; }\nfn main() -> Int { return 0; }\n",
    );
    rewrite_first_output_role(
        &mut list,
        |instruction| match instruction {
            Instruction::ListIndex { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Home {
            scope: StorageScopeId(0),
            conditional: false,
        },
    );
    assert_ir_error(&list, "loan instruction result must be a Loan");
}

#[test]
fn entity_results_must_use_entity_flow_registers() {
    let mut module = lower_ok(
        "entity Holder { value: Int }\nfn main() -> Int { lifecycle level { let holder = Holder(value: 1); return holder.value; }; }\n",
    );
    rewrite_first_output_role(
        &mut module,
        |instruction| match instruction {
            Instruction::AllocateEntity { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Trivial,
    );

    assert_ir_error(&module, "entity-flow register must use EntityFlow storage");
}

#[test]
fn plain_results_must_not_be_managed_homes() {
    let mut module = lower_ok("fn main() -> Int { let value = 1; return value; }\n");
    rewrite_first_output_role(
        &mut module,
        |instruction| match instruction {
            Instruction::ConstInt { dst, .. } => Some(*dst),
            _ => None,
        },
        RegisterStorage::Home {
            scope: StorageScopeId(0),
            conditional: false,
        },
    );

    assert_ir_error(&module, "plain register must use Trivial storage");
}

#[test]
fn managed_parameter_roles_follow_their_parameter_modes() {
    let mut loan = lower_ok(
        "fn inspect(value: Text) -> Int { return value.byte_length; }\nfn main() -> Int { return 0; }\n",
    );
    let parameter = loan.functions[0].parameters[0];
    loan.functions[0].register_storage[parameter.0 as usize] = RegisterStorage::Trivial;
    assert_ir_error(&loan, "managed loan parameter must be a Loan");

    let mut take =
        lower_ok("fn consume(take value: Text) { return; }\nfn main() -> Int { return 0; }\n");
    let parameter = take.functions[0].parameters[0];
    take.functions[0].register_storage[parameter.0 as usize] = RegisterStorage::Loan;
    assert_ir_error(&take, "managed consuming parameter must be a Home");
}

#[test]
fn managed_phi_inputs_must_match_the_destination_role() {
    let mut module = lower_ok(
        "fn choose(left: Bool, right: Bool) -> Bool { return left && right; }\nfn main() -> Int { return 0; }\n",
    );
    let (function_index, destination, inputs) = module
        .functions
        .iter()
        .enumerate()
        .find_map(|(function_index, function)| {
            function.blocks.iter().find_map(|block| {
                block
                    .instructions
                    .iter()
                    .find_map(|instruction| match instruction {
                        Instruction::Phi { dst, inputs, .. } => {
                            Some((function_index, *dst, inputs.clone()))
                        }
                        _ => None,
                    })
            })
        })
        .expect("compiler-produced Phi exists");
    let function = &mut module.functions[function_index];
    function.register_types[destination.0 as usize] = IrType::Text;
    function.register_storage[destination.0 as usize] = RegisterStorage::Loan;
    for (_, input) in &inputs {
        function.register_types[input.0 as usize] = IrType::Text;
        function.register_storage[input.0 as usize] = RegisterStorage::Loan;
    }
    let owned_input = inputs
        .iter()
        .map(|(_, input)| *input)
        .find(|input| !function.parameters.contains(input))
        .expect("short-circuit Phi has a non-parameter input");
    function.register_storage[owned_input.0 as usize] = RegisterStorage::Home {
        scope: StorageScopeId(0),
        conditional: false,
    };

    assert_ir_error(
        &module,
        "managed Phi input storage role must match its destination",
    );
}

#[test]
fn compiler_produced_register_categories_validate() {
    for source in [
        "fn main() -> Int { return 1; }\n",
        "fn main() -> Int { let value: Text = \"Keld\"; return 0; }\n",
        "struct Holder { value: Text }\nfn main() -> Int { let holder = Holder(value: \"Keld\"); return holder.value.byte_length; }\n",
        "entity Holder { value: Int }\nfn main() -> Int { lifecycle level { let holder = Holder(value: 1); return holder.value; }; }\n",
        "entity Holder { value: Int }\nfn same(left: Holder, right: Holder) -> Bool { return left == right; }\nfn main() -> Int { return 0; }\n",
        "fn main() -> Int { var value: Text = \"old\"; value = \"new\"; return 0; }\n",
    ] {
        let module = lower_ok(source);
        assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
    }
}
