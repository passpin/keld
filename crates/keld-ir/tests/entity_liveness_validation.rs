use keld_ir::{
    Function, Instruction, IrBlock, IrBlockId, IrDefinition, IrDefinitionKind, IrType, Module,
    Register, RegisterStorage, Terminator, TestModuleBuilder, ViewId, ViewMode, validate,
};
use keld_semantics::{DefId, FieldId, FunctionId, ParameterMode};
use keld_source::{Diagnostic, SourceId, Span};
use keld_storage::LoanEffect;

fn span() -> Span {
    Span::new(SourceId(0), 0, 1).unwrap()
}

fn assert_retired_entity_rejected(diagnostics: &[Diagnostic]) {
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code.0 == "KLD9002"
                && diagnostic
                    .primary
                    .message
                    .contains("entity identity is retired")
        }),
        "expected retired-entity diagnostic, got {diagnostics:#?}"
    );
}

#[test]
fn direct_double_retire_is_rejected() {
    let module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .instruction(Instruction::RetireEntity {
            entity: Register(0),
            span: span(),
        })
        .instruction(Instruction::RetireEntity {
            entity: Register(0),
            span: span(),
        })
        .finish();

    assert_retired_entity_rejected(&validate(&module));
}

#[test]
fn copy_alias_use_after_retire_is_rejected() {
    let mut module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .instruction(Instruction::Copy {
            dst: Register(3),
            src: Register(0),
            span: span(),
        })
        .instruction(Instruction::RetireEntity {
            entity: Register(0),
            span: span(),
        })
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(3),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::CloseView {
            view: ViewId(0),
            span: span(),
        })
        .finish();
    module.functions[0]
        .register_types
        .push(IrType::Entity(DefId(0)));
    module.functions[0]
        .register_storage
        .push(RegisterStorage::EntityFlow);

    assert_retired_entity_rejected(&validate(&module));
}

#[test]
fn retiring_a_phi_rejects_use_of_any_possible_input_identity() {
    let module = phi_retirement_module();

    assert_retired_entity_rejected(&validate(&module));
}

#[test]
fn identity_retired_on_one_predecessor_is_not_live_after_the_join() {
    let module = maybe_retired_copy_alias_module();

    assert_retired_entity_rejected(&validate(&module));
}

