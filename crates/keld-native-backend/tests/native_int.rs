use keld_ir::{
    AllocationPhase, AllocationSchedule, Function, Instruction, IntBinaryOp, IrBlock, IrBlockId,
    IrType, Module, Register, RegisterStorage, Terminator,
};
use keld_native_backend::{
    BackendError, BuildRequest, NativeArtifact, OptimizationLevel, SourceMetadata,
    build_executable, build_executable_with_test_controls,
};
use keld_source::{SourceId, SourceText, Span};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_TEMP: AtomicU64 = AtomicU64::new(1);
type AllocationCase = (&'static str, &'static str, fn() -> Module);

fn span() -> Span {
    Span::new(SourceId(0), 0, 0).expect("empty source span")
}

fn const_module(value: i64) -> Module {
    let span = span();
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types: vec![IrType::Lifecycle, IrType::Int],
            register_storage: vec![RegisterStorage::Trivial, RegisterStorage::Trivial],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![IrBlock {
                id: IrBlockId(0),
                instructions: vec![Instruction::ConstInt {
                    dst: Register(1),
                    value,
                    span,
                }],
                terminator: Terminator::Return(Some(Register(1))),
            }],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

fn scalar_module(register_types: Vec<IrType>, blocks: Vec<IrBlock>) -> Module {
    let span = span();
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_storage: vec![RegisterStorage::Trivial; register_types.len()],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            register_types,
            blocks,
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

fn block(id: u32, instructions: Vec<Instruction>, terminator: Terminator) -> IrBlock {
    IrBlock {
        id: IrBlockId(id),
        instructions,
        terminator,
    }
}

fn metadata() -> SourceMetadata {
    SourceMetadata {
        path: PathBuf::from("native-int.keld"),
        source: SourceText::from_str(SourceId(0), "fn main() -> Int { return 42 }\n")
            .expect("source text"),
    }
}

fn request(
    output: &Path,
    runtime_dll: &Path,
    import_library: &Path,
    optimization: OptimizationLevel,
) -> BuildRequest {
    let prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(".tools/llvm/22.1.8-mingw64"))
        })
        .expect("LLVM prefix");
    BuildRequest {
        output: output.to_path_buf(),
        optimization,
        llvm_prefix: prefix,
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll: runtime_dll.to_path_buf(),
        runtime_import_library: import_library.to_path_buf(),
    }
}

fn add_context_setup_export(path: &Path) {
    let exports = std::fs::read_to_string(path)
        .expect("runtime export list must be readable")
        .replace(
            "keld_rt_v1_context_new\n",
            "keld_rt_v1_context_new\nkeld_rt_v1_context_new_at\nkeld_rt_v1_test_site\n",
        );
    std::fs::write(path, exports).expect("runtime export list must be writable");
}

fn native_temp(tag: &str) -> PathBuf {
    let sequence = NEXT_TEST_TEMP.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "keld-native-{tag}-{}-{sequence}",
        std::process::id()
    ))
}

fn runtime_artifacts(tag: &str) -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let ffi_dll = root.join("target/x86_64-pc-windows-gnu/release/keld_runtime_v1.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {}",
        ffi_dll.display()
    );
    runtime_artifacts_with_dll(tag, &ffi_dll)
}

fn runtime_artifacts_with_dll(tag: &str, ffi_dll: &Path) -> (PathBuf, PathBuf) {
    let temp = native_temp(tag);
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\nkeld_rt_v1_context_new\nkeld_rt_v1_context_destroy\nkeld_rt_v1_context_status\nkeld_rt_v1_context_fault\nkeld_rt_v1_context_fault_parts\nkeld_rt_v1_context_root_lifecycle\nkeld_rt_v1_value_copy\nkeld_rt_v1_value_drop\nkeld_rt_v1_text_new\nkeld_rt_v1_text_byte_length\nkeld_rt_v1_text_is_empty\nkeld_rt_v1_text_equal\nkeld_rt_v1_text_concat\nkeld_rt_v1_list_new\nkeld_rt_v1_list_length\nkeld_rt_v1_list_push\nkeld_rt_v1_list_get\nkeld_rt_v1_list_remove\nkeld_rt_v1_list_replace\nkeld_rt_v1_list_try_remove\nkeld_rt_v1_list_clear\nkeld_rt_v1_list_reserve\nkeld_rt_v1_list_try_reserve\nkeld_rt_v1_struct_new\nkeld_rt_v1_struct_field\nkeld_rt_v1_place_resolve\nkeld_rt_v1_place_replace\nkeld_rt_v1_home_track\nkeld_rt_v1_home_untrack\nkeld_rt_v1_cleanup_scope\nkeld_rt_v1_begin_lifecycle\nkeld_rt_v1_end_lifecycle\nkeld_rt_v1_allocate_entity\nkeld_rt_v1_entity_to_link\nkeld_rt_v1_resolve_link\nkeld_rt_v1_entity_field\nkeld_rt_v1_replace_field\nkeld_rt_v1_keep_entity\nkeld_rt_v1_retire_entity\n",
    )
    .expect("runtime export list");
    add_context_setup_export(&export_list);
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(dlltool)
        .args([
            "--input-def",
            export_list.to_str().expect("export list path"),
            "--dllname",
            "keld_runtime_v1.dll",
            "--output-lib",
        ])
        .arg(&import_library)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "dlltool failed: {status}");
    (runtime_dll, import_library)
}

fn test_runtime_artifacts(tag: &str) -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let ffi_dll = root.join("target/x86_64-pc-windows-gnu/release/keld_runtime_v1_test.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi-test first: {}",
        ffi_dll.display()
    );
    runtime_artifacts_with_dll(tag, &ffi_dll)
}

fn run_test_runtime(
    tag: &str,
    module: &Module,
    metadata: &SourceMetadata,
    controls: &str,
    optimization: OptimizationLevel,
) -> std::process::Output {
    let (runtime_dll, import_library) = test_runtime_artifacts(tag);
    let directory = runtime_dll.parent().expect("runtime directory");
    let control = directory.join("control.txt");
    let observation = directory.join("observation.txt");
    std::fs::write(&control, controls).expect("control file");
    let output = directory.join("program.exe");
    let artifact = build_executable_with_test_controls(
        module,
        metadata,
        &request(&output, &runtime_dll, &import_library, optimization),
    )
    .expect("native test-runtime build");
    Command::new(&artifact.executable)
        .env("KELD_TEST_CONTROL", control)
        .env("KELD_TEST_OBSERVATION", observation)
        .output()
        .expect("native test-runtime executable")
}

fn binary_module(lhs: i64, op: IntBinaryOp, rhs: i64) -> Module {
    let span = span();
    scalar_module(
        vec![IrType::Lifecycle, IrType::Int, IrType::Int, IrType::Int],
        vec![block(
            0,
            vec![
                Instruction::ConstInt {
                    dst: Register(1),
                    value: lhs,
                    span,
                },
                Instruction::ConstInt {
                    dst: Register(2),
                    value: rhs,
                    span,
                },
                Instruction::CheckedBinaryInt {
                    dst: Register(3),
                    op,
                    lhs: Register(1),
                    rhs: Register(2),
                    span,
                },
            ],
            Terminator::Return(Some(Register(3))),
        )],
    )
}

fn text_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::Text,
        IrType::Text,
        IrType::Text,
        IrType::Int,
        IrType::Bool,
        IrType::Bool,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    register_storage[1] = RegisterStorage::Home {
        scope: keld_flow::StorageScopeId(0),
        conditional: false,
    };
    register_storage[2] = register_storage[1].clone();
    register_storage[3] = register_storage[1].clone();
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![block(
                0,
                vec![
                    Instruction::ConstText {
                        dst: Register(1),
                        value: "hé".to_owned(),
                        span: source_span,
                    },
                    Instruction::Copy {
                        dst: Register(2),
                        src: Register(1),
                        span: source_span,
                    },
                    Instruction::TextConcat {
                        dst: Register(3),
                        lhs: Register(1),
                        rhs: Register(2),
                        span: source_span,
                    },
                    Instruction::TextByteLength {
                        dst: Register(4),
                        text: Register(3),
                        span: source_span,
                    },
                    Instruction::TextIsEmpty {
                        dst: Register(5),
                        text: Register(3),
                        span: source_span,
                    },
                    Instruction::Compare {
                        dst: Register(6),
                        op: keld_ir::CompareOp::Eq,
                        lhs: Register(1),
                        rhs: Register(2),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(3),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(2),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(1),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(7),
                        value: 4,
                        span: source_span,
                    },
                ],
                Terminator::Return(Some(Register(7))),
            )],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

