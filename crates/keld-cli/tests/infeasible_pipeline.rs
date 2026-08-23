use keld_cli::compile_source;
use keld_ir::{Instruction, Terminator, validate};
use keld_storage::ValueStorage;
use std::path::Path;

#[test]
fn lifecycle_infeasible_managed_block_does_not_reach_ir_as_executable() {
    let source = "entity Enemy {\nhealth: Int\n}\nfn inspect(enemy: Enemy) -> Int { let alias = enemy; if enemy == alias { return enemy.health } else { let values: List[Int] = List(); values.push(1); return values.length } }\nfn main() -> Int { return 0 }\n";
    let compilation = compile_source(Path::new("infeasible-cfg.keld"), source.as_bytes().to_vec());

    assert!(
        compilation.diagnostics.is_empty(),
        "{:#?}",
        compilation.diagnostics
    );
    let module = compilation.ir.as_ref().expect("accepted source has IR");
    assert!(validate(module).is_empty(), "{:#?}", validate(module));
    let storage = compilation
        .storage
        .as_ref()
        .expect("accepted source has storage annotations");
    assert!(
        storage.annotations.functions[0]
            .values
            .iter()
            .any(|value| matches!(value, ValueStorage::Unreachable))
    );
    let inspect = &module.functions[0];
    assert!(inspect.blocks.iter().any(|block| {
        block.instructions.is_empty() && matches!(block.terminator, Terminator::Unreachable)
    }));
    assert!(
        inspect.blocks[1]
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ReadField { .. }))
    );
}
