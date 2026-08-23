use keld_interpreter::{Interpreter, ValueKind};
use keld_ir::Module;
use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};
use keld_source::{SourceId, SourceText};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
const SAMPLES: usize = 5;
const TEXT_A: &str = "abcdefghijklmnopqrstuvwxyz";
const TEXT_B: &str = "!";

const KELD_SCALAR: &str = r#"
fn main() -> Int {
    var i = 0
    var x = 1
    var acc = 0
    while i < 10000000 {
        x = (x * 48271 + 1) % 2147483647
        i += 1
        if x % 7 == 0 {
            continue
        }
        acc = (acc + x % 97) % 1000000007
    }
    return acc
}
"#;

const KELD_TEXT: &str = r#"
fn main() -> Int {
    var i = 0
    var count = 0
    while i < 1000000 {
        if ("abcdefghijklmnopqrstuvwxyz" + "!").is_empty {
            count += 7
        } else {
            count += 1
        }
        i += 1
    }
    return count + i
}
"#;

const C_SOURCE: &str = r#"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int64_t scalar(void) {
    int64_t i = 0, x = 1, acc = 0;
    while (i < 10000000) {
        x = (x * 48271 + 1) % 2147483647;
        i += 1;
        if (x % 7 == 0) continue;
        acc = (acc + x % 97) % 1000000007;
    }
    return acc;
}

static int64_t text_loop(const char *a, const char *b) {
    const size_t la = strlen(a), lb = strlen(b);
    int64_t i = 0, count = 0;
    while (i < 1000000) {
        char *s = (char *)malloc(la + lb + 1);
        if (!s) exit(2);
        memcpy(s, a, la);
        memcpy(s + la, b, lb);
        s[la + lb] = '\0';
        count += s[0] == '\0' ? 7 : 1;
        free(s);
        i += 1;
    }
    return count + i;
}

int main(int argc, char **argv) {
    if (argc < 2) return 3;
    if (strcmp(argv[1], "scalar") == 0) {
        printf("%lld\n", (long long)scalar());
        return 0;
    }
    if (argc < 4) return 4;
    printf("%lld\n", (long long)text_loop(argv[2], argv[3]));
    return 0;
}
"#;

const CPP_SOURCE: &str = r#"
#include <cstdint>
#include <iostream>
#include <string>

static std::int64_t scalar() {
    std::int64_t i = 0, x = 1, acc = 0;
    while (i < 10000000) {
        x = (x * 48271 + 1) % 2147483647;
        ++i;
        if (x % 7 == 0) continue;
        acc = (acc + x % 97) % 1000000007;
    }
    return acc;
}

static std::int64_t text_loop(const std::string& a, const std::string& b) {
    std::int64_t i = 0, count = 0;
    while (i < 1000000) {
        std::string s = a + b;
        count += s.empty() ? 7 : 1;
        ++i;
    }
    return count + i;
}

int main(int argc, char** argv) {
    if (argc < 2) return 3;
    const std::string mode = argv[1];
    if (mode == "scalar") {
        std::cout << scalar() << '\n';
        return 0;
    }
    if (argc < 4) return 4;
    std::cout << text_loop(argv[2], argv[3]) << '\n';
}
"#;

const RUST_SOURCE: &str = r#"
fn scalar() -> i64 {
    let mut i = 0_i64;
    let mut x = 1_i64;
    let mut acc = 0_i64;
    while i < 10_000_000 {
        x = (x * 48_271 + 1) % 2_147_483_647;
        i += 1;
        if x % 7 == 0 {
            continue;
        }
        acc = (acc + x % 97) % 1_000_000_007;
    }
    acc
}

fn text_loop(a: &str, b: &str) -> i64 {
    let mut i = 0_i64;
    let mut count = 0_i64;
    while i < 1_000_000 {
        let mut s = String::with_capacity(a.len() + b.len());
        s.push_str(a);
        s.push_str(b);
        count += if s.is_empty() { 7 } else { 1 };
        i += 1;
    }
    count + i
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("scalar") => println!("{}", scalar()),
        Some("text") => println!("{}", text_loop(&args[2], &args[3])),
        _ => std::process::exit(3),
    }
}
"#;

