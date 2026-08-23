use keld_ir::{
    Instruction, IrType, Register, TestModuleBuilder, ViewId, ViewMode, lower, validate,
};
use keld_lifecycle::verify_text_for_test;
use keld_semantics::DefId;
use keld_source::{SourceId, Span};

fn span() -> Span {
    Span::new(SourceId(0), 0, 1).unwrap()
}

#[test]
fn structural_operation_with_an_open_view_is_invalid() {
    let module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(0),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::RetireEntity {
            entity: Register(0),
            span: span(),
        })
        .finish();

    let diagnostics = validate(&module);
    assert_eq!(diagnostics[0].code.0, "KLD9001");
}

#[test]
fn a_read_view_cannot_be_used_to_write() {
    let module = TestModuleBuilder::new()
        .parameter(Register(0), IrType::Entity(DefId(0)))
        .parameter(Register(1), IrType::Int)
        .instruction(Instruction::OpenView {
            view: ViewId(0),
            entity: Register(0),
            mode: ViewMode::Read,
            span: span(),
        })
        .instruction(Instruction::WriteField {
            view: ViewId(0),
            field: keld_semantics::FieldId(0),
            value: Register(1),
            span: span(),
        })
        .instruction(Instruction::CloseView {
            view: ViewId(0),
            span: span(),
        })
        .finish();

    assert_eq!(validate(&module)[0].code.0, "KLD9001");
}

#[test]
fn register_use_must_be_dominated_by_its_definition() {
    let mut module = TestModuleBuilder::new().finish();
    module.functions[0].register_types.push(IrType::Int);
    module.functions[0].blocks[0].instructions.insert(
        0,
        Instruction::Copy {
            dst: Register(2),
            src: Register(1),
            span: span(),
        },
    );

    assert!(
        validate(&module)
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD9002")
    );
}

#[test]
fn constructors_and_calls_require_complete_unique_arguments() {
    let verified = verify_text_for_test(
        "struct Pair {\nleft: Int\nright: Int\n}\nfn add(a: Int, b: Int) -> Int { return a + b }\nfn main() -> Int { let pair = Pair(left: 1, right: 2); return add(pair.left, pair.right) }\n",
    )
    .module
    .expect("program must verify");
    let mut module = lower(&verified);
    for instruction in module.functions.iter_mut().flat_map(|function| {
        function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
    }) {
        match instruction {
            Instruction::ConstructStruct { fields, .. } => {
                fields.pop();
            }
            Instruction::Call { arguments, .. } => {
                arguments.pop();
            }
            _ => {}
        }
    }

    assert!(
        validate(&module)
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "KLD9002")
    );
}
