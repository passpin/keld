use keld_ir::{
    ArgumentProjection, ArgumentSource, Instruction, IrType, Register, RegisterStorage,
    TestModuleBuilder, ViewId, ViewMode, validate,
};
use keld_semantics::{DefId, FieldId};
use keld_source::{SourceId, Span};

fn span() -> Span {
    Span::new(SourceId(0), 0, 1).unwrap()
}

fn assert_open_view_error(module: &keld_ir::Module) {
    let diagnostics = validate(module);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD9001"),
        "{diagnostics:#?}"
    );
}

#[test]
fn projected_list_push_is_rejected_while_entity_view_is_open() {
    let mut module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .parameter(Register(1), IrType::List(Box::new(IrType::Int)))
        .parameter(Register(2), IrType::Int)
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(0),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::ListPushPlace {
            list: Register(1),
            source: ArgumentSource {
                base: Register(0),
                projections: vec![ArgumentProjection::Field(FieldId(0))],
            },
            value: Register(2),
            span: span(),
        })
        .instruction(Instruction::CloseView {
            view: ViewId(0),
            span: span(),
        })
        .finish();

    module.definitions[0].fields[0].1 = IrType::List(Box::new(IrType::Int));
    module.functions[0].register_storage[1] = RegisterStorage::Loan;

    assert_open_view_error(&module);
}

#[test]
fn projected_list_remove_is_rejected_while_entity_view_is_open() {
    let mut module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .parameter(Register(1), IrType::List(Box::new(IrType::Int)))
        .parameter(Register(2), IrType::Int)
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(0),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::ListRemovePlace {
            dst: Register(5),
            list: Register(1),
            source: ArgumentSource {
                base: Register(0),
                projections: vec![ArgumentProjection::Field(FieldId(0))],
            },
            index: Register(2),
            span: span(),
        })
        .instruction(Instruction::CloseView {
            view: ViewId(0),
            span: span(),
        })
        .finish();

    module.definitions[0].fields[0].1 = IrType::List(Box::new(IrType::Int));
    module.functions[0].register_storage[1] = RegisterStorage::Loan;
    module.functions[0].register_types.push(IrType::Int);
    module.functions[0]
        .register_storage
        .push(RegisterStorage::Trivial);

    assert_open_view_error(&module);
}