#[allow(clippy::too_many_lines)]
fn list_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::List(Box::new(IrType::Int)),
        IrType::Int,
        IrType::Int,
        IrType::Int,
        IrType::Optional(Box::new(IrType::Int)),
        IrType::Int,
        IrType::Optional(Box::new(IrType::Int)),
        IrType::Bool,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    register_storage[1] = RegisterStorage::Home {
        scope: keld_flow::StorageScopeId(0),
        conditional: false,
    };
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![block(
                0,
                vec![
                    Instruction::ListNew {
                        dst: Register(1),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(2),
                        value: 7,
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(3),
                        value: 0,
                        span: source_span,
                    },
                    Instruction::ListPush {
                        list: Register(1),
                        value: Register(2),
                        span: source_span,
                    },
                    Instruction::ListLength {
                        dst: Register(4),
                        list: Register(1),
                        span: source_span,
                    },
                    Instruction::ListGet {
                        dst: Register(5),
                        receiver: keld_ir::Receiver {
                            list: Register(1),
                            source: None,
                        },
                        index: Register(3),
                        span: source_span,
                    },
                    Instruction::ListRemove {
                        dst: Register(6),
                        list: Register(1),
                        index: Register(3),
                        span: source_span,
                    },
                    Instruction::ListTryRemove {
                        dst: Register(7),
                        receiver: keld_ir::Receiver {
                            list: Register(1),
                            source: None,
                        },
                        index: Register(3),
                        span: source_span,
                    },
                    Instruction::ListTryReserve {
                        dst: Register(8),
                        receiver: keld_ir::Receiver {
                            list: Register(1),
                            source: None,
                        },
                        additional: Register(4),
                        span: source_span,
                    },
                    Instruction::ListReserve {
                        receiver: keld_ir::Receiver {
                            list: Register(1),
                            source: None,
                        },
                        additional: Register(4),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(1),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(9),
                        value: 7,
                        span: source_span,
                    },
                ],
                Terminator::Return(Some(Register(9))),
            )],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

#[allow(clippy::too_many_lines)]
fn projected_list_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::List(Box::new(IrType::List(Box::new(IrType::Int)))),
        IrType::List(Box::new(IrType::Int)),
        IrType::Int,
        IrType::Int,
        IrType::List(Box::new(IrType::Int)),
        IrType::Int,
        IrType::Int,
        IrType::Int,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    for register in [1_u32, 2] {
        register_storage[register as usize] = RegisterStorage::Home {
            scope: keld_flow::StorageScopeId(0),
            conditional: false,
        };
    }
    register_storage[5] = RegisterStorage::Loan;
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![block(
                0,
                vec![
                    Instruction::ListNew {
                        dst: Register(1),
                        span: source_span,
                    },
                    Instruction::ListNew {
                        dst: Register(2),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(3),
                        value: 7,
                        span: source_span,
                    },
                    Instruction::ListPush {
                        list: Register(2),
                        value: Register(3),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(4),
                        value: 0,
                        span: source_span,
                    },
                    Instruction::ListPush {
                        list: Register(1),
                        value: Register(2),
                        span: source_span,
                    },
                    Instruction::ListIndex {
                        dst: Register(5),
                        receiver: keld_ir::Receiver {
                            list: Register(1),
                            source: None,
                        },
                        index: Register(4),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(6),
                        value: 9,
                        span: source_span,
                    },
                    Instruction::ListPushPlace {
                        list: Register(5),
                        source: keld_ir::ArgumentSource {
                            base: Register(1),
                            projections: vec![keld_ir::ArgumentProjection::Index(Register(4))],
                        },
                        value: Register(6),
                        span: source_span,
                    },
                    Instruction::ListLength {
                        dst: Register(8),
                        list: Register(1),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(1),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(9),
                        value: 9,
                        span: source_span,
                    },
                ],
                Terminator::Return(Some(Register(9))),
            )],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

fn managed_phi_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::Bool,
        IrType::Text,
        IrType::Text,
        IrType::Text,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    for register in [2_u32, 3, 4] {
        register_storage[register as usize] = RegisterStorage::Home {
            scope: keld_flow::StorageScopeId(0),
            conditional: false,
        };
    }
    Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![
                block(
                    0,
                    vec![Instruction::ConstBool {
                        dst: Register(1),
                        value: true,
                        span: source_span,
                    }],
                    Terminator::Branch {
                        condition: Register(1),
                        then_block: IrBlockId(1),
                        else_block: IrBlockId(2),
                    },
                ),
                block(
                    1,
                    vec![Instruction::ConstText {
                        dst: Register(2),
                        value: "then".to_owned(),
                        span: source_span,
                    }],
                    Terminator::Goto(IrBlockId(3)),
                ),
                block(
                    2,
                    vec![Instruction::ConstText {
                        dst: Register(3),
                        value: "else".to_owned(),
                        span: source_span,
                    }],
                    Terminator::Goto(IrBlockId(3)),
                ),
                block(
                    3,
                    vec![
                        Instruction::Phi {
                            dst: Register(4),
                            inputs: vec![(IrBlockId(1), Register(2)), (IrBlockId(2), Register(3))],
                            span: source_span,
                        },
                        Instruction::TextByteLength {
                            dst: Register(5),
                            text: Register(4),
                            span: source_span,
                        },
                        Instruction::DropHome {
                            home: Register(4),
                            span: source_span,
                        },
                        Instruction::CleanupTrackedScope {
                            scope: keld_flow::StorageScopeId(0),
                            span: source_span,
                        },
                    ],
                    Terminator::Return(Some(Register(5))),
                ),
            ],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

