use keld_ir::{
    Function, Instruction, IntBinaryOp, IrBlock, IrBlockId, IrType, Module, Register,
    RegisterStorage, Terminator,
};
use keld_native_backend::{
    BuildRequest, NativeArtifact, OptimizationLevel, SourceMetadata, build_executable,
};
use keld_source::{SourceId, SourceText, Span};
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn runtime_artifacts(tag: &str) -> (PathBuf, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let ffi_dll = root.join("target/x86_64-pc-windows-gnu/release/keld_native_ffi.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {}",
        ffi_dll.display()
    );
    let temp = std::env::temp_dir().join(format!("keld-native-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\n",
    )
    .expect("runtime export list");
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
    let ffi_dll = target.join("keld_native_ffi.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = std::env::temp_dir().join(format!("keld-native-int-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\n",
    )
    .expect("runtime export list");
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
    let ffi_dll = target.join("keld_native_ffi.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = std::env::temp_dir().join(format!("keld-native-scalar-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\n",
    )
    .expect("runtime export list");
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
#[allow(clippy::too_many_lines)]
fn scalar_faults_preserve_kind_span_and_unreachable_is_internal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let ffi_dll = target.join("keld_native_ffi.dll");
    assert!(
        ffi_dll.is_file(),
        "build keld-native-ffi first: {ffi_dll:?}"
    );
    let temp = std::env::temp_dir().join(format!("keld-native-fault-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("temporary directory");
    let runtime_dll = temp.join("keld_runtime_v1.dll");
    std::fs::copy(&ffi_dll, &runtime_dll).expect("runtime DLL copy");
    let import_library = temp.join("libkeld_runtime_v1.dll.a");
    let export_list = temp.join("runtime.def");
    std::fs::write(
        &export_list,
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\n",
    )
    .expect("runtime export list");
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
