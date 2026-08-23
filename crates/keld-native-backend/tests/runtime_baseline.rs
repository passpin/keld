use keld_interpreter::{Interpreter, ValueKind};
use keld_ir::Module;
use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};
use keld_source::{SourceId, SourceText};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
const SAMPLES: usize = 5;

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
    assert!(diagnostics.is_empty(), "invalid benchmark IR: {diagnostics:#?}");
    (module, source_text)
}

fn temp_dir(label: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "keld-runtime-bench-{label}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("benchmark temporary directory");
    path
}

fn runtime_artifacts(directory: &Path) -> (PathBuf, PathBuf) {
    let root = workspace_root();
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let source_dll = target.join("keld_runtime_v1.dll");
    let import = target.join("libkeld_runtime_v1.dll.a");
    assert!(source_dll.is_file(), "missing {}", source_dll.display());
    assert!(import.is_file(), "missing {}", import.display());
    let dll = directory.join("keld_runtime_v1.dll");
    std::fs::copy(&source_dll, &dll).expect("runtime DLL copy");
    (dll, import)
}

fn build_native(
    label: &str,
    module: &Module,
    source: &SourceText,
    optimization: OptimizationLevel,
) -> (PathBuf, PathBuf, Duration) {
    let directory = temp_dir(&format!("{label}-{optimization:?}"));
    let (dll, import) = runtime_artifacts(&directory);
    let output = directory.join(format!("{label}-{optimization:?}.exe"));
    let llvm_prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .expect("LLVM_SYS_221_PREFIX");
    let request = BuildRequest {
        output: output.clone(),
        optimization,
        llvm_prefix,
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll: dll,
        runtime_import_library: import,
    };
    let metadata = SourceMetadata {
        path: PathBuf::from(format!("{label}.keld")),
        source: source.clone(),
    };
    let started = Instant::now();
    build_executable(module, &metadata, &request).expect("native benchmark build");
    let elapsed = started.elapsed();
    (output, directory, elapsed)
}

fn interpreter_once(module: &Module) -> (i64, Duration) {
    let mut interpreter = Interpreter::new(module).expect("interpreter setup");
    let started = Instant::now();
    let result = interpreter.run_main().expect("interpreter run");
    let elapsed = started.elapsed();
    let ValueKind::Int(value) = result.value.kind() else {
        panic!("benchmark main did not return Int");
    };
    (*value, elapsed)
}

fn native_once(executable: &Path) -> (i64, Duration) {
    let started = Instant::now();
    let output = Command::new(executable)
        .output()
        .expect("native benchmark executable");
    let elapsed = started.elapsed();
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    let text = String::from_utf8(output.stdout).expect("native stdout is UTF-8");
    let value = text.trim().parse::<i64>().expect("native stdout is Int");
    (value, elapsed)
}

fn median_ms(mut samples: Vec<Duration>) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1000.0
}

fn bench_case(label: &str, source: &str) {
    let (module, source_text) = compile_source(source);

    let (expected, _) = interpreter_once(&module);
    let mut interpreter_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (value, elapsed) = interpreter_once(&module);
        assert_eq!(value, expected);
        interpreter_samples.push(elapsed);
    }

    let (o0, o0_dir, o0_build) = build_native(label, &module, &source_text, OptimizationLevel::O0);
    let (o2, o2_dir, o2_build) = build_native(label, &module, &source_text, OptimizationLevel::O2);

    assert_eq!(native_once(&o0).0, expected);
    assert_eq!(native_once(&o2).0, expected);

    let mut o0_samples = Vec::with_capacity(SAMPLES);
    let mut o2_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (value, elapsed) = native_once(&o0);
        assert_eq!(value, expected);
        o0_samples.push(elapsed);
    }
    for _ in 0..SAMPLES {
        let (value, elapsed) = native_once(&o2);
        assert_eq!(value, expected);
        o2_samples.push(elapsed);
    }

    let interpreter_ms = median_ms(interpreter_samples);
    let o0_ms = median_ms(o0_samples);
    let o2_ms = median_ms(o2_samples);
    let o0_size = std::fs::metadata(&o0).expect("O0 executable metadata").len();
    let o2_size = std::fs::metadata(&o2).expect("O2 executable metadata").len();

    println!(
        "BENCH label={label} result={expected} samples={SAMPLES} interpreter_ms={interpreter_ms:.3} native_o0_ms={o0_ms:.3} native_o2_ms={o2_ms:.3} speedup_o0={:.2} speedup_o2={:.2}",
        interpreter_ms / o0_ms,
        interpreter_ms / o2_ms,
    );
    println!(
        "BUILD label={label} native_o0_ms={:.3} native_o2_ms={:.3} o0_bytes={o0_size} o2_bytes={o2_size}",
        o0_build.as_secs_f64() * 1000.0,
        o2_build.as_secs_f64() * 1000.0,
    );

    let _ = std::fs::remove_dir_all(o0_dir);
    let _ = std::fs::remove_dir_all(o2_dir);
}

#[test]
#[ignore = "manual runtime benchmark"]
fn runtime_baseline() {
    bench_case(
        "scalar_control",
        r#"
fn main() -> Int {
    var i = 0
    var x = 1
    var acc = 0
    while i < 5000000 {
        x = (x * 48271 + 1) % 2147483647
        i += 1
        if x % 7 == 0 {
            continue
        }
        acc = (acc + x % 97) % 1000000007
    }
    return acc
}
"#,
    );

    bench_case(
        "managed_text",
        r#"
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
"#,
    );
}