#[allow(clippy::too_many_lines)]
fn struct_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::Text,
        IrType::Struct(keld_ir::DefId(0)),
        IrType::Int,
        IrType::Text,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    for register in [1_u32, 2] {
        register_storage[register as usize] = RegisterStorage::Home {
            scope: keld_flow::StorageScopeId(0),
            conditional: false,
        };
    }
    register_storage[4] = RegisterStorage::Loan;
    Module {
        definitions: vec![keld_ir::IrDefinition {
            id: keld_ir::DefId(0),
            kind: keld_ir::IrDefinitionKind::Struct,
            fields: vec![
                (keld_ir::FieldId(0), IrType::Int),
                (keld_ir::FieldId(1), IrType::Text),
            ],
        }],
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![block(
                0,
                vec![
                    Instruction::ConstText {
                        dst: Register(1),
                        value: "struct".to_owned(),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(3),
                        value: 11,
                        span: source_span,
                    },
                    Instruction::ConstructStruct {
                        dst: Register(2),
                        definition: keld_ir::DefId(0),
                        fields: vec![
                            (keld_ir::FieldId(0), Register(3)),
                            (keld_ir::FieldId(1), Register(1)),
                        ],
                        span: source_span,
                    },
                    Instruction::ReadStructField {
                        dst: Register(4),
                        base: Register(2),
                        field: keld_ir::FieldId(1),
                        span: source_span,
                    },
                    Instruction::DropHome {
                        home: Register(2),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(5),
                        value: 11,
                        span: source_span,
                    },
                ],
                Terminator::Return(Some(Register(5))),
            )],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

#[allow(clippy::too_many_lines)]
fn entity_module() -> Module {
    let source_span = span();
    let register_types = vec![
        IrType::Lifecycle,
        IrType::Text,
        IrType::Entity(keld_ir::DefId(0)),
        IrType::Int,
        IrType::Link {
            entity: keld_ir::DefId(0),
            optional: false,
        },
        IrType::Text,
        IrType::Int,
        IrType::Int,
    ];
    let mut register_storage = vec![RegisterStorage::Trivial; register_types.len()];
    register_storage[1] = RegisterStorage::Home {
        scope: keld_flow::StorageScopeId(0),
        conditional: false,
    };
    register_storage[2] = RegisterStorage::EntityFlow;
    register_storage[5] = RegisterStorage::Loan;
    Module {
        definitions: vec![keld_ir::IrDefinition {
            id: keld_ir::DefId(0),
            kind: keld_ir::IrDefinitionKind::Entity,
            fields: vec![
                (keld_ir::FieldId(0), IrType::Int),
                (keld_ir::FieldId(1), IrType::Text),
            ],
        }],
        functions: vec![Function {
            id: keld_semantics::FunctionId(0),
            span: source_span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types,
            register_storage,
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![block(
                0,
                vec![
                    Instruction::ConstText {
                        dst: Register(1),
                        value: "entity".to_owned(),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(3),
                        value: 17,
                        span: source_span,
                    },
                    Instruction::AllocateEntity {
                        dst: Register(2),
                        definition: keld_ir::DefId(0),
                        fields: vec![
                            (keld_ir::FieldId(0), Register(3)),
                            (keld_ir::FieldId(1), Register(1)),
                        ],
                        lifecycle: Register(0),
                        span: source_span,
                    },
                    Instruction::EntityToLink {
                        dst: Register(4),
                        entity: Register(2),
                        span: source_span,
                    },
                    Instruction::OpenView {
                        view: keld_ir::ViewId(0),
                        entity: Register(2),
                        mode: keld_ir::ViewMode::Read,
                        span: source_span,
                    },
                    Instruction::ReadField {
                        dst: Register(5),
                        view: keld_ir::ViewId(0),
                        field: keld_ir::FieldId(1),
                        span: source_span,
                    },
                    Instruction::CloseView {
                        view: keld_ir::ViewId(0),
                        span: source_span,
                    },
                    Instruction::TextByteLength {
                        dst: Register(6),
                        text: Register(5),
                        span: source_span,
                    },
                    Instruction::RetireEntity {
                        entity: Register(2),
                        span: source_span,
                    },
                    Instruction::ConstInt {
                        dst: Register(7),
                        value: 6,
                        span: source_span,
                    },
                ],
                Terminator::Return(Some(Register(7))),
            )],
            entry: IrBlockId(0),
        }],
        main: keld_semantics::FunctionId(0),
    }
}

fn unary_module(value: i64) -> Module {
    let span = span();
    scalar_module(
        vec![IrType::Lifecycle, IrType::Int, IrType::Int],
        vec![block(
            0,
            vec![
                Instruction::ConstInt {
                    dst: Register(1),
                    value,
                    span,
                },
                Instruction::CheckedUnaryInt {
                    dst: Register(2),
                    op: keld_ir::IntUnaryOp::Neg,
                    src: Register(1),
                    span,
                },
            ],
            Terminator::Return(Some(Register(2))),
        )],
    )
}

fn comparison_module(lhs: i64, op: keld_ir::CompareOp, rhs: i64) -> Module {
    let span = span();
    scalar_module(
        vec![
            IrType::Lifecycle,
            IrType::Int,
            IrType::Int,
            IrType::Bool,
            IrType::Int,
            IrType::Int,
        ],
        vec![
            block(
                0,
                vec![
                    Instruction::ConstInt {
                        dst: Register(1),
                        value: lhs,
                        span,
                    },
                    Instruction::ConstInt {
                        dst: Register(2),
                        value: rhs,
                        span,
                    },
                    Instruction::Compare {
                        dst: Register(3),
                        op,
                        lhs: Register(1),
                        rhs: Register(2),
                        span,
                    },
                    Instruction::ConstInt {
                        dst: Register(4),
                        value: 1,
                        span,
                    },
                    Instruction::ConstInt {
                        dst: Register(5),
                        value: 0,
                        span,
                    },
                ],
                Terminator::Branch {
                    condition: Register(3),
                    then_block: IrBlockId(1),
                    else_block: IrBlockId(2),
                },
            ),
            block(1, Vec::new(), Terminator::Return(Some(Register(4)))),
            block(2, Vec::new(), Terminator::Return(Some(Register(5)))),
        ],
    )
}

#[test]
fn invalid_ir_is_rejected_before_toolchain_access() {
    let mut module = const_module(1);
    module.functions[0].blocks[0].terminator = Terminator::Return(Some(Register(99)));
    let error = build_executable(
        &module,
        &metadata(),
        &request(
            Path::new("missing.exe"),
            Path::new("missing.dll"),
            Path::new("missing.dll.a"),
            OptimizationLevel::O0,
        ),
    )
    .expect_err("invalid executable IR must be rejected");
    assert!(error.is_invalid_ir());
}

#[test]
fn builds_and_runs_a_const_int_program() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let ffi_dll = target.join("keld_runtime_v1.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = native_temp("int");
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\nkeld_rt_v1_context_new\nkeld_rt_v1_context_destroy\nkeld_rt_v1_context_status\nkeld_rt_v1_context_fault\nkeld_rt_v1_context_fault_parts\nkeld_rt_v1_context_root_lifecycle\nkeld_rt_v1_value_copy\nkeld_rt_v1_value_drop\nkeld_rt_v1_text_new\nkeld_rt_v1_text_byte_length\nkeld_rt_v1_text_is_empty\nkeld_rt_v1_text_equal\nkeld_rt_v1_text_concat\nkeld_rt_v1_list_new\nkeld_rt_v1_list_length\nkeld_rt_v1_list_push\nkeld_rt_v1_list_get\nkeld_rt_v1_list_remove\nkeld_rt_v1_list_replace\nkeld_rt_v1_list_try_remove\nkeld_rt_v1_list_clear\nkeld_rt_v1_list_reserve\nkeld_rt_v1_list_try_reserve\n",
    )
    .expect("runtime export list");
    add_context_setup_export(&export_list);
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(dlltool)
        .args([
            "--input-def",
            export_list.to_str().expect("export list path"),
            "--dllname",
            "keld_runtime_v1.dll",
            "--output-lib",
        ])
        .arg(&import_library)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "dlltool failed: {status}");
    for (index, (value, optimization)) in [
        (42_i64, OptimizationLevel::O0),
        (-42_i64, OptimizationLevel::O2),
        (i64::MIN, OptimizationLevel::O0),
        (i64::MAX, OptimizationLevel::O2),
    ]
    .into_iter()
    .enumerate()
    {
        let output = temp.join(format!("native-{index}.exe"));
        let artifact: NativeArtifact = build_executable(
            &const_module(value),
            &metadata(),
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("native executable");
        assert!(child.status.success(), "native status: {}", child.status);
        assert_eq!(child.stdout, format!("{value}\n").as_bytes());
        if index == 0 {
            let objdump = std::env::var_os("KELD_OBJDUMP").unwrap_or_else(|| "objdump".into());
            let imports = Command::new(objdump)
                .arg("-p")
                .arg(&artifact.executable)
                .output()
                .expect("PE import audit tool");
            assert!(
                imports.status.success(),
                "objdump failed: {}",
                imports.status
            );
            let imports = String::from_utf8_lossy(&imports.stdout).to_ascii_lowercase();
            assert!(imports.contains("keld_runtime_v1.dll"));
            assert!(!imports.contains("libllvm-22.dll"));
            assert!(!imports.contains("libgcc"));
            assert!(!imports.contains("libwinpthread"));
        }
    }
}

