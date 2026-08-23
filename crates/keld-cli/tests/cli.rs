use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn keld() -> Command {
    Command::new(env!("CARGO_BIN_EXE_keld"))
}

fn assert_native_artifacts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(PathBuf::from)
        .expect("workspace root");
    let target = root.join("target/x86_64-pc-windows-gnu/release");
    assert!(
        target.join("keld_runtime_v1.dll").is_file(),
        "build the native runtime DLL before CLI native tests"
    );
    assert!(
        target.join("libkeld_runtime_v1.dll.a").is_file(),
        "build the native runtime import library before CLI native tests"
    );
}

#[test]
fn run_representative_programs_prints_only_the_main_result() {
    for (name, expected) in [
        ("cyclic_graph.keld", "20\n"),
        ("keep_survives.keld", "30\n"),
        ("stale_link.keld", "4\n"),
        ("alias_distinct.keld", "20\n"),
        ("broad_retirement.keld", "40\n"),
        ("numeric_edges.keld", "0\n"),
        ("control_flow_loop.keld", "8\n"),
        ("control_flow_allocations.keld", "6\n"),
    ] {
        let output = keld()
            .args(["run", "--engine", "interpreter"])
            .arg(fixture(name))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{name}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "{name}"
        );
        assert!(output.stderr.is_empty(), "{name}");
    }
}

#[test]
fn check_is_silent_on_success_and_static_failures_have_stable_codes() {
    let success = keld()
        .arg("check")
        .arg(fixture("cyclic_graph.keld"))
        .output()
        .unwrap();
    assert_eq!(success.status.code(), Some(0));
    assert!(success.stdout.is_empty());
    assert!(success.stderr.is_empty());

    for (name, line, column, code, message, help) in [
        (
            "fail_retired_use.keld",
            5,
            11,
            "KLD1001",
            "use of a retired entity reference",
            "move this use before `retire` or remove the retirement",
        ),
        (
            "fail_may_alias.keld",
            5,
            11,
            "KLD1008",
            "this reference may alias an entity retired here or by a call",
            "prove the identities distinct before retirement, or avoid the broad retirement",
        ),
        (
            "fail_unsupported.keld",
            2,
            5,
            "KLD0004",
            "`match` is parsed but not supported by the bootstrap compiler",
            "remove this feature or use the currently supported bootstrap subset",
        ),
    ] {
        let path = fixture(name);
        let output = keld().arg("check").arg(&path).output().unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(1), "{name}: {stderr}");
        assert_eq!(
            stderr,
            format!(
                "{}:{line}:{column}: error[{code}]: {message}\n  --> {message}\n  = help: {help}\n",
                path.display()
            ),
            "{name}"
        );
    }
}

#[test]
fn missing_source_is_a_static_read_failure() {
    let path = fixture("missing.keld");
    let output = keld().arg("check").arg(&path).output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        stderr.starts_with(&format!(
            "{}:1:1: error[KLD0001]: unable to read source: ",
            path.display()
        )),
        "{stderr}"
    );
    assert!(stderr.ends_with('\n'));
}

#[test]
fn runtime_fault_and_command_misuse_have_distinct_exit_codes() {
    let runtime = keld()
        .args(["run", "--engine", "interpreter"])
        .arg(fixture("runtime_div_zero.keld"))
        .output()
        .unwrap();
    assert_eq!(runtime.status.code(), Some(2));
    assert!(runtime.stdout.is_empty());
    assert!(
        String::from_utf8(runtime.stderr)
            .unwrap()
            .contains("DivisionByZeroFault")
    );

    let misuse = keld().args(["run", "--engine", "native"]).output().unwrap();
    assert_eq!(misuse.status.code(), Some(64));
    assert!(misuse.stdout.is_empty());
    assert_eq!(
        String::from_utf8(misuse.stderr).unwrap(),
        "usage: keld check <file> | keld build <source> -o <program.exe> | keld run --engine <interpreter|native> <file> | keld dump-ir <file>\n"
    );
}

