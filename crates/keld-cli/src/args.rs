use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str = "usage: keld check <file> | keld build <source> -o <program.exe> | keld run --engine <interpreter|native> <file> | keld dump-ir <file>";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Engine {
    Interpreter,
    Native,
}

pub(crate) enum Command {
    Check(PathBuf),
    Build { source: PathBuf, output: PathBuf },
    Run { engine: Engine, path: PathBuf },
    DumpIr(PathBuf),
}

impl Command {
    pub fn path(&self) -> &Path {
        match self {
            Self::Check(path) | Self::DumpIr(path) => path,
            Self::Build { source, .. } | Self::Run { path: source, .. } => source,
        }
    }
}

pub(crate) fn parse_args(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    match arguments.as_slice() {
        [command, path] if command == OsStr::new("check") => {
            Ok(Command::Check(PathBuf::from(path)))
        }
        [command, path] if command == OsStr::new("dump-ir") => {
            Ok(Command::DumpIr(PathBuf::from(path)))
        }
        [command, source, output_flag, output]
            if command == OsStr::new("build") && output_flag == OsStr::new("-o") =>
        {
            Ok(Command::Build {
                source: PathBuf::from(source),
                output: PathBuf::from(output),
            })
        }
        [command, engine_flag, engine, path]
            if command == OsStr::new("run") && engine_flag == OsStr::new("--engine") =>
        {
            let engine = match engine.as_os_str() {
                value if value == OsStr::new("interpreter") => Engine::Interpreter,
                value if value == OsStr::new("native") => Engine::Native,
                _ => return Err(()),
            };
            Ok(Command::Run {
                engine,
                path: PathBuf::from(path),
            })
        }
        _ => Err(()),
    }
}
