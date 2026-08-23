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
const A: &str = "abcdefghijklmnopqrstuvwxyz";
const B: &str = "!";
const EXPECTED: &str = "abcdefghijklmnopqrstuvwxyz!";

const KELD_SOURCE: &str = r#"
fn main() -> Int {
    let a = "abcdefghijklmnopqrstuvwxyz"
    let b = "!"
    let expected = "abcdefghijklmnopqrstuvwxyz!"
    var i = 0
    var count = 0
    while i < 1000000 {
        if (a + b) == expected {
            count += 1
        } else {
            count += 7
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

int main(int argc, char **argv) {
    if (argc < 4) return 3;
    const char *a = argv[1], *b = argv[2], *expected = argv[3];
    const size_t la = strlen(a), lb = strlen(b);
    int64_t i = 0, count = 0;
    while (i < 1000000) {
        char *s = (char *)malloc(la + lb + 1);
        if (!s) return 2;
        memcpy(s, a, la);
        memcpy(s + la, b, lb);
        s[la + lb] = '\0';
        count += strcmp(s, expected) == 0 ? 1 : 7;
        free(s);
        i += 1;
    }
    printf("%lld\n", (long long)(count + i));
    return 0;
}
"#;

const CPP_SOURCE: &str = r"
#include <cstdint>
#include <iostream>
#include <string>

int main(int argc, char **argv) {
    if (argc < 4) return 3;
    const std::string a = argv[1], b = argv[2], expected = argv[3];
    std::int64_t i = 0, count = 0;
    while (i < 1000000) {
        const std::string s = a + b;
        count += s == expected ? 1 : 7;
        ++i;
    }
    std::cout << count + i << '\n';
}
";

const RUST_SOURCE: &str = r#"
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let a = &args[1];
    let b = &args[2];
    let expected = &args[3];
    let mut i = 0_i64;
    let mut count = 0_i64;
    while i < 1_000_000 {
        let mut s = String::with_capacity(a.len() + b.len());
        s.push_str(a);
        s.push_str(b);
        count += if &s == expected { 1 } else { 7 };
        i += 1;
    }
    println!("{}", count + i);
}
"#;

const JAVA_SOURCE: &str = r"
public final class TextContentBench {
    public static void main(String[] args) {
        if (args.length < 3) System.exit(3);
        String a = args[0], b = args[1], expected = args[2];
        long i = 0, count = 0;
        while (i < 1_000_000L) {
            String s = a + b;
            count += s.equals(expected) ? 1 : 7;
            i += 1;
        }
        System.out.println(count + i);
    }
}
";

const PYTHON_SOURCE: &str = r"
import sys

a, b, expected = sys.argv[1:4]
i = 0
count = 0
while i < 1_000_000:
    s = a + b
    count += 1 if s == expected else 7
    i += 1
print(count + i)
";

const SWIFT_SOURCE: &str = r"
import Foundation

let args = CommandLine.arguments
let a = args[1], b = args[2], expected = args[3]
var i: Int64 = 0
var count: Int64 = 0
while i < 1_000_000 {
    let s = a + b
    count += s == expected ? 1 : 7
    i += 1
}
print(count + i)
";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn temp_dir() -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "keld-text-content-bench-{}-{id}",
        std::process::id()
    ));
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
    assert!(keld_ir::validate(&module).is_empty());
    let mut interpreter = Interpreter::new(&module).expect("interpreter setup");
    let result = interpreter.run_main().expect("interpreter run");
    let ValueKind::Int(value) = result.value.kind() else {
        panic!("text benchmark main did not return Int");
    };
    (module, source_text, *value)
}

