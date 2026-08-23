use keld_flow::StorageScopeId;
use keld_ir::{
    ArgumentProjection, ArgumentSource, Instruction, IntBinaryOp, IrType, Register, RegisterStorage,
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

#[test]
fn copy_cannot_overwrite_a_live_managed_local_home() {
    let mut module = TestModuleBuilder::new().finish();
    let local = Register(2);
    let first = Register(3);
    let second = Register(4);
    let function = &mut module.functions[0];
    function.locals.push(local);
    for _ in 0..3 {
        function.register_types.push(IrType::Text);
        function.register_storage.push(RegisterStorage::Home {
            scope: StorageScopeId(0),
            conditional: false,
        });
    }
    function.blocks[0].instructions.splice(
        0..0,
        [
            Instruction::ConstText {
                dst: first,
                value: "first".to_owned(),
                span: span(),
            },
            Instruction::Copy {
                dst: local,
                src: first,
                span: span(),
            },
            Instruction::ConstText {
                dst: second,
                value: "second".to_owned(),
                span: span(),
            },
            Instruction::Copy {
                dst: local,
                src: second,
                span: span(),
            },
            Instruction::DropHome {
                home: local,
                span: span(),
            },
            Instruction::DropHome {
                home: second,
                span: span(),
            },
            Instruction::DropHome {
                home: first,
                span: span(),
            },
        ],
    );

    let diagnostics = validate(&module);
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code.0 == "KLD9006"
                && diagnostic.primary.message == "copy destination is already live"
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn scalar_local_cannot_be_used_directly_as_an_arithmetic_operand() {
    let mut module = TestModuleBuilder::new().finish();
    let seed = Register(2);
    let local = Register(3);
    let rhs = Register(4);
    let sum = Register(5);
    let function = &mut module.functions[0];
    function.locals.push(local);
    for _ in 0..4 {
        function.register_types.push(IrType::Int);
        function.register_storage.push(RegisterStorage::Trivial);
    }
    function.blocks[0].instructions.splice(
        0..0,
        [
            Instruction::ConstInt {
                dst: seed,
                value: 10,
                span: span(),
            },
            Instruction::Copy {
                dst: local,
                src: seed,
                span: span(),
            },
            Instruction::ConstInt {
                dst: rhs,
                value: 3,
                span: span(),
            },
            Instruction::CheckedBinaryInt {
                dst: sum,
                op: IntBinaryOp::Add,
                lhs: local,
                rhs,
                span: span(),
            },
        ],
    );

    let diagnostics = validate(&module);
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code.0 == "KLD9002"
                && diagnostic.primary.message == "scalar source local must be read through Copy"
        }),
        "{diagnostics:#?}"
    );
}