#[test]
fn retirement_before_an_entry_backedge_is_rejected() {
    let test_span = span();
    let entity = DefId(0);
    let module = Module {
        definitions: vec![entity_definition(entity)],
        functions: vec![Function {
            id: FunctionId(0),
            span: test_span,
            parameters: vec![Register(0)],
            locals: Vec::new(),
            parameter_modes: vec![ParameterMode::Loan],
            parameter_effects: vec![LoanEffect::Read],
            current_lifecycle: Register(1),
            register_types: vec![IrType::Entity(entity), IrType::Lifecycle],
            register_storage: vec![RegisterStorage::EntityFlow, RegisterStorage::Trivial],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![IrBlock {
                id: IrBlockId(0),
                instructions: vec![Instruction::RetireEntity {
                    entity: Register(0),
                    span: test_span,
                }],
                terminator: Terminator::Goto(IrBlockId(0)),
            }],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    };

    assert_retired_entity_rejected(&validate(&module));
}

#[test]
fn retiring_one_parameter_does_not_retire_an_independent_entity_parameter() {
    let module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .parameter(Register(1), IrType::Entity(DefId(0)))
        .instruction(Instruction::RetireEntity {
            entity: Register(0),
            span: span(),
        })
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(1),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::CloseView {
            view: ViewId(0),
            span: span(),
        })
        .finish();

    assert_eq!(validate(&module), []);
}

fn phi_retirement_module() -> Module {
    let test_span = span();
    let entity = DefId(0);
    Module {
        definitions: vec![entity_definition(entity)],
        functions: vec![Function {
            id: FunctionId(0),
            span: test_span,
            parameters: vec![Register(0), Register(1), Register(2)],
            locals: Vec::new(),
            parameter_modes: vec![ParameterMode::Loan; 3],
            parameter_effects: vec![LoanEffect::Read; 3],
            current_lifecycle: Register(3),
            register_types: vec![
                IrType::Entity(entity),
                IrType::Entity(entity),
                IrType::Bool,
                IrType::Lifecycle,
                IrType::Int,
                IrType::Entity(entity),
            ],
            register_storage: vec![
                RegisterStorage::EntityFlow,
                RegisterStorage::EntityFlow,
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
                RegisterStorage::EntityFlow,
            ],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![
                IrBlock {
                    id: IrBlockId(0),
                    instructions: vec![],
                    terminator: Terminator::Branch {
                        condition: Register(2),
                        then_block: IrBlockId(1),
                        else_block: IrBlockId(2),
                    },
                },
                IrBlock {
                    id: IrBlockId(1),
                    instructions: vec![],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(2),
                    instructions: vec![],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(3),
                    instructions: vec![
                        Instruction::Phi {
                            dst: Register(5),
                            inputs: vec![(IrBlockId(1), Register(0)), (IrBlockId(2), Register(1))],
                            span: test_span,
                        },
                        Instruction::RetireEntity {
                            entity: Register(5),
                            span: test_span,
                        },
                        Instruction::OpenView {
                            view: ViewId(0),
                            entity: Register(0),
                            mode: ViewMode::Read,
                            span: test_span,
                        },
                        Instruction::CloseView {
                            view: ViewId(0),
                            span: test_span,
                        },
                        Instruction::ConstInt {
                            dst: Register(4),
                            value: 0,
                            span: test_span,
                        },
                    ],
                    terminator: Terminator::Return(Some(Register(4))),
                },
            ],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    }
}

fn maybe_retired_copy_alias_module() -> Module {
    let test_span = span();
    let entity = DefId(0);
    Module {
        definitions: vec![entity_definition(entity)],
        functions: vec![Function {
            id: FunctionId(0),
            span: test_span,
            parameters: vec![Register(0), Register(1)],
            locals: Vec::new(),
            parameter_modes: vec![ParameterMode::Loan; 2],
            parameter_effects: vec![LoanEffect::Read; 2],
            current_lifecycle: Register(2),
            register_types: vec![
                IrType::Entity(entity),
                IrType::Bool,
                IrType::Lifecycle,
                IrType::Int,
                IrType::Entity(entity),
            ],
            register_storage: vec![
                RegisterStorage::EntityFlow,
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
                RegisterStorage::Trivial,
                RegisterStorage::EntityFlow,
            ],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![
                IrBlock {
                    id: IrBlockId(0),
                    instructions: vec![Instruction::Copy {
                        dst: Register(4),
                        src: Register(0),
                        span: test_span,
                    }],
                    terminator: Terminator::Branch {
                        condition: Register(1),
                        then_block: IrBlockId(1),
                        else_block: IrBlockId(2),
                    },
                },
                IrBlock {
                    id: IrBlockId(1),
                    instructions: vec![Instruction::RetireEntity {
                        entity: Register(0),
                        span: test_span,
                    }],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(2),
                    instructions: vec![],
                    terminator: Terminator::Goto(IrBlockId(3)),
                },
                IrBlock {
                    id: IrBlockId(3),
                    instructions: vec![
                        Instruction::OpenView {
                            view: ViewId(0),
                            entity: Register(4),
                            mode: ViewMode::Read,
                            span: test_span,
                        },
                        Instruction::CloseView {
                            view: ViewId(0),
                            span: test_span,
                        },
                        Instruction::ConstInt {
                            dst: Register(3),
                            value: 0,
                            span: test_span,
                        },
                    ],
                    terminator: Terminator::Return(Some(Register(3))),
                },
            ],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    }
}

fn entity_definition(id: DefId) -> IrDefinition {
    IrDefinition {
        id,
        kind: IrDefinitionKind::Entity,
        fields: vec![(FieldId(0), IrType::Int)],
    }
}
