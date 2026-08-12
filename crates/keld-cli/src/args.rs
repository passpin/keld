use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str =
    "usage: keld check <file> | keld run --engine interpreter <file> | keld dump-ir <file>";

pub(crate) enum Command {
    Check(PathBuf),
    Run(PathBuf),
    DumpIr(PathBuf),
}

impl Command {
    pub fn path(&self) -> &Path {
        match self {
            Self::Check(path) | Self::Run(path) | Self::DumpIr(path) => path,
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
        [command, engine_flag, engine, path]
            if command == OsStr::new("run")
                && engine_flag == OsStr::new("--engine")
                && engine == OsStr::new("interpreter") =>
        {
            Ok(Command::Run(PathBuf::from(path)))
        }
        _ => Err(()),
    }
}