const JAVA_SOURCE: &str = r#"
public final class Bench {
    static long scalar() {
        long i = 0, x = 1, acc = 0;
        while (i < 10_000_000L) {
            x = (x * 48_271L + 1L) % 2_147_483_647L;
            i += 1;
            if (x % 7L == 0L) continue;
            acc = (acc + x % 97L) % 1_000_000_007L;
        }
        return acc;
    }

    static long textLoop(String a, String b) {
        long i = 0, count = 0;
        while (i < 1_000_000L) {
            String s = a + b;
            count += s.isEmpty() ? 7 : 1;
            i += 1;
        }
        return count + i;
    }

    public static void main(String[] args) {
        if (args.length == 0) System.exit(3);
        if (args[0].equals("scalar")) {
            System.out.println(scalar());
        } else {
            System.out.println(textLoop(args[1], args[2]));
        }
    }
}
"#;

const PYTHON_SOURCE: &str = r#"
import sys

def scalar():
    i = 0
    x = 1
    acc = 0
    while i < 10_000_000:
        x = (x * 48_271 + 1) % 2_147_483_647
        i += 1
        if x % 7 == 0:
            continue
        acc = (acc + x % 97) % 1_000_000_007
    return acc

def text_loop(a, b):
    i = 0
    count = 0
    while i < 1_000_000:
        s = a + b
        count += 7 if not s else 1
        i += 1
    return count + i

if sys.argv[1] == "scalar":
    print(scalar())
else:
    print(text_loop(sys.argv[2], sys.argv[3]))
"#;

const SWIFT_SOURCE: &str = r#"
import Foundation

func scalar() -> Int64 {
    var i: Int64 = 0
    var x: Int64 = 1
    var acc: Int64 = 0
    while i < 10_000_000 {
        x = (x * 48_271 + 1) % 2_147_483_647
        i += 1
        if x % 7 == 0 { continue }
        acc = (acc + x % 97) % 1_000_000_007
    }
    return acc
}

func textLoop(_ a: String, _ b: String) -> Int64 {
    var i: Int64 = 0
    var count: Int64 = 0
    while i < 1_000_000 {
        let s = a + b
        count += s.isEmpty ? 7 : 1
        i += 1
    }
    return count + i
}

let args = CommandLine.arguments
if args[1] == "scalar" {
    print(scalar())
} else {
    print(textLoop(args[2], args[3]))
}
"#;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn temp_dir() -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("keld-language-bench-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).expect("benchmark temporary directory");
    path
}

fn compile_keld(source: &str) -> (Module, SourceText, i64) {
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
    assert!(
        diagnostics.is_empty(),
        "invalid benchmark IR: {diagnostics:#?}"
    );
    let mut interpreter = Interpreter::new(&module).expect("interpreter setup");
    let result = interpreter.run_main().expect("interpreter run");
    let ValueKind::Int(value) = result.value.kind() else {
        panic!("benchmark main did not return Int");
    };
    (module, source_text, *value)
}

fn build_keld(
    directory: &Path,
    label: &str,
    source: &str,
    optimization: OptimizationLevel,
) -> (PathBuf, i64) {
    let (module, source_text, expected) = compile_keld(source);
    let target = workspace_root().join("target/x86_64-pc-windows-gnu/release");
    let runtime_source = target.join("keld_runtime_v1.dll");
    let runtime_import = target.join("libkeld_runtime_v1.dll.a");
    assert!(
        runtime_source.is_file(),
        "missing {}",
        runtime_source.display()
    );
    assert!(
        runtime_import.is_file(),
        "missing {}",
        runtime_import.display()
    );
    let runtime_dll = directory.join("keld_runtime_v1.dll");
    if !runtime_dll.exists() {
        std::fs::copy(&runtime_source, &runtime_dll).expect("runtime DLL copy");
    }
    let executable = directory.join(format!("keld-{label}-{optimization:?}.exe"));
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
        path: PathBuf::from(format!("bench-{label}.keld")),
        source: source_text,
    };
    build_executable(&module, &metadata, &request).expect("Keld native build");
    (executable, expected)
}

