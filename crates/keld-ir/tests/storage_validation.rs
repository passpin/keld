use keld_flow::StorageScopeId;
use keld_ir::{ArgumentProjection, Instruction, IrType, Module, Terminator, lower, validate};
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
