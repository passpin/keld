use keld_ir::TestModuleBuilder;
use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};
use keld_source::{SourceId, SourceText};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn request(root: &Path, output: PathBuf) -> BuildRequest {
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    BuildRequest {
        output,
        optimization: OptimizationLevel::O2,
        llvm_prefix: std::env::var_os("LLVM_SYS_221_PREFIX")
            .map(PathBuf::from)
            .expect("LLVM_SYS_221_PREFIX"),
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll: target.join("keld_runtime_v1.dll"),
        runtime_import_library: target.join("libkeld_runtime_v1.dll.a"),
    }
}

#[test]
fn production_executable_does_not_import_test_site() {
    let root = workspace_root();
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "keld-production-test-site-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("temporary directory");

    let output = directory.join("program.exe");
    let module = TestModuleBuilder::new().finish();
    let source =
        SourceText::from_str(SourceId(0), "production test-site audit").expect("source text");
    build_executable(
        &module,
        &SourceMetadata {
            path: PathBuf::from("production-test-site.keld"),
            source,
        },
        &request(&root, output.clone()),
    )
    .expect("production native build");

    let image = std::fs::read(&output).expect("native executable");
    let forbidden = b"keld_rt_v1_test_site";
    assert!(
        !image
            .windows(forbidden.len())
            .any(|window| window == forbidden),
        "production executable must not import the test-control site marker"
    );

    let _ = std::fs::remove_dir_all(directory);
}