fn run_checked(program: &Path, args: &[OsString], directory: &Path, expected: i64) -> Duration {
    let started = Instant::now();
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", program.display()));
    let elapsed = started.elapsed();
    assert_success(&output, program);
    let text = String::from_utf8(output.stdout).expect("benchmark stdout UTF-8");
    let value = text.trim().parse::<i64>().unwrap_or_else(|error| {
        panic!(
            "{} returned non-Int stdout {text:?}: {error}",
            program.display()
        )
    });
    assert_eq!(
        value,
        expected,
        "checksum mismatch for {}",
        program.display()
    );
    elapsed
}

fn assert_success(output: &Output, program: &Path) {
    assert!(
        output.status.success(),
        "{} failed: status={:?}\nstdout={}\nstderr={}",
        program.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn median_ms(program: &Path, args: &[OsString], directory: &Path, expected: i64) -> f64 {
    let _ = run_checked(program, args, directory, expected);
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        samples.push(run_checked(program, args, directory, expected));
    }
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1000.0
}

fn compile_command(program: &Path, args: &[OsString], directory: &Path) {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("failed to invoke {}: {error}", program.display()));
    assert_success(&output, program);
}

fn available(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn report(language: &str, workload: &str, ms: f64, expected: i64) {
    println!(
        "LANGBENCH language={language} workload={workload} median_ms={ms:.3} samples={SAMPLES} checksum={expected} methodology=whole_process"
    );
}

#[test]
#[ignore = "manual cross-language runtime benchmark on Windows"]
fn compare_keld_with_common_languages() {
    let directory = temp_dir();
    let (keld_scalar_o0, scalar_expected) =
        build_keld(&directory, "scalar", KELD_SCALAR, OptimizationLevel::O0);
    let (keld_scalar_o2, scalar_expected_o2) =
        build_keld(&directory, "scalar", KELD_SCALAR, OptimizationLevel::O2);
    let (keld_text_o0, text_expected) =
        build_keld(&directory, "text", KELD_TEXT, OptimizationLevel::O0);
    let (keld_text_o2, text_expected_o2) =
        build_keld(&directory, "text", KELD_TEXT, OptimizationLevel::O2);
    assert_eq!(scalar_expected_o2, scalar_expected);
    assert_eq!(text_expected_o2, text_expected);

    let no_args: Vec<OsString> = Vec::new();
    report(
        "keld-o0",
        "scalar",
        median_ms(&keld_scalar_o0, &no_args, &directory, scalar_expected),
        scalar_expected,
    );
    report(
        "keld-o2",
        "scalar",
        median_ms(&keld_scalar_o2, &no_args, &directory, scalar_expected),
        scalar_expected,
    );
    report(
        "keld-o0",
        "text",
        median_ms(&keld_text_o0, &no_args, &directory, text_expected),
        text_expected,
    );
    report(
        "keld-o2",
        "text",
        median_ms(&keld_text_o2, &no_args, &directory, text_expected),
        text_expected,
    );

    let c_path = directory.join("bench.c");
    let cpp_path = directory.join("bench.cpp");
    let rust_path = directory.join("bench.rs");
    let java_path = directory.join("Bench.java");
    let python_path = directory.join("bench.py");
    let swift_path = directory.join("bench.swift");
    std::fs::write(&c_path, C_SOURCE).expect("write C source");
    std::fs::write(&cpp_path, CPP_SOURCE).expect("write C++ source");
    std::fs::write(&rust_path, RUST_SOURCE).expect("write Rust source");
    std::fs::write(&java_path, JAVA_SOURCE).expect("write Java source");
    std::fs::write(&python_path, PYTHON_SOURCE).expect("write Python source");
    std::fs::write(&swift_path, SWIFT_SOURCE).expect("write Swift source");

    let gcc = std::env::var_os("KELD_MINGW_GCC")
        .map(PathBuf::from)
        .expect("KELD_MINGW_GCC");
    let gpp = gcc.with_file_name("g++.exe");
    let c_exe = directory.join("c-bench.exe");
    let cpp_exe = directory.join("cpp-bench.exe");
    let rust_exe = directory.join("rust-bench.exe");
    compile_command(
        &gcc,
        &[
            c_path.clone().into_os_string(),
            "-O2".into(),
            "-std=c11".into(),
            "-o".into(),
            c_exe.clone().into_os_string(),
        ],
        &directory,
    );
    compile_command(
        &gpp,
        &[
            cpp_path.clone().into_os_string(),
            "-O2".into(),
            "-std=c++20".into(),
            "-o".into(),
            cpp_exe.clone().into_os_string(),
        ],
        &directory,
    );
    compile_command(
        Path::new("rustc"),
        &[
            rust_path.clone().into_os_string(),
            "-C".into(),
            "opt-level=2".into(),
            "-o".into(),
            rust_exe.clone().into_os_string(),
        ],
        &directory,
    );
    compile_command(
        Path::new("javac"),
        &[java_path.clone().into_os_string()],
        &directory,
    );

    let scalar_args = vec![OsString::from("scalar")];
    let text_args = vec![
        OsString::from("text"),
        OsString::from(TEXT_A),
        OsString::from(TEXT_B),
    ];

    for (language, executable) in [("c", &c_exe), ("cpp", &cpp_exe), ("rust", &rust_exe)] {
        report(
            language,
            "scalar",
            median_ms(executable, &scalar_args, &directory, scalar_expected),
            scalar_expected,
        );
        report(
            language,
            "text",
            median_ms(executable, &text_args, &directory, text_expected),
            text_expected,
        );
    }

    let java_scalar = vec![
        OsString::from("-cp"),
        directory.clone().into_os_string(),
        OsString::from("Bench"),
        OsString::from("scalar"),
    ];
    let java_text = vec![
        OsString::from("-cp"),
        directory.clone().into_os_string(),
        OsString::from("Bench"),
        OsString::from("text"),
        OsString::from(TEXT_A),
        OsString::from(TEXT_B),
    ];
    report(
        "java",
        "scalar",
        median_ms(Path::new("java"), &java_scalar, &directory, scalar_expected),
        scalar_expected,
    );
    report(
        "java",
        "text",
        median_ms(Path::new("java"), &java_text, &directory, text_expected),
        text_expected,
    );

    let python_scalar = vec![
        python_path.clone().into_os_string(),
        OsString::from("scalar"),
    ];
    let python_text = vec![
        python_path.clone().into_os_string(),
        OsString::from("text"),
        OsString::from(TEXT_A),
        OsString::from(TEXT_B),
    ];
    report(
        "python",
        "scalar",
        median_ms(
            Path::new("python"),
            &python_scalar,
            &directory,
            scalar_expected,
        ),
        scalar_expected,
    );
    report(
        "python",
        "text",
        median_ms(Path::new("python"), &python_text, &directory, text_expected),
        text_expected,
    );

    if available("swiftc", "--version") {
        let swift_exe = directory.join("swift-bench.exe");
        compile_command(
            Path::new("swiftc"),
            &[
                swift_path.into_os_string(),
                "-O".into(),
                "-o".into(),
                swift_exe.clone().into_os_string(),
            ],
            &directory,
        );
        report(
            "swift",
            "scalar",
            median_ms(&swift_exe, &scalar_args, &directory, scalar_expected),
            scalar_expected,
        );
        report(
            "swift",
            "text",
            median_ms(&swift_exe, &text_args, &directory, text_expected),
            text_expected,
        );
    } else {
        println!("LANGBENCH language=swift status=unavailable-on-runner");
    }

    let _ = std::fs::remove_dir_all(directory);
}