#[test]
fn impossible_list_capacity_is_a_capacity_runtime_fault() {
    let output = keld()
        .args(["run", "--engine", "interpreter"])
        .arg(fixture("runtime_capacity.keld"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("CapacityFault")
    );
}

#[test]
fn deferred_containers_and_views_remain_outside_the_bootstrap_surface() {
    for (name, source) in [
        (
            "map",
            "fn main() -> Int { let value: Map[Int] = none; return 0 }\n",
        ),
        (
            "set",
            "fn main() -> Int { let value: Set[Int] = none; return 0 }\n",
        ),
        (
            "slice",
            "fn main() -> Int { let value: Slice[Int] = none; return 0 }\n",
        ),
        (
            "iterator",
            "fn main() -> Int { let items: List[Int] = List(); return items.iterator() }\n",
        ),
        (
            "for",
            "fn main() -> Int { for item in List() { return 0 } return 0 }\n",
        ),
        (
            "substring",
            "fn main() -> Int { return \"text\".substring(0, 1) }\n",
        ),
        ("text-index", "fn main() -> Int { return \"text\"[0] }\n"),
    ] {
        let path = std::env::temp_dir().join(format!("keld-task12-{name}.keld"));
        std::fs::write(&path, source).expect("temporary source must be writable");
        let output = keld().arg("check").arg(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(output.status.code(), Some(1), "{name}");
    }
}

#[test]
fn dump_ir_is_stable() {
    let path = fixture("numeric_edges.keld");
    let first = keld().arg("dump-ir").arg(&path).output().unwrap();
    let second = keld().arg("dump-ir").arg(path).output().unwrap();

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(first.stdout, second.stdout);
    assert!(first.stderr.is_empty());
    assert!(
        String::from_utf8(first.stdout)
            .unwrap()
            .contains("function f0")
    );
}

#[test]
fn native_engine_matches_representative_source_fixtures() {
    assert_native_artifacts();
    for (name, expected) in [
        ("cyclic_graph.keld", "20\n"),
        ("keep_survives.keld", "30\n"),
        ("stale_link.keld", "4\n"),
        ("alias_distinct.keld", "20\n"),
        ("broad_retirement.keld", "40\n"),
        ("numeric_edges.keld", "0\n"),
        ("control_flow_loop.keld", "8\n"),
        ("control_flow_allocations.keld", "6\n"),
    ] {
        let output = keld()
            .args(["run", "--engine", "native"])
            .arg(fixture(name))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{name}");
        assert_eq!(output.stdout, expected.as_bytes(), "{name}");
        assert!(output.stderr.is_empty(), "{name}");
    }
}

#[test]
fn native_engine_forwards_runtime_faults_with_interpreter_format() {
    assert_native_artifacts();
    for (name, kind) in [
        ("runtime_div_zero.keld", "DivisionByZeroFault"),
        ("runtime_capacity.keld", "CapacityFault"),
    ] {
        let output = keld()
            .args(["run", "--engine", "native"])
            .arg(fixture(name))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(output.stdout.is_empty(), "{name}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(kind), "{name}: {stderr}");
    }
}

#[test]
fn native_build_handles_unicode_paths_and_refuses_collisions() {
    assert_native_artifacts();
    let directory = std::env::temp_dir().join(format!(
        "keld-cli-native-build-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source space 한글.keld");
    let output = directory.join("program space 한글.exe");
    std::fs::copy(fixture("cyclic_graph.keld"), &source).unwrap();
    let built = keld()
        .args(["build"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(
        built.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(output.is_file());
    assert!(directory.join("keld_runtime_v1.dll").is_file());
    let child = Command::new(&output)
        .env("PATH", "C:\\Windows\\System32;C:\\Windows")
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(0));
    assert_eq!(child.stdout, b"20\n");
    assert!(child.stderr.is_empty());

    let collision = keld()
        .args(["build"])
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(collision.status.code(), Some(70));
    assert!(String::from_utf8_lossy(&collision.stderr).contains("refusing to overwrite"));

    let mismatch_dir = directory.join("mismatch");
    std::fs::create_dir_all(&mismatch_dir).unwrap();
    let mismatch_source = mismatch_dir.join("source.keld");
    let mismatch_output = mismatch_dir.join("program.exe");
    std::fs::copy(&source, &mismatch_source).unwrap();
    std::fs::write(mismatch_dir.join("keld_runtime_v1.dll"), b"wrong runtime").unwrap();
    let mismatch = keld()
        .args(["build"])
        .arg(&mismatch_source)
        .arg("-o")
        .arg(&mismatch_output)
        .output()
        .unwrap();
    assert_eq!(mismatch.status.code(), Some(70));
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("sibling runtime DLL differs"));
    assert!(!mismatch_output.exists());
    std::fs::remove_dir_all(directory).unwrap();
}
