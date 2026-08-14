mod args;
mod driver;
mod render;

pub use driver::{Compilation, DriverFailure, check_source, compile_source, run_source};

use args::{Command, Engine, USAGE, parse_args};
use keld_interpreter::ValueKind;
use keld_native_backend::{
    BackendError, BuildRequest, OptimizationLevel, SourceMetadata, build_executable,
};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_NATIVE_RUN: AtomicU64 = AtomicU64::new(1);

pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let Ok(command) = parse_args(arguments) else {
        let _ = writeln!(stderr, "{USAGE}");
        return 64;
    };
    execute_command(&command, &mut stdout, &mut stderr)
}

#[allow(clippy::too_many_lines)]
fn execute_command(command: &Command, stdout: &mut impl Write, stderr: &mut impl Write) -> i32 {
    let path = command.path().to_path_buf();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "{}:1:1: error[KLD0001]: unable to read source: {error}",
                path.display()
            );
            return 1;
        }
    };
    match command {
        Command::Check(_) => {
            let compilation = check_source(&path, bytes);
            if compilation.diagnostics.is_empty() {
                0
            } else {
                let _ = write!(stderr, "{}", render::diagnostics(&compilation));
                1
            }
        }
        Command::Build { output, .. } => {
            let compilation = compile_source(&path, bytes);
            if !compilation.diagnostics.is_empty() {
                let _ = write!(stderr, "{}", render::diagnostics(&compilation));
                return 1;
            }
            let Some(source) = compilation.source.clone() else {
                let _ = writeln!(stderr, "internal error: source text is missing");
                return 70;
            };
            let Some(ir) = compilation.ir.as_ref() else {
                let _ = writeln!(stderr, "internal error: validated IR is missing");
                return 70;
            };
            let request = native_request(output, OptimizationLevel::O2);
            match build_executable(ir, &SourceMetadata { path, source }, &request) {
                Ok(_) => 0,
                Err(BackendError::InvalidIr(diagnostics)) => {
                    let _ = writeln!(stderr, "internal error: native IR validation failed");
                    for diagnostic in diagnostics {
                        let _ = writeln!(stderr, "{diagnostic:?}");
                    }
                    70
                }
                Err(error) => {
                    let _ = writeln!(stderr, "{error}");
                    70
                }
            }
        }
        Command::DumpIr(_) => {
            let compilation = compile_source(&path, bytes);
            if !compilation.diagnostics.is_empty() {
                let _ = write!(stderr, "{}", render::diagnostics(&compilation));
                return 1;
            }
            let Some(ir) = compilation.ir else {
                let _ = writeln!(stderr, "internal error: validated IR is missing");
                return 70;
            };
            let _ = write!(stdout, "{}", ir.dump());
            0
        }
        Command::Run { engine, .. } => match engine {
            Engine::Interpreter => match run_source(&path, bytes) {
                Ok(result) => {
                    if let ValueKind::Int(value) = result.value.kind() {
                        let _ = writeln!(stdout, "{value}");
                        0
                    } else {
                        let _ = writeln!(stderr, "internal error: main returned a non-Int value");
                        70
                    }
                }
                Err(DriverFailure::Static(compilation)) => {
                    let _ = write!(stderr, "{}", render::diagnostics(&compilation));
                    1
                }
                Err(DriverFailure::Runtime {
                    fault,
                    path,
                    source,
                }) => {
                    let _ = write!(stderr, "{}", render::runtime_fault(&path, &source, &fault));
                    2
                }
                Err(DriverFailure::Internal(error)) => {
                    let _ = writeln!(stderr, "internal error: {error}");
                    70
                }
            },
            Engine::Native => run_native(&path, bytes, stderr),
        },
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn configured_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn native_runtime_paths() -> (PathBuf, PathBuf) {
    let root = workspace_root();
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    let dll =
        configured_path("KELD_RUNTIME_DLL").unwrap_or_else(|| target.join("keld_runtime_v1.dll"));
    let import = configured_path("KELD_RUNTIME_IMPORT_LIBRARY")
        .unwrap_or_else(|| target.join("libkeld_runtime_v1.dll.a"));
    (dll, import)
}

fn native_llvm_prefix() -> PathBuf {
    configured_path("KELD_LLVM_PREFIX")
        .or_else(|| configured_path("LLVM_SYS_221_PREFIX"))
        .unwrap_or_else(|| workspace_root().join(".tools/llvm/22.1.8-mingw64"))
}

fn native_request(output: &Path, optimization: OptimizationLevel) -> BuildRequest {
    let (runtime_dll, runtime_import_library) = native_runtime_paths();
    BuildRequest {
        output: output.to_path_buf(),
        optimization,
        llvm_prefix: native_llvm_prefix(),
        gcc: configured_path("KELD_MINGW_GCC"),
        runtime_dll,
        runtime_import_library,
    }
}

fn run_native(path: &Path, bytes: Vec<u8>, stderr: &mut impl Write) -> i32 {
    let id = NEXT_NATIVE_RUN.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("keld-native-run-{}-{id}", std::process::id()));
    if let Err(error) = std::fs::create_dir(&directory) {
        let _ = writeln!(
            stderr,
            "internal error: unable to create native run directory: {error}"
        );
        return 70;
    }
    let output = directory.join("program.exe");
    let result = (|| {
        let compilation = compile_source(path, bytes);
        if !compilation.diagnostics.is_empty() {
            let _ = write!(stderr, "{}", render::diagnostics(&compilation));
            return 1;
        }
        let Some(source) = compilation.source.clone() else {
            let _ = writeln!(stderr, "internal error: source text is missing");
            return 70;
        };
        let Some(ir) = compilation.ir.as_ref() else {
            let _ = writeln!(stderr, "internal error: validated IR is missing");
            return 70;
        };
        let request = native_request(&output, OptimizationLevel::O0);
        if let Err(error) = build_executable(
            ir,
            &SourceMetadata {
                path: path.to_path_buf(),
                source,
            },
            &request,
        ) {
            let _ = writeln!(stderr, "{error}");
            return 70;
        }
        match ProcessCommand::new(&output).status() {
            Ok(status) => status.code().unwrap_or(70),
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "internal error: unable to run native executable: {error}"
                );
                70
            }
        }
    })();
    if let Err(error) = std::fs::remove_dir_all(&directory) {
        let _ = writeln!(
            stderr,
            "internal error: unable to remove native run directory: {error}"
        );
        return 70;
    }
    result
}