fn build_keld(directory: &Path, optimization: OptimizationLevel) -> (PathBuf, i64) {
    let (module, source, expected) = compile_keld(KELD_SOURCE);
    let target = workspace_root().join("target/x86_64-pc-windows-gnu/release");
    let runtime_source = target.join("keld_runtime_v1.dll");
    let runtime_import = target.join("libkeld_runtime_v1.dll.a");
    let runtime_dll = directory.join("keld_runtime_v1.dll");
    if !runtime_dll.exists() {
        std::fs::copy(runtime_source, &runtime_dll).expect("runtime DLL copy");
    }
    let executable = directory.join(format!("keld-text-content-{optimization:?}.exe"));
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
    build_executable(
        &module,
        &SourceMetadata {
            path: PathBuf::from("text-content-bench.keld"),
            source,
        },
        &request,
    )
    .expect("Keld native build");
    (executable, expected)
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

fn run_checked(program: &Path, args: &[OsString], directory: &Path, expected: i64) -> Duration {
    let started = Instant::now();
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", program.display()));
    let elapsed = started.elapsed();
    assert_success(&output, program);
    let stdout = String::from_utf8(output.stdout).expect("benchmark stdout UTF-8");
    assert_eq!(
        stdout.trim().parse::<i64>().expect("integer checksum"),
        expected,
        "checksum mismatch for {}",
        program.display()
    );
    elapsed
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

fn write_sources(directory: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let c = directory.join("text.c");
    let cpp = directory.join("text.cpp");
    let rust = directory.join("text.rs");
    let java = directory.join("TextContentBench.java");
    let python = directory.join("text.py");
    let swift = directory.join("text.swift");
    for (path, source) in [
        (&c, C_SOURCE),
        (&cpp, CPP_SOURCE),
        (&rust, RUST_SOURCE),
        (&java, JAVA_SOURCE),
        (&python, PYTHON_SOURCE),
        (&swift, SWIFT_SOURCE),
    ] {
        std::fs::write(path, source).expect("write benchmark source");
    }
    (c, cpp, rust, java, python, swift)
}

fn build_native_languages(
    directory: &Path,
    sources: &(PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf),
) -> (PathBuf, PathBuf, PathBuf) {
    let gcc = std::env::var_os("KELD_MINGW_GCC")
        .map(PathBuf::from)
        .expect("KELD_MINGW_GCC");
    let c_exe = directory.join("c-text.exe");
    let cpp_exe = directory.join("cpp-text.exe");
    let rust_exe = directory.join("rust-text.exe");
    compile_command(
        &gcc,
        &[
            sources.0.clone().into_os_string(),
            "-O2".into(),
            "-std=c11".into(),
            "-o".into(),
            c_exe.clone().into_os_string(),
        ],
        directory,
    );
    compile_command(
        &gcc.with_file_name("g++.exe"),
        &[
            sources.1.clone().into_os_string(),
            "-O2".into(),
            "-std=c++20".into(),
            "-o".into(),
            cpp_exe.clone().into_os_string(),
        ],
        directory,
    );
    compile_command(
        Path::new("rustc"),
        &[
            sources.2.clone().into_os_string(),
            "-C".into(),
            "opt-level=2".into(),
            "-o".into(),
            rust_exe.clone().into_os_string(),
        ],
        directory,
    );
    compile_command(
        Path::new("javac"),
        &[sources.3.clone().into_os_string()],
        directory,
    );
    (c_exe, cpp_exe, rust_exe)
}

fn report(language: &str, ms: f64, expected: i64) {
    println!(
        "TEXTCONTENT language={language} median_ms={ms:.3} samples={SAMPLES} checksum={expected} methodology=whole_process"
    );
}

#[test]
#[ignore = "manual cross-language text-content benchmark on Windows"]
fn compare_text_content_consumption() {
    let directory = temp_dir();
    let (keld_o0, expected) = build_keld(&directory, OptimizationLevel::O0);
    let (keld_o2, expected_o2) = build_keld(&directory, OptimizationLevel::O2);
    assert_eq!(expected_o2, expected);
    let no_args = Vec::<OsString>::new();
    report(
        "keld-o0",
        median_ms(&keld_o0, &no_args, &directory, expected),
        expected,
    );
    report(
        "keld-o2",
        median_ms(&keld_o2, &no_args, &directory, expected),
        expected,
    );

    let sources = write_sources(&directory);
    let (c, cpp, rust) = build_native_languages(&directory, &sources);
    let args = vec![A.into(), B.into(), EXPECTED.into()];
    for (language, executable) in [("c", &c), ("cpp", &cpp), ("rust", &rust)] {
        report(
            language,
            median_ms(executable, &args, &directory, expected),
            expected,
        );
    }

    let java_args = vec![
        "-cp".into(),
        directory.clone().into_os_string(),
        "TextContentBench".into(),
        A.into(),
        B.into(),
        EXPECTED.into(),
    ];
    report(
        "java",
        median_ms(Path::new("java"), &java_args, &directory, expected),
        expected,
    );
    let python_args = vec![
        sources.4.clone().into_os_string(),
        A.into(),
        B.into(),
        EXPECTED.into(),
    ];
    report(
        "python",
        median_ms(Path::new("python"), &python_args, &directory, expected),
        expected,
    );

    if Command::new("swiftc")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        let swift = directory.join("swift-text.exe");
        compile_command(
            Path::new("swiftc"),
            &[
                sources.5.clone().into_os_string(),
                "-O".into(),
                "-o".into(),
                swift.clone().into_os_string(),
            ],
            &directory,
        );
        report(
            "swift",
            median_ms(&swift, &args, &directory, expected),
            expected,
        );
    } else {
        println!("TEXTCONTENT language=swift status=unavailable-on-runner");
    }
    let _ = std::fs::remove_dir_all(directory);
}
