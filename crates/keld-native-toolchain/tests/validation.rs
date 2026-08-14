use keld_native_toolchain::{
    reject_msvc_fallback, require_runtime_dll, validate_archive_checksum, validate_gcc_target,
    validate_llvm_version,
};
use std::path::Path;

#[test]
fn archive_checksum_mismatch_is_rejected() {
    let error = validate_archive_checksum("llvm.pkg.tar.zst", "expected", "actual")
        .expect_err("a mismatched archive must be rejected");
    assert!(error.to_string().contains("llvm.pkg.tar.zst"));
}

#[test]
fn llvm_version_and_gcc_target_are_exact() {
    assert!(validate_llvm_version("22.1.7").is_err());
    assert!(validate_llvm_version("22.1.8").is_ok());
    assert!(validate_gcc_target("x86_64-pc-windows-msvc").is_err());
    assert!(validate_gcc_target("x86_64-w64-mingw32").is_ok());
}

#[test]
fn runtime_dll_and_msvc_fallback_are_rejected() {
    assert!(require_runtime_dll(Path::new("missing/keld_runtime_v1.dll")).is_err());
    assert!(reject_msvc_fallback(Path::new("C:/Windows/System32/link.exe")).is_err());
}

#[test]
fn lock_pins_the_complete_llvm_runtime_archive_closure() {
    let lock = include_str!("../../../.tools/llvm/llvm-packages.lock.json");
    for required in [
        "22.1.8",
        "221.0.1",
        "x86_64-w64-windows-gnu",
        "mingw-w64-x86_64-llvm",
        "mingw-w64-x86_64-llvm-libs",
        "mingw-w64-x86_64-llvm-tools",
        "mingw-w64-x86_64-gcc-libs",
        "mingw-w64-x86_64-libwinpthread",
        "mingw-w64-x86_64-tzdata",
        "mingw-w64-x86_64-libffi",
        "mingw-w64-x86_64-libxml2",
        "mingw-w64-x86_64-libiconv",
        "mingw-w64-x86_64-zlib",
        "mingw-w64-x86_64-zstd",
    ] {
        assert!(lock.contains(required), "lock is missing {required}");
    }
    assert_eq!(lock.matches("sha256").count(), 11);
    assert_eq!(lock.matches("https://").count(), 11);
}

#[test]
fn bootstrap_scripts_verify_before_publish_and_activate_the_prefix() {
    let bootstrap = include_str!("../../../scripts/bootstrap-llvm.ps1");
    assert!(bootstrap.contains("Get-FileHash -Algorithm SHA256"));
    assert!(bootstrap.contains("Move-Item -LiteralPath $candidate -Destination $prefix"));
    let activate = include_str!("../../../scripts/activate-llvm.ps1");
    assert!(activate.contains("LLVM_SYS_221_PREFIX"));
}