#[test]
fn test_runtime_reports_context_allocation_at_main_span() {
    let (runtime_dll, import_library) = test_runtime_artifacts("context-fault");
    let directory = runtime_dll.parent().expect("runtime directory");
    let control = directory.join("control.txt");
    let observation = directory.join("observation.txt");
    std::fs::write(&control, "site_id=1 phase=context attempt=1\n").expect("control file");
    let metadata = SourceMetadata {
        path: PathBuf::from("setup.keld"),
        source: SourceText::from_str(SourceId(0), "setup\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = directory.join(format!("context-fault-{index}.exe"));
        let artifact = build_executable(
            &const_module(7),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("native context-fault build");
        let child = Command::new(&artifact.executable)
            .env("KELD_TEST_CONTROL", &control)
            .env("KELD_TEST_OBSERVATION", &observation)
            .output()
            .expect("native context-fault executable");
        assert_eq!(child.status.code(), Some(2));
        assert!(child.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&child.stderr),
            "setup.keld:1:1: runtime[AllocationFault]: runtime allocation failed\n"
        );
    }
}

#[test]
fn test_runtime_allocation_controls_report_faults_at_the_current_instruction() {
    let metadata = SourceMetadata {
        path: PathBuf::from("allocation.keld"),
        source: SourceText::from_str(SourceId(0), "allocation\n").expect("source"),
    };
    let cases: &[AllocationCase] = &[
        ("text", "site_id=1 phase=text attempt=1\n", text_module),
        ("copy", "site_id=1 phase=copy attempt=1\n", text_module),
        ("concat", "site_id=1 phase=concat attempt=1\n", text_module),
        (
            "struct",
            "site_id=1 phase=struct attempt=1\n",
            struct_module,
        ),
        (
            "entity",
            "site_id=1 phase=entity attempt=1\n",
            entity_module,
        ),
        (
            "list-growth",
            "site_id=1 phase=list_growth_preferred attempt=1\nsite_id=1 phase=list_growth_exact attempt=1\n",
            list_module,
        ),
    ];
    for optimization in [OptimizationLevel::O0, OptimizationLevel::O2] {
        for (tag, controls, builder) in cases {
            let output = run_test_runtime(
                &format!("allocation-{tag}-{optimization:?}"),
                &builder(),
                &metadata,
                controls,
                optimization,
            );
            assert_eq!(output.status.code(), Some(2), "{tag} {optimization:?}");
            assert!(output.stdout.is_empty(), "{tag} {optimization:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stderr),
                "allocation.keld:1:1: runtime[AllocationFault]: runtime allocation failed\n",
                "{tag} {optimization:?}"
            );
        }
    }
}

#[test]
fn frozen_site_schedule_controls_native_at_the_same_site_as_interpreter() {
    let module = text_module();
    let schedule = AllocationSchedule::from_module(&module);
    let function = module
        .functions
        .iter()
        .find(|function| function.id == module.main)
        .expect("main function");
    let base = schedule
        .base_id(function.id, function.entry, 0)
        .expect("entry allocation coordinate");
    let site = schedule.site_id(base, AllocationPhase::Text, 0);
    let controls = format!(
        "site_id={site} phase={} attempt=1\n",
        AllocationPhase::Text.as_str()
    );
    let metadata = SourceMetadata {
        path: PathBuf::from("frozen-sites.keld"),
        source: SourceText::from_str(SourceId(0), "frozen sites\n").expect("source"),
    };
    for optimization in [OptimizationLevel::O0, OptimizationLevel::O2] {
        let output = run_test_runtime(
            &format!("frozen-site-{optimization:?}"),
            &module,
            &metadata,
            &controls,
            optimization,
        );
        assert_eq!(output.status.code(), Some(2), "{optimization:?}");
        assert!(output.stdout.is_empty(), "{optimization:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "frozen-sites.keld:1:1: runtime[AllocationFault]: runtime allocation failed\n",
            "{optimization:?}"
        );
    }
}

#[test]
fn test_runtime_list_preferred_growth_failure_retries_exact_capacity() {
    let metadata = SourceMetadata {
        path: PathBuf::from("list-growth.keld"),
        source: SourceText::from_str(SourceId(0), "list growth\n").expect("source"),
    };
    for optimization in [OptimizationLevel::O0, OptimizationLevel::O2] {
        let output = run_test_runtime(
            &format!("list-growth-fallback-{optimization:?}"),
            &list_module(),
            &metadata,
            "site_id=1 phase=list_growth_preferred attempt=1\n",
            optimization,
        );
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, b"7\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn lowers_scalar_cfg_and_phi_at_both_optimization_levels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let ffi_dll = target.join("keld_runtime_v1.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = native_temp("scalar");
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\nkeld_rt_v1_context_new\nkeld_rt_v1_context_destroy\nkeld_rt_v1_context_status\nkeld_rt_v1_context_fault\nkeld_rt_v1_context_fault_parts\nkeld_rt_v1_context_root_lifecycle\nkeld_rt_v1_value_copy\nkeld_rt_v1_value_drop\nkeld_rt_v1_text_new\nkeld_rt_v1_text_byte_length\nkeld_rt_v1_text_is_empty\nkeld_rt_v1_text_equal\nkeld_rt_v1_text_concat\nkeld_rt_v1_list_new\nkeld_rt_v1_list_length\nkeld_rt_v1_list_push\nkeld_rt_v1_list_get\nkeld_rt_v1_list_remove\nkeld_rt_v1_list_replace\nkeld_rt_v1_list_try_remove\nkeld_rt_v1_list_clear\nkeld_rt_v1_list_reserve\nkeld_rt_v1_list_try_reserve\n",
    )
    .expect("runtime export list");
    add_context_setup_export(&export_list);
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(dlltool)
        .args([
            "--input-def",
            export_list.to_str().expect("export list path"),
            "--dllname",
            "keld_runtime_v1.dll",
            "--output-lib",
        ])
        .arg(&import_library)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "dlltool failed: {status}");

    let source = SourceText::from_str(SourceId(0), "branch\n").expect("source");
    let metadata = SourceMetadata {
        path: PathBuf::from("task4.keld"),
        source,
    };
    let module = scalar_module(
        vec![
            IrType::Lifecycle,
            IrType::Bool,
            IrType::Int,
            IrType::Int,
            IrType::Int,
        ],
        vec![
            block(
                0,
                vec![Instruction::ConstBool {
                    dst: Register(1),
                    value: true,
                    span: span(),
                }],
                Terminator::Branch {
                    condition: Register(1),
                    then_block: IrBlockId(1),
                    else_block: IrBlockId(2),
                },
            ),
            block(
                1,
                vec![Instruction::ConstInt {
                    dst: Register(2),
                    value: 7,
                    span: span(),
                }],
                Terminator::Goto(IrBlockId(3)),
            ),
            block(
                2,
                vec![Instruction::ConstInt {
                    dst: Register(3),
                    value: 9,
                    span: span(),
                }],
                Terminator::Goto(IrBlockId(3)),
            ),
            block(
                3,
                vec![Instruction::Phi {
                    dst: Register(4),
                    inputs: vec![(IrBlockId(1), Register(2)), (IrBlockId(2), Register(3))],
                    span: span(),
                }],
                Terminator::Return(Some(Register(4))),
            ),
        ],
    );
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = temp.join(format!("scalar-{index}.exe"));
        let artifact = build_executable(
            &module,
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("native scalar build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("native scalar executable");
        assert!(child.status.success(), "native status: {}", child.status);
        assert_eq!(child.stdout, b"7\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn reverse_order_phi_after_checked_predecessor_runs_at_both_optimization_levels() {
    let (runtime_dll, import_library) = runtime_artifacts("reverse-phi");
    let metadata = SourceMetadata {
        path: PathBuf::from("reverse-phi.keld"),
        source: SourceText::from_str(SourceId(0), "reverse phi\n").expect("source"),
    };
    let module = scalar_module(
        vec![
            IrType::Lifecycle,
            IrType::Bool,
            IrType::Int,
            IrType::Int,
            IrType::Int,
        ],
        vec![
            block(
                0,
                vec![
                    Instruction::ConstBool {
                        dst: Register(1),
                        value: true,
                        span: span(),
                    },
                    Instruction::ConstInt {
                        dst: Register(2),
                        value: 4,
                        span: span(),
                    },
                ],
                Terminator::Branch {
                    condition: Register(1),
                    then_block: IrBlockId(1),
                    else_block: IrBlockId(2),
                },
            ),
            block(
                1,
                vec![Instruction::Phi {
                    dst: Register(4),
                    inputs: vec![(IrBlockId(0), Register(2)), (IrBlockId(2), Register(3))],
                    span: span(),
                }],
                Terminator::Return(Some(Register(4))),
            ),
            block(
                2,
                vec![Instruction::CheckedBinaryInt {
                    dst: Register(3),
                    op: IntBinaryOp::Add,
                    lhs: Register(2),
                    rhs: Register(2),
                    span: span(),
                }],
                Terminator::Goto(IrBlockId(1)),
            ),
        ],
    );
    let directory = runtime_dll.parent().expect("runtime directory");
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = directory.join(format!("reverse-phi-{index}.exe"));
        let artifact = build_executable(
            &module,
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("reverse-order Phi native build");
        let child = Command::new(&artifact.executable)
            .env("PATH", "C:\\Windows\\System32;C:\\Windows")
            .output()
            .expect("reverse-order Phi native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"4\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn invalid_runtime_dll_is_rejected_before_link() {
    let (runtime_dll, import_library) = runtime_artifacts("invalid-runtime");
    std::fs::write(&runtime_dll, b"not a PE and not ABI version 1").expect("invalid DLL");
    let output = runtime_dll
        .parent()
        .expect("runtime directory")
        .join("invalid-runtime.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("same-named non-PE runtime must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(error.to_string().contains("x86-64 PE"));
    assert!(!output.exists());
}

#[test]
fn import_library_with_wrong_dll_name_hidden_by_benign_member_is_rejected() {
    let (runtime_dll, import_library) = runtime_artifacts("wrong-import");
    let directory = runtime_dll.parent().expect("runtime directory");
    let definition = directory.join("runtime.def");
    let contents = std::fs::read_to_string(&definition)
        .expect("runtime export definition")
        .replace("LIBRARY keld_runtime_v1.dll", "LIBRARY other_runtime.dll");
    std::fs::write(&definition, contents).expect("wrong runtime export definition");
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(dlltool)
        .args([
            "--input-def",
            definition.to_str().expect("definition path"),
            "--dllname",
            "other_runtime.dll",
            "--output-lib",
        ])
        .arg(&import_library)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "dlltool failed: {status}");
    let benign_member = directory.join("benign.o");
    std::fs::write(
        &benign_member,
        b"benign metadata keld_runtime_v1.dll keld_rt_v1_abi_version",
    )
    .expect("benign archive member");
    let ar = std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into());
    let status = Command::new(ar)
        .args(["r", import_library.to_str().expect("import library path")])
        .arg(&benign_member)
        .status()
        .expect("ar");
    assert!(status.success(), "ar failed: {status}");
    let archive = std::fs::read(&import_library).expect("import archive");
    assert!(
        String::from_utf8_lossy(&archive).contains("keld_runtime_v1.dll")
            && String::from_utf8_lossy(&archive).contains("keld_rt_v1_abi_version"),
        "the benign member must contain both legacy validation strings"
    );
    let output = directory.join("wrong-import.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("wrong import-library DLL must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(error.to_string().contains("keld_runtime_v1.dll"));
    assert!(!output.exists());
}

#[test]
fn mixed_runtime_import_heads_are_rejected() {
    let (runtime_dll, import_library) = runtime_artifacts("mixed-import-heads");
    let directory = runtime_dll.parent().expect("runtime directory");
    let definition = directory.join("runtime.def");
    let contents = std::fs::read_to_string(&definition).expect("runtime export definition");
    let other_definition = directory.join("other-runtime.def");
    let other_contents = contents
        .replace("LIBRARY keld_runtime_v1.dll", "LIBRARY other_runtime.dll")
        .lines()
        .filter(|line| *line != "keld_rt_v1_abi_version")
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&other_definition, other_contents).expect("other runtime export definition");
    let other_import = directory.join("libother_runtime.dll.a");
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(&dlltool)
        .args([
            "--input-def",
            other_definition.to_str().expect("other definition path"),
            "--dllname",
            "other_runtime.dll",
            "--output-lib",
        ])
        .arg(&other_import)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "other dlltool failed: {status}");

    let extracted = directory.join("other-members");
    std::fs::create_dir_all(&extracted).expect("other archive extraction directory");
    let status = Command::new(std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into()))
        .current_dir(&extracted)
        .args(["x", other_import.to_str().expect("other import path")])
        .status()
        .expect("ar extract");
    assert!(status.success(), "ar extract failed: {status}");
    let archive_members = Command::new(std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into()))
        .args(["t", other_import.to_str().expect("other import path")])
        .output()
        .expect("ar list");
    assert!(archive_members.status.success(), "ar list failed");
    let remaining_imports = String::from_utf8(archive_members.stdout)
        .expect("archive member names")
        .lines()
        .filter(|member| !member.ends_with("_h.o") && !member.ends_with("_t.o"))
        .map(|member| extracted.join(member))
        .collect::<Vec<_>>();
    assert!(
        remaining_imports.len() >= 2,
        "other archive must provide remaining runtime imports"
    );

    let ar = std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into());
    let mut append = Command::new(ar);
    append
        .current_dir(directory)
        .args(["r", import_library.to_str().expect("import library path")]);
    for member in &remaining_imports {
        append.arg(member);
    }
    let status = append.status().expect("ar append");
    assert!(status.success(), "ar append failed: {status}");

    let output = directory.join("mixed-import-heads.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("mixed runtime import heads must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(
        error
            .to_string()
            .contains("runtime import library must contain a COFF import")
    );
    assert!(!output.exists());
}

#[test]
fn mixed_runtime_short_imports_are_rejected() {
    let (runtime_dll, import_library) = runtime_artifacts("mixed-short-imports");
    let directory = runtime_dll.parent().expect("runtime directory");
    let definition = directory.join("runtime.def");
    let contents = std::fs::read_to_string(&definition).expect("runtime export definition");
    let other_definition = directory.join("other-runtime.def");
    let other_contents = contents
        .replace("LIBRARY keld_runtime_v1.dll", "LIBRARY other_runtime.dll")
        .lines()
        .filter(|line| *line != "keld_rt_v1_abi_version")
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&other_definition, other_contents).expect("other runtime export definition");
    let other_import = directory.join("libother_runtime.dll.a");
    let llvm_prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(".tools/llvm/22.1.8-mingw64"))
        })
        .expect("LLVM prefix");
    let status = Command::new(llvm_prefix.join("bin/llvm-dlltool.exe"))
        .args([
            "-m",
            "i386:x86-64",
            "-d",
            other_definition.to_str().expect("other definition path"),
            "-D",
            "other_runtime.dll",
            "-l",
        ])
        .arg(&other_import)
        .status()
        .expect("llvm-dlltool");
    assert!(status.success(), "llvm-dlltool failed: {status}");

    let ar = std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into());
    let extracted = directory.join("short-import-members");
    std::fs::create_dir_all(&extracted).expect("short import extraction directory");
    let mut short_imports = Vec::new();
    for occurrence in 4..=6 {
        let status = Command::new(&ar)
            .current_dir(&extracted)
            .arg("xN")
            .arg(occurrence.to_string())
            .arg(other_import.to_str().expect("other import path"))
            .arg("other_runtime.dll")
            .status()
            .expect("ar short import extract");
        assert!(status.success(), "ar short import extract failed: {status}");
        let extracted_member = extracted.join("other_runtime.dll");
        let renamed = extracted.join(format!("short-{occurrence}.o"));
        std::fs::rename(extracted_member, &renamed).expect("rename short import member");
        short_imports.push(renamed);
    }

    let mut append = Command::new(llvm_prefix.join("bin/llvm-ar.exe"));
    append
        .current_dir(directory)
        .args(["qS", import_library.to_str().expect("import library path")]);
    for member in &short_imports {
        append.arg(member);
    }
    let status = append.status().expect("ar short import append");
    assert!(status.success(), "ar short import append failed: {status}");

    let output = directory.join("mixed-short-imports.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("mixed runtime short imports must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(
        error
            .to_string()
            .contains("runtime import library must contain a COFF import")
    );
    assert!(!output.exists());
}

