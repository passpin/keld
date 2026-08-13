mod args;
mod driver;
mod render;

pub use driver::{Compilation, DriverFailure, check_source, compile_source, run_source};

use args::{Command, USAGE, parse_args};
use keld_interpreter::ValueKind;
use std::ffi::OsString;
use std::io::Write;

pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let Ok(command) = parse_args(arguments) else {
        let _ = writeln!(stderr, "{USAGE}");
        return 64;
    };
    execute_command(&command, &mut stdout, &mut stderr)
}

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
        Command::Run(_) => match run_source(&path, bytes) {
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
    }
}
