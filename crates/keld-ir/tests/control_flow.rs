use keld_ir::{
    Function, Instruction, IrBlock, IrBlockId, IrType, Module, Register, RegisterStorage, Terminator,
    lower, validate,
};
use keld_semantics::FunctionId;
use keld_source::{SourceId, Span};
use keld_storage::verify_text_for_test;

fn successors(terminator: &Terminator) -> Vec<IrBlockId> {
    match terminator {
        Terminator::Goto(target) => vec![*target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::ResolveLink { live, absent, .. } => vec![*live, *absent],
        Terminator::Return(_) | Terminator::Fault { .. } | Terminator::Unreachable => Vec::new(),
    }
}

#[test]
fn source_loop_lowers_to_valid_cyclic_executable_ir() {
    let source = include_str!("../../keld-cli/tests/fixtures/control_flow_loop.keld");
    let verified = verify_text_for_test(source)
        .module
        .expect("Control Flow-1 fixture must verify through storage");
    let module = lower(&verified);
    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));

    let function = &module.functions[module.main.0 as usize];
    assert!(function.blocks.iter().any(|block| {
        successors(&block.terminator)
            .into_iter()
            .any(|successor| successor.0 <= block.id.0)
    }), "lowered while must contain a real CFG back-edge");
}

#[test]
fn validator_accepts_a_minimal_valid_cycle() {
    let span = Span::new(SourceId(0), 0, 0).expect("empty test span is valid");
    let module = Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: FunctionId(0),
            span,
            parameters: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types: vec![IrType::Lifecycle, IrType::Int, IrType::Bool],
            register_storage: vec![
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
            ],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![
                IrBlock {
                    id: IrBlockId(0),
                    instructions: vec![
                        Instruction::ConstInt {
                            dst: Register(1),
                            value: 0,
                            span,
                        },
                        Instruction::ConstBool {
                            dst: Register(2),
                            value: true,
                            span,
                        },
                    ],
                    terminator: Terminator::Goto(IrBlockId(1)),
                },
                IrBlock {
                    id: IrBlockId(1),
                    instructions: Vec::new(),
                    terminator: Terminator::Branch {
                        condition: Register(2),
                        then_block: IrBlockId(1),
                        else_block: IrBlockId(2),
                    },
                },
                IrBlock {
                    id: IrBlockId(2),
                    instructions: Vec::new(),
                    terminator: Terminator::Return(Some(Register(1))),
                },
            ],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    };

    assert!(validate(&module).is_empty(), "{:#?}", validate(&module));
}
