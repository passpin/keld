use keld_ir::{
    AllocationPhase, AllocationSchedule, Function, FunctionId, Instruction, IrBlock, IrBlockId,
    IrType, Module, Register, RegisterStorage, Terminator,
};
use keld_source::{SourceId, Span};

fn sample_module(reverse_functions: bool) -> Module {
    let span = Span::new(SourceId(0), 0, 1).expect("span");
    let first = Function {
        id: FunctionId(4),
        span,
        parameters: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Text, IrType::Int],
        register_storage: vec![
            RegisterStorage::Trivial,
            RegisterStorage::Home {
                scope: keld_flow::StorageScopeId(0),
                conditional: false,
            },
            RegisterStorage::Trivial,
        ],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![IrBlock {
            id: IrBlockId(3),
            instructions: vec![Instruction::ConstText {
                dst: Register(1),
                value: "x".to_owned(),
                span,
            }],
            terminator: Terminator::Return(Some(Register(2))),
        }],
        entry: IrBlockId(3),
    };
    let second = Function {
        id: FunctionId(1),
        span,
        parameters: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Text, IrType::Text, IrType::Int],
        register_storage: vec![
            RegisterStorage::Trivial,
            RegisterStorage::Home {
                scope: keld_flow::StorageScopeId(0),
                conditional: false,
            },
            RegisterStorage::Home {
                scope: keld_flow::StorageScopeId(0),
                conditional: false,
            },
            RegisterStorage::Trivial,
        ],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![IrBlock {
            id: IrBlockId(0),
            instructions: vec![
                Instruction::ConstText {
                    dst: Register(1),
                    value: "x".to_owned(),
                    span,
                },
                Instruction::Copy {
                    dst: Register(2),
                    src: Register(1),
                    span,
                },
            ],
            terminator: Terminator::Return(Some(Register(3))),
        }],
        entry: IrBlockId(0),
    };
    let functions = if reverse_functions {
        vec![first, second]
    } else {
        vec![second, first]
    };
    Module {
        definitions: Vec::new(),
        functions,
        main: FunctionId(1),
    }
}

#[test]
fn allocation_schedule_is_independent_of_storage_order_and_tracks_preorder_ordinals() {
    let forward = AllocationSchedule::from_module(&sample_module(false));
    let reversed = AllocationSchedule::from_module(&sample_module(true));
    assert_eq!(forward, reversed);

    let base = forward
        .base_id(FunctionId(1), IrBlockId(0), 1)
        .expect("copy coordinate");
    assert_eq!(
        forward.site_id(base, AllocationPhase::Copy, 0),
        forward.site_id(base, AllocationPhase::Copy, 0)
    );
    assert_ne!(
        forward.site_id(base, AllocationPhase::Copy, 0),
        forward.site_id(base, AllocationPhase::Copy, 1)
    );
    assert_ne!(
        forward.site_id(base, AllocationPhase::Copy, 0),
        forward.site_id(base, AllocationPhase::Handle, 0)
    );
}
