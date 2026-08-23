use keld_ir::Module;
use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};
use keld_source::{SourceId, SourceText};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn compile_source(source: &str) -> (Module, SourceText) {
    let source_text = SourceText::from_str(SourceId(0), source).expect("source text");
    let flow = keld_flow::lower_text_for_test(source).expect("flow lowering");
    let verified = keld_lifecycle::verify(flow)
        .module
        .expect("lifecycle verification");
    let storage = keld_storage::verify(verified)
        .module
        .expect("storage verification");
    let module = keld_ir::lower(&storage);
    let diagnostics = keld_ir::validate(&module);
    assert!(diagnostics.is_empty(), "invalid regression IR: {diagnostics:#?}");
    (module, source_text)
}

fn temp_dir() -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "keld-loop-stack-regression-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("temporary directory");
    path
}

#[test]
#[ignore = "requires pinned Windows GNU native toolchain"]
fn managed_loop_does_not_grow_the_native_stack_per_iteration() {
    let source = r#"
fn main() -> Int {
    var i = 0
    var count = 0
    while i < 500000 {
        if ("abcdefghijklmnopqrstuvwxyz" + "!").is_empty {
            count += 1
        }
        i += 1
    }
    return i + count
}
"#;
    let (module, source_text) = compile_source(source);
    let directory = temp_dir();
    let root = workspace_root();
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let runtime_source = target.join("keld_runtime_v1.dll");
    let runtime_import = target.join("libkeld_runtime_v1.dll.a");
    assert!(runtime_source.is_file(), "missing {}", runtime_source.display());
    assert!(runtime_import.is_file(), "missing {}", runtime_import.display());
    let runtime_dll = directory.join("keld_runtime_v1.dll");
    std::fs::copy(&runtime_source, &runtime_dll).expect("runtime copy");
    let executable = directory.join("loop-stack.exe");
    let request = BuildRequest {
        output: executable.clone(),
        optimization: OptimizationLevel::O0,
        llvm_prefix: std::env::var_os("LLVM_SYS_221_PREFIX")
            .map(PathBuf::from)
            .expect("LLVM_SYS_221_PREFIX"),
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll,
        runtime_import_library: runtime_import,
    };
    let metadata = SourceMetadata {
        path: PathBuf::from("loop_stack_regression.keld"),
        source: source_text,
    };
    build_executable(&module, &metadata, &request).expect("native build");

    let output = Command::new(&executable).output().expect("native executable");
    assert_eq!(
        output.status.code(),
        Some(0),
        "native process failed: status={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "500000");

    let _ = std::fs::remove_dir_all(directory);
}
