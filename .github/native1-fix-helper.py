from pathlib import Path
import sys


def add_test() -> None:
    path = Path("crates/keld-native-backend/tests/native_int.rs")
    source = path.read_text(encoding="utf-8")
    marker = "#[test]\nfn native_program_rejects_runtime_abi_version_mismatch()"
    if marker not in source:
        raise SystemExit("regression insertion marker missing")
    test = r'''#[test]
fn ordinary_coff_runtime_symbol_override_is_rejected() {
    let (runtime_dll, import_library) = runtime_artifacts("ordinary-coff-override");
    let directory = runtime_dll.parent().expect("runtime directory");
    let llvm_prefix = std::env::var_os("LLVM_SYS_221_PREFIX")
        .map(PathBuf::from)
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(".tools/llvm/22.1.8-mingw64"))
        })
        .expect("LLVM prefix");
    let assembly = directory.join("runtime-override.s");
    std::fs::write(
        &assembly,
        ".text\n.globl keld_rt_v1_print_int\nkeld_rt_v1_print_int:\n  ret\n",
    )
    .expect("runtime override assembly");
    let object = directory.join("runtime-override.o");
    let status = Command::new(llvm_prefix.join("bin/llvm-mc.exe"))
        .arg("-triple=x86_64-w64-windows-gnu")
        .arg("-filetype=obj")
        .arg("-o")
        .arg(&object)
        .arg(&assembly)
        .status()
        .expect("llvm-mc");
    assert!(status.success(), "llvm-mc failed: {status}");

    let ar = std::env::var_os("KELD_AR").unwrap_or_else(|| "ar".into());
    let listing = Command::new(&ar)
        .args(["t", import_library.to_str().expect("import library path")])
        .output()
        .expect("ar list");
    assert!(listing.status.success(), "ar list failed");
    let first_member = String::from_utf8(listing.stdout)
        .expect("archive member names")
        .lines()
        .next()
        .expect("runtime import archive member")
        .to_owned();
    let object_name = object
        .file_name()
        .and_then(|name| name.to_str())
        .expect("override object name");
    let status = Command::new(&ar)
        .current_dir(directory)
        .arg("r")
        .arg(&import_library)
        .arg(object_name)
        .status()
        .expect("ar append");
    assert!(status.success(), "ar append failed: {status}");
    let status = Command::new(&ar)
        .current_dir(directory)
        .arg("mb")
        .arg(&first_member)
        .arg(&import_library)
        .arg(object_name)
        .status()
        .expect("ar move");
    assert!(status.success(), "ar move failed: {status}");

    let output = directory.join("ordinary-coff-override.exe");
    let error = build_executable(
        &const_module(7),
        &metadata(),
        &request(
            &output,
            &runtime_dll,
            &import_library,
            OptimizationLevel::O0,
        ),
    )
    .expect_err("ordinary COFF runtime symbol override must be rejected");
    assert!(matches!(error, BackendError::Toolchain(_)));
    assert!(
        error
            .to_string()
            .contains("runtime import library must contain a COFF import")
    );
    assert!(!output.exists());
}

'''
    path.write_text(source.replace(marker, test + marker, 1), encoding="utf-8")


def apply_fix() -> None:
    path = Path("crates/keld-native-backend/src/lib.rs")
    source = path.read_text(encoding="utf-8")
    old = '''    if coff_section_data(member, b".idata$6").is_none() {
        return true;
    }
'''
    new = '''    if coff_section_data(member, b".idata$6").is_none() {
        let Some(runtime_symbols) = coff_symbols_matching(member, |name, _section| {
            name.starts_with(b"keld_rt_v1_") || name.starts_with(b"__imp_keld_rt_v1_")
        }) else {
            return true;
        };
        return runtime_symbols.is_empty();
    }
'''
    if source.count(old) != 1:
        raise SystemExit("validator patch target changed")
    path.write_text(source.replace(old, new, 1), encoding="utf-8")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: native1-fix-helper.py add-test|apply-fix")
    if sys.argv[1] == "add-test":
        add_test()
    elif sys.argv[1] == "apply-fix":
        apply_fix()
    else:
        raise SystemExit(f"unknown action: {sys.argv[1]}")


if __name__ == "__main__":
    main()