#[test]
fn ordinary_coff_runtime_symbol_override_is_rejected() {
    let (runtime_dll, import_library) = runtime_artifacts("ordinary-coff-override");
    let directory = runtime_dll.parent().expect("runtime directory");
    let llvm_prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(".tools/llvm/22.1.8-mingw64"))
        })
        .expect("LLVM prefix");
    let assembly = directory.join("runtime-override.s");
    std::fs::write(
        &assembly,
        ".text\n.globl keld_rt_v1_print_int\nkeld_rt_v1_print_int:\n  ret\n",
    )
    .expect("runtime override assembly");
    let object = directory.join("runtime-override.o");
    let status = Command::new(llvm_prefix.join("bin/llvm-mc.exe"))
        .arg("-triple=x86_64-w64-windows-gnu")
        .arg("-filetype=obj")
        .arg("-o")
        .arg(&object)
        .arg(&assembly)
        .status()
        .expect("llvm-mc");
    assert!(status.success(), "llvm-mc failed: {status}");

    let ar = std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into());
    let listing = Command::new(&ar)
        .args(["t", import_library.to_str().expect("import library path")])
        .output()
        .expect("ar list");
    assert!(listing.status.success(), "ar list failed");
    let first_member = String::from_utf8(listing.stdout)
        .expect("archive member names")
        .lines()
        .next()
        .expect("runtime import archive member")
        .to_owned();
    let object_name = object
        .file_name()
        .and_then(|name| name.to_str())
        .expect("override object name");
    let status = Command::new(&ar)
        .current_dir(directory)
        .arg("r")
        .arg(&import_library)
        .arg(object_name)
        .status()
        .expect("ar append");
    assert!(status.success(), "ar append failed: {status}");
    let status = Command::new(&ar)
        .current_dir(directory)
        .arg("mb")
        .arg(&first_member)
        .arg(&import_library)
        .arg(object_name)
        .status()
        .expect("ar move");
    assert!(status.success(), "ar move failed: {status}");

    let output = directory.join("ordinary-coff-override.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("ordinary COFF runtime symbol override must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(
        error
            .to_string()
            .contains("runtime import library must contain a COFF import")
    );
    assert!(!output.exists());
}

