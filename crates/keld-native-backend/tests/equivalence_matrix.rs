use keld_interpreter::{Interpreter, ValueKind};
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
    assert!(diagnostics.is_empty(), "invalid equivalence IR: {diagnostics:#?}");
    (module, source_text)
}

fn interpreter_result(module: &Module) -> i64 {
    let mut interpreter = Interpreter::new(module).expect("interpreter setup");
    let result = interpreter.run_main().expect("interpreter execution");
    let ValueKind::Int(value) = result.value.kind() else {
        panic!("equivalence main did not return Int");
    };
    *value
}

fn native_result(
    label: &str,
    module: &Module,
    source: &SourceText,
    optimization: OptimizationLevel,
) -> i64 {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "keld-equivalence-{label}-{optimization:?}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("temporary directory");

    let root = workspace_root();
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let runtime_source = target.join("keld_runtime_v1.dll");
    let runtime_import = target.join("libkeld_runtime_v1.dll.a");
    assert!(runtime_source.is_file(), "missing {}", runtime_source.display());
    assert!(runtime_import.is_file(), "missing {}", runtime_import.display());
    let runtime_dll = directory.join("keld_runtime_v1.dll");
    std::fs::copy(&runtime_source, &runtime_dll).expect("runtime DLL copy");

    let executable = directory.join(format!("{label}-{optimization:?}.exe"));
    let request = BuildRequest {
        output: executable.clone(),
        optimization,
        llvm_prefix: std::env::var_os("LLVM_SYS_221_PREFIX")
            .map(PathBuf::from)
            .expect("LLVM_SYS_221_PREFIX"),
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll,
        runtime_import_library: runtime_import,
    };
    let metadata = SourceMetadata {
        path: PathBuf::from(format!("{label}.keld")),
        source: source.clone(),
    };
    build_executable(module, &metadata, &request).expect("native build");
    let output = Command::new(&executable)
        .output()
        .expect("native executable");
    assert_eq!(
        output.status.code(),
        Some(0),
        "native {optimization:?} failed for {label}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("native stdout UTF-8");
    let result = text.trim().parse::<i64>().expect("native Int stdout");
    let _ = std::fs::remove_dir_all(directory);
    result
}

fn assert_equivalent(label: &str, source: &str) {
    let (module, source_text) = compile_source(source);
    let expected = interpreter_result(&module);
    let o0 = native_result(label, &module, &source_text, OptimizationLevel::O0);
    let o2 = native_result(label, &module, &source_text, OptimizationLevel::O2);
    assert_eq!(o0, expected, "O0 mismatch for {label}");
    assert_eq!(o2, expected, "O2 mismatch for {label}");
}

fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}

#[test]
#[ignore = "requires pinned Windows GNU native toolchain"]
fn generated_scalar_programs_match_interpreter_at_o0_and_o2() {
    let mut seed = 0x5eed_cafe_d00d_beefu64;
    for case in 0..24 {
        let iterations = 25 + next(&mut seed) % 750;
        let initial = 1 + next(&mut seed) % 20_000;
        let multiplier = 3 + next(&mut seed) % 89;
        let addend = 1 + next(&mut seed) % 997;
        let modulus = 100_003 + next(&mut seed) % 800_000;
        let branch = 2 + next(&mut seed) % 17;
        let contribution = 11 + next(&mut seed) % 181;
        let source = format!(
            r#"
fn main() -> Int {{
    var i = 0
    var x = {initial}
    var acc = 0
    while i < {iterations} {{
        x = (x * {multiplier} + {addend}) % {modulus}
        if x % {branch} == 0 {{
            acc = (acc + i + x % {contribution}) % 1000003
        }} else {{
            acc = (acc + x % 97) % 1000003
        }}
        i += 1
    }}
    return acc + i
}}
"#
        );
        assert_equivalent(&format!("generated-{case}"), &source);
    }
}

#[test]
#[ignore = "requires pinned Windows GNU native toolchain"]
fn control_flow_and_managed_text_cases_match_interpreter() {
    let cases = [
        (
            "zero-iteration",
            r#"
fn main() -> Int {
    var i = 5
    var acc = 19
    while i < 5 {
        acc += 1000
    }
    return acc + i
}
"#,
        ),
        (
            "nested-break-continue",
            r#"
fn main() -> Int {
    var outer = 0
    var inner = 0
    var acc = 0
    while outer < 31 {
        inner = 0
        while inner < 23 {
            inner += 1
            if inner % 5 == 0 {
                continue
            }
            acc = (acc + outer + inner) % 1000003
            if acc % 113 == 0 {
                break
            }
        }
        outer += 1
    }
    return acc + outer + inner
}
"#,
        ),
        (
            "managed-text-loop",
            r#"
fn main() -> Int {
    var i = 0
    var count = 0
    while i < 250 {
        if ("abcdefghijklmnopqrstuvwxyz" + "!").is_empty {
            count += 7
        } else {
            count += 1
        }
        i += 1
    }
    return count + i
}
"#,
        ),
    ];

    for (label, source) in cases {
        assert_equivalent(label, source);
    }
}
