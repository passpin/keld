use keld_flow::StorageScopeId;
use keld_ir::{Instruction, Module, lower, validate};
use keld_storage::verify_text_for_test;

fn lower_ok(source: &str) -> Module {
    let verification = verify_text_for_test(source);
    let verified = verification
        .module
        .unwrap_or_else(|| panic!("source must verify: {:#?}", verification.diagnostics));
    lower(&verified)
}

fn assert_ir_error(module: &Module, message: &str) {
    let diagnostics = validate(module);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD9006"
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