#[test]
fn native_program_rejects_runtime_abi_version_mismatch() {
    let (runtime_dll, import_library) = test_runtime_artifacts("abi-mismatch");
    let directory = runtime_dll.parent().expect("runtime directory");
    let metadata = SourceMetadata {
        path: PathBuf::from("abi-mismatch.keld"),
        source: SourceText::from_str(SourceId(0), "abi mismatch\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = directory.join(format!("abi-mismatch-{index}.exe"));
        let observation = directory.join(format!("abi-mismatch-{index}.observation"));
        let artifact = build_executable(
            &const_module(7),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("ABI mismatch native build");
        let child = Command::new(&artifact.executable)
            .env("KELD_TEST_ABI_VERSION", "2")
            .env("KELD_TEST_OBSERVATION", &observation)
            .output()
            .expect("ABI mismatch native executable");
        assert_eq!(child.status.code(), Some(70));
        assert!(child.stdout.is_empty());
        assert!(child.stderr.is_empty());
        assert!(
            !observation.exists(),
            "ABI mismatch must precede context setup"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn scalar_faults_preserve_kind_span_and_unreachable_is_internal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let ffi_dll = target.join("keld_runtime_v1.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = native_temp("fault");
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\nkeld_rt_v1_context_new\nkeld_rt_v1_context_destroy\nkeld_rt_v1_context_status\nkeld_rt_v1_context_fault\nkeld_rt_v1_context_fault_parts\nkeld_rt_v1_context_root_lifecycle\nkeld_rt_v1_value_copy\nkeld_rt_v1_value_drop\nkeld_rt_v1_text_new\nkeld_rt_v1_text_byte_length\nkeld_rt_v1_text_is_empty\nkeld_rt_v1_text_equal\nkeld_rt_v1_text_concat\nkeld_rt_v1_list_new\nkeld_rt_v1_list_length\nkeld_rt_v1_list_push\nkeld_rt_v1_list_get\nkeld_rt_v1_list_remove\nkeld_rt_v1_list_replace\nkeld_rt_v1_list_try_remove\nkeld_rt_v1_list_clear\nkeld_rt_v1_list_reserve\nkeld_rt_v1_list_try_reserve\n",
    )
    .expect("runtime export list");
    add_context_setup_export(&export_list);
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let status = Command::new(dlltool)
        .args([
            "--input-def",
            export_list.to_str().expect("export list path"),
            "--dllname",
            "keld_runtime_v1.dll",
            "--output-lib",
        ])
        .arg(&import_library)
        .arg(&runtime_dll)
        .status()
        .expect("dlltool");
    assert!(status.success(), "dlltool failed: {status}");

    let source = SourceText::from_str(SourceId(0), "x\ny\n").expect("source");
    let metadata = SourceMetadata {
        path: PathBuf::from("fault.keld"),
        source,
    };
    let fault_span = Span::new(SourceId(0), 2, 3).expect("fault span");
    let overflow = scalar_module(
        vec![IrType::Lifecycle, IrType::Int, IrType::Int, IrType::Int],
        vec![block(
            0,
            vec![
                Instruction::ConstInt {
                    dst: Register(1),
                    value: i64::MAX,
                    span: fault_span,
                },
                Instruction::ConstInt {
                    dst: Register(2),
                    value: 1,
                    span: fault_span,
                },
                Instruction::CheckedBinaryInt {
                    dst: Register(3),
                    op: IntBinaryOp::Add,
                    lhs: Register(1),
                    rhs: Register(2),
                    span: fault_span,
                },
            ],
            Terminator::Return(Some(Register(3))),
        )],
    );
    let unreachable = scalar_module(
        vec![IrType::Lifecycle],
        vec![block(0, Vec::new(), Terminator::Unreachable)],
    );
    let cases = [
        (
            overflow,
            "fault.keld:2:1: runtime[ArithmeticFault]: checked integer arithmetic overflow\n",
            2,
        ),
        (unreachable, "", 70),
    ];
    for (optimization_index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        for (index, (module, expected_stderr, expected_status)) in cases.iter().enumerate() {
            let output = temp.join(format!("fault-{optimization_index}-{index}.exe"));
            let artifact = build_executable(
                module,
                &metadata,
                &request(&output, &runtime_dll, &import_library, optimization),
            )
            .expect("native fault build");
            let child = Command::new(&artifact.executable)
                .output()
                .expect("native fault executable");
            assert_eq!(child.status.code(), Some(*expected_status));
            assert_eq!(String::from_utf8_lossy(&child.stderr), *expected_stderr);
            assert!(child.stdout.is_empty());
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn scalar_numeric_operations_match_checked_interpreter_boundaries() {
    let (runtime_dll, import_library) = runtime_artifacts("numeric");
    let metadata = SourceMetadata {
        path: PathBuf::from("numeric.keld"),
        source: SourceText::from_str(SourceId(0), "numeric\n").expect("source"),
    };
    let successful = [
        ("add", binary_module(7, IntBinaryOp::Add, 5), "12\n"),
        ("sub", binary_module(7, IntBinaryOp::Sub, 5), "2\n"),
        ("mul", binary_module(7, IntBinaryOp::Mul, 5), "35\n"),
        ("div", binary_module(7, IntBinaryOp::Div, 5), "1\n"),
        ("rem", binary_module(7, IntBinaryOp::Rem, 5), "2\n"),
        ("shl", binary_module(1, IntBinaryOp::Shl, 3), "8\n"),
        ("shr", binary_module(-8, IntBinaryOp::Shr, 2), "-2\n"),
        ("neg", unary_module(7), "-7\n"),
    ];
    for (index, (name, module, expected)) in successful.into_iter().enumerate() {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("{name}-{index}.exe"));
        let artifact = build_executable(
            &module,
            &metadata,
            &request(
                &output,
                &runtime_dll,
                &import_library,
                if index % 2 == 0 {
                    OptimizationLevel::O0
                } else {
                    OptimizationLevel::O2
                },
            ),
        )
        .expect("numeric native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("numeric native executable");
        assert!(child.status.success(), "native status: {}", child.status);
        assert_eq!(child.stdout, expected.as_bytes());
        assert!(child.stderr.is_empty());
    }

    let comparisons = [
        (keld_ir::CompareOp::Eq, 1),
        (keld_ir::CompareOp::NotEq, 0),
        (keld_ir::CompareOp::Less, 0),
        (keld_ir::CompareOp::LessEq, 1),
        (keld_ir::CompareOp::Greater, 0),
        (keld_ir::CompareOp::GreaterEq, 1),
    ];
    for (index, (op, expected)) in comparisons.into_iter().enumerate() {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("compare-{index}.exe"));
        let artifact = build_executable(
            &comparison_module(3, op, 3),
            &metadata,
            &request(
                &output,
                &runtime_dll,
                &import_library,
                OptimizationLevel::O2,
            ),
        )
        .expect("comparison native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("comparison native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, format!("{expected}\n").as_bytes());
    }

    let fault_cases = [
        (
            "div-zero",
            binary_module(7, IntBinaryOp::Div, 0),
            "DivisionByZeroFault",
        ),
        (
            "rem-zero",
            binary_module(7, IntBinaryOp::Rem, 0),
            "DivisionByZeroFault",
        ),
        (
            "div-overflow",
            binary_module(i64::MIN, IntBinaryOp::Div, -1),
            "ArithmeticFault",
        ),
        (
            "shl-overflow",
            binary_module(1, IntBinaryOp::Shl, 63),
            "ArithmeticFault",
        ),
        (
            "shl-invalid",
            binary_module(1, IntBinaryOp::Shl, 64),
            "ShiftFault",
        ),
        (
            "shr-invalid",
            binary_module(1, IntBinaryOp::Shr, -1),
            "ShiftFault",
        ),
        ("neg-overflow", unary_module(i64::MIN), "ArithmeticFault"),
    ];
    for (index, (name, module, expected_kind)) in fault_cases.into_iter().enumerate() {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("{name}-{index}.exe"));
        let artifact = build_executable(
            &module,
            &metadata,
            &request(
                &output,
                &runtime_dll,
                &import_library,
                OptimizationLevel::O0,
            ),
        )
        .expect("fault native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("fault native executable");
        assert_eq!(child.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&child.stderr).contains(expected_kind));
        assert!(child.stdout.is_empty());
    }
}

#[test]
fn text_literals_copy_concat_and_queries_run_natively() {
    let (runtime_dll, import_library) = runtime_artifacts("text");
    let metadata = SourceMetadata {
        path: PathBuf::from("text.keld"),
        source: SourceText::from_str(SourceId(0), "text\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("text-{index}.exe"));
        let artifact = build_executable(
            &text_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("text native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("text native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"4\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn direct_list_operations_run_natively() {
    let (runtime_dll, import_library) = runtime_artifacts("list");
    let metadata = SourceMetadata {
        path: PathBuf::from("list.keld"),
        source: SourceText::from_str(SourceId(0), "list\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("list-{index}.exe"));
        let artifact = build_executable(
            &list_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("list native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("list native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"7\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn projected_list_receiver_runs_natively_at_both_optimization_levels() {
    let (runtime_dll, import_library) = runtime_artifacts("projected-list");
    let metadata = SourceMetadata {
        path: PathBuf::from("projected-list.keld"),
        source: SourceText::from_str(SourceId(0), "projected list\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("projected-list-{index}.exe"));
        let artifact = build_executable(
            &projected_list_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("projected list native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("projected list native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"9\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn managed_phi_transfers_the_selected_envelope_at_both_optimization_levels() {
    let (runtime_dll, import_library) = runtime_artifacts("managed-phi");
    let metadata = SourceMetadata {
        path: PathBuf::from("managed-phi.keld"),
        source: SourceText::from_str(SourceId(0), "managed phi\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("managed-phi-{index}.exe"));
        let artifact = build_executable(
            &managed_phi_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("managed Phi native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("managed Phi native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"4\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn struct_construction_and_managed_field_copy_run_natively() {
    let (runtime_dll, import_library) = runtime_artifacts("struct");
    let metadata = SourceMetadata {
        path: PathBuf::from("struct.keld"),
        source: SourceText::from_str(SourceId(0), "struct\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("struct-{index}.exe"));
        let artifact = build_executable(
            &struct_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("struct native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("struct native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"11\n");
        assert!(child.stderr.is_empty());
    }
}

#[test]
fn entity_allocation_links_views_and_retirement_run_natively() {
    let (runtime_dll, import_library) = runtime_artifacts("entity");
    let metadata = SourceMetadata {
        path: PathBuf::from("entity.keld"),
        source: SourceText::from_str(SourceId(0), "entity\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("entity-{index}.exe"));
        let artifact = build_executable(
            &entity_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("entity native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("entity native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"6\n");
        assert!(child.stderr.is_empty());
    }
}

fn identity_and_main_module() -> Module {
    let source_span = span();
    let identity = Function {
        id: keld_semantics::FunctionId(0),
        span: source_span,
        parameters: vec![Register(1)],
        locals: Vec::new(),
        parameter_modes: vec![keld_semantics::ParameterMode::Loan],
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Int],
        register_storage: vec![RegisterStorage::Trivial, RegisterStorage::Trivial],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(0, Vec::new(), Terminator::Return(Some(Register(1))))],
        entry: IrBlockId(0),
    };
    let main = Function {
        id: keld_semantics::FunctionId(1),
        span: source_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Int, IrType::Int],
        register_storage: vec![RegisterStorage::Trivial; 3],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            vec![
                Instruction::ConstInt {
                    dst: Register(1),
                    value: 41,
                    span: source_span,
                },
                Instruction::Call {
                    dst: Some(Register(2)),
                    function: keld_semantics::FunctionId(0),
                    arguments: vec![(keld_ir::ParameterIndex(0), Register(1))],
                    argument_sources: vec![(
                        keld_ir::ParameterIndex(0),
                        Some(keld_ir::ArgumentSource {
                            base: Register(1),
                            projections: Vec::new(),
                        }),
                    )],
                    current_lifecycle: Register(0),
                    span: source_span,
                },
            ],
            Terminator::Return(Some(Register(2))),
        )],
        entry: IrBlockId(0),
    };
    Module {
        definitions: Vec::new(),
        functions: vec![identity, main],
        main: keld_semantics::FunctionId(1),
    }
}

fn unit_call_module() -> Module {
    let source_span = span();
    let unit = Function {
        id: keld_semantics::FunctionId(0),
        span: source_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle],
        register_storage: vec![RegisterStorage::Trivial],
        storage_scope_parents: vec![None],
        return_type: IrType::Unit,
        blocks: vec![block(0, Vec::new(), Terminator::Return(None))],
        entry: IrBlockId(0),
    };
    let main = Function {
        id: keld_semantics::FunctionId(1),
        span: source_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Int],
        register_storage: vec![RegisterStorage::Trivial; 2],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            vec![
                Instruction::Call {
                    dst: None,
                    function: keld_semantics::FunctionId(0),
                    arguments: Vec::new(),
                    argument_sources: Vec::new(),
                    current_lifecycle: Register(0),
                    span: source_span,
                },
                Instruction::ConstInt {
                    dst: Register(1),
                    value: 9,
                    span: source_span,
                },
            ],
            Terminator::Return(Some(Register(1))),
        )],
        entry: IrBlockId(0),
    };
    Module {
        definitions: Vec::new(),
        functions: vec![unit, main],
        main: keld_semantics::FunctionId(1),
    }
}

fn managed_call_module(mode: keld_semantics::ParameterMode) -> Module {
    let source_span = span();
    let managed_type = IrType::Text;
    let mut callee_instructions = vec![Instruction::TextByteLength {
        dst: Register(2),
        text: Register(1),
        span: source_span,
    }];
    if mode == keld_semantics::ParameterMode::Take {
        callee_instructions.push(Instruction::DropIfLive {
            home: Register(1),
            span: source_span,
        });
    }
    let callee = Function {
        id: keld_semantics::FunctionId(0),
        span: source_span,
        parameters: vec![Register(1)],
        locals: Vec::new(),
        parameter_modes: vec![mode],
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, managed_type.clone(), IrType::Int],
        register_storage: vec![
            RegisterStorage::Trivial,
            if mode == keld_semantics::ParameterMode::Take {
                RegisterStorage::Home {
                    scope: keld_flow::StorageScopeId(0),
                    conditional: false,
                }
            } else {
                RegisterStorage::Loan
            },
            RegisterStorage::Trivial,
        ],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            callee_instructions,
            Terminator::Return(Some(Register(2))),
        )],
        entry: IrBlockId(0),
    };
    let mut main_storage = vec![RegisterStorage::Trivial; 3];
    main_storage[1] = RegisterStorage::Home {
        scope: keld_flow::StorageScopeId(0),
        conditional: false,
    };
    let main = Function {
        id: keld_semantics::FunctionId(1),
        span: source_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, managed_type, IrType::Int],
        register_storage: main_storage,
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            vec![
                Instruction::ConstText {
                    dst: Register(1),
                    value: "managed call".to_owned(),
                    span: source_span,
                },
                Instruction::Call {
                    dst: Some(Register(2)),
                    function: keld_semantics::FunctionId(0),
                    arguments: vec![(keld_ir::ParameterIndex(0), Register(1))],
                    argument_sources: vec![(
                        keld_ir::ParameterIndex(0),
                        Some(keld_ir::ArgumentSource {
                            base: Register(1),
                            projections: Vec::new(),
                        }),
                    )],
                    current_lifecycle: Register(0),
                    span: source_span,
                },
                Instruction::CleanupTrackedScope {
                    scope: keld_flow::StorageScopeId(0),
                    span: source_span,
                },
            ],
            Terminator::Return(Some(Register(2))),
        )],
        entry: IrBlockId(0),
    };
    Module {
        definitions: Vec::new(),
        functions: vec![callee, main],
        main: keld_semantics::FunctionId(1),
    }
}

fn faulting_callee_module() -> Module {
    let function_span = Span::new(SourceId(0), 0, 1).expect("function span");
    let fault_span = Span::new(SourceId(0), 2, 3).expect("fault span");
    let callee = Function {
        id: keld_semantics::FunctionId(0),
        span: function_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Int, IrType::Int, IrType::Int],
        register_storage: vec![RegisterStorage::Trivial; 4],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            vec![
                Instruction::ConstInt {
                    dst: Register(1),
                    value: i64::MAX,
                    span: fault_span,
                },
                Instruction::ConstInt {
                    dst: Register(2),
                    value: 1,
                    span: fault_span,
                },
                Instruction::CheckedBinaryInt {
                    dst: Register(3),
                    op: IntBinaryOp::Add,
                    lhs: Register(1),
                    rhs: Register(2),
                    span: fault_span,
                },
            ],
            Terminator::Return(Some(Register(3))),
        )],
        entry: IrBlockId(0),
    };
    let main = Function {
        id: keld_semantics::FunctionId(1),
        span: function_span,
        parameters: Vec::new(),
        locals: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: Register(0),
        register_types: vec![IrType::Lifecycle, IrType::Int],
        register_storage: vec![RegisterStorage::Trivial; 2],
        storage_scope_parents: vec![None],
        return_type: IrType::Int,
        blocks: vec![block(
            0,
            vec![Instruction::Call {
                dst: Some(Register(1)),
                function: keld_semantics::FunctionId(0),
                arguments: Vec::new(),
                argument_sources: Vec::new(),
                current_lifecycle: Register(0),
                span: function_span,
            }],
            Terminator::Return(Some(Register(1))),
        )],
        entry: IrBlockId(0),
    };
    Module {
        definitions: Vec::new(),
        functions: vec![callee, main],
        main: keld_semantics::FunctionId(1),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn scalar_calls_cover_owned_results_unit_calls_and_callee_faults() {
    let (runtime_dll, import_library) = runtime_artifacts("calls");
    let metadata = SourceMetadata {
        path: PathBuf::from("calls.keld"),
        source: SourceText::from_str(SourceId(0), "call\n").expect("source"),
    };
    for (index, (module, expected_stdout, expected_status, expected_stderr)) in [
        (identity_and_main_module(), "41\n", 0, ""),
        (unit_call_module(), "9\n", 0, ""),
    ]
    .into_iter()
    .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("call-{index}.exe"));
        let artifact = build_executable(
            &module,
            &metadata,
            &request(
                &output,
                &runtime_dll,
                &import_library,
                if index == 0 {
                    OptimizationLevel::O0
                } else {
                    OptimizationLevel::O2
                },
            ),
        )
        .expect("call native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("call native executable");
        assert_eq!(child.status.code(), Some(expected_status));
        assert_eq!(child.stdout, expected_stdout.as_bytes());
        assert_eq!(String::from_utf8_lossy(&child.stderr), expected_stderr);
    }
}

#[test]
fn scalar_callee_fault_keeps_the_callee_location() {
    let (runtime_dll, import_library) = runtime_artifacts("call-fault");
    let metadata = SourceMetadata {
        path: PathBuf::from("call-fault.keld"),
        source: SourceText::from_str(SourceId(0), "a\nb\n").expect("source"),
    };
    for (index, optimization) in [OptimizationLevel::O0, OptimizationLevel::O2]
        .into_iter()
        .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("callee-fault-{index}.exe"));
        let artifact = build_executable(
            &faulting_callee_module(),
            &metadata,
            &request(&output, &runtime_dll, &import_library, optimization),
        )
        .expect("callee fault native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("callee fault native executable");
        assert_eq!(child.status.code(), Some(2));
        assert_eq!(
            String::from_utf8_lossy(&child.stderr),
            "call-fault.keld:2:1: runtime[ArithmeticFault]: checked integer arithmetic overflow\n"
        );
    }
}

#[test]
fn managed_call_modes_transfer_and_mutate_the_encoded_argument() {
    let (runtime_dll, import_library) = runtime_artifacts("managed-calls");
    let metadata = SourceMetadata {
        path: PathBuf::from("managed-calls.keld"),
        source: SourceText::from_str(SourceId(0), "managed calls\n").expect("source"),
    };
    for (index, mode) in [
        keld_semantics::ParameterMode::Loan,
        keld_semantics::ParameterMode::Take,
    ]
    .into_iter()
    .enumerate()
    {
        let output = runtime_dll
            .parent()
            .expect("runtime directory")
            .join(format!("managed-call-{index}.exe"));
        let artifact = build_executable(
            &managed_call_module(mode),
            &metadata,
            &request(
                &output,
                &runtime_dll,
                &import_library,
                OptimizationLevel::O2,
            ),
        )
        .expect("managed call native build");
        let child = Command::new(&artifact.executable)
            .output()
            .expect("managed call native executable");
        assert_eq!(child.status.code(), Some(0));
        assert_eq!(child.stdout, b"12\n");
        assert!(child.stderr.is_empty());
    }
}
