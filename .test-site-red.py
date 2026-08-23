from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
marker = "#[cfg(test)]\nmod text_ir_audit {"
start = text.find(marker)
if start < 0:
    raise SystemExit("text_ir_audit module not found")
replacement = r'''#[cfg(test)]
mod text_ir_audit {
    use super::*;
    use keld_ir::{Function, IrBlock, RegisterStorage};
    use keld_source::{SourceId, Span};

    fn text_module() -> Module {
        let span = Span::new(SourceId(0), 0, 0).expect("span");
        let register_types = vec![
            IrType::Lifecycle,
            IrType::Text,
            IrType::Text,
            IrType::Text,
            IrType::Bool,
            IrType::Int,
        ];
        let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
        for index in 1..=3 {
            register_storage[index] = RegisterStorage::Home {
                scope: keld_flow::StorageScopeId(0),
                conditional: false,
            };
        }
        Module {
            definitions: Vec::new(),
            functions: vec![Function {
                id: FunctionId(0),
                span,
                parameters: Vec::new(),
                locals: Vec::new(),
                parameter_modes: Vec::new(),
                parameter_effects: Vec::new(),
                current_lifecycle: Register(0),
                register_types,
                register_storage,
                storage_scope_parents: vec![None],
                return_type: IrType::Int,
                blocks: vec![IrBlock {
                    id: IrBlockId(0),
                    instructions: vec![
                        Instruction::ConstText {
                            dst: Register(1),
                            value: "abc".to_owned(),
                            span,
                        },
                        Instruction::ConstText {
                            dst: Register(2),
                            value: "!".to_owned(),
                            span,
                        },
                        Instruction::TextConcat {
                            dst: Register(3),
                            lhs: Register(1),
                            rhs: Register(2),
                            span,
                        },
                        Instruction::Compare {
                            dst: Register(4),
                            op: CompareOp::Eq,
                            lhs: Register(3),
                            rhs: Register(3),
                            span,
                        },
                        Instruction::DropHome {
                            home: Register(3),
                            span,
                        },
                        Instruction::DropHome {
                            home: Register(2),
                            span,
                        },
                        Instruction::DropHome {
                            home: Register(1),
                            span,
                        },
                        Instruction::ConstInt {
                            dst: Register(5),
                            value: 0,
                            span,
                        },
                    ],
                    terminator: Terminator::Return(Some(Register(5))),
                }],
                entry: IrBlockId(0),
            }],
            main: FunctionId(0),
        }
    }

    #[test]
    fn production_text_ir_has_no_test_site_calls() {
        let module = text_module();
        assert!(validate(&module).is_empty());
        let source = SourceText::from_str(SourceId(0), "text audit").expect("source");
        let ir = lower_scalar_ir(
            &module,
            &SourceMetadata {
                path: PathBuf::from("text-audit.keld"),
                source,
            },
        )
        .expect("LLVM IR lowering");
        assert_eq!(ir.matches("call i32 @keld_rt_v1_text_concat").count(), 1);
        assert_eq!(
            ir.matches("call i32 @keld_rt_v1_test_site").count(),
            0,
            "production codegen must not cross the test-control ABI"
        );
    }
}
'''
path.write_text(text[:start] + replacement, encoding="utf-8")
