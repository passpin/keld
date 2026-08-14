use keld_ir::{
    Function, Instruction, IrBlock, IrBlockId, IrType, Module, Register, RegisterStorage,
    Terminator,
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
        "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\n",
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
