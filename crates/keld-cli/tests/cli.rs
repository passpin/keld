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

#[test]
fn run_representative_programs_prints_only_the_main_result() {
    for (name, expected) in [
        ("cyclic_graph.keld", "20\n"),
        ("keep_survives.keld", "30\n"),
        ("stale_link.keld", "4\n"),
        ("alias_distinct.keld", "20\n"),
        ("broad_retirement.keld", "40\n"),
        ("numeric_edges.keld", "0\n"),
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
            1,
            19,
            "KLD0004",
            "`var` is parsed but not supported by the bootstrap compiler",
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
        "usage: keld check <file> | keld run --engine interpreter <file> | keld dump-ir <file>\n"
    );
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
