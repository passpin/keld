use keld_interpreter::{
    AllocationObservation, Interpreter, InterpreterFailure, RuntimeFault, RuntimeFaultKind,
    TestControls, ValueKind,
};
use keld_ir::{AllocationPhase, AllocationSchedule, Module};
use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};
use keld_source::{SourceId, SourceText};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Event {
    site_id: u32,
    phase: AllocationPhase,
    attempt: u64,
    allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Observation {
    Returned(i64),
    Fault {
        kind: RuntimeFaultKind,
        span: keld_source::Span,
    },
    Internal(String),
}

#[derive(Debug)]
struct NativeRun {
    output: Output,
    events: Vec<Event>,
}

type Failure = (u32, AllocationPhase, u64);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn compile_fixture(source: &str) -> (Module, SourceText) {
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
    (module, source_text)
}

fn temp_dir(tag: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "keld-native-diff-{tag}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("differential temporary directory");
    path
}

fn test_runtime_dll() -> PathBuf {
    let path =
        workspace_root().join("target/x86_64-pc-windows-gnu/release/keld_runtime_v1_test.dll");
    assert!(
        path.is_file(),
        "build keld-native-ffi-test first: {}",
        path.display()
    );
    path
}

fn runtime_artifacts(directory: &Path) -> (PathBuf, PathBuf) {
    let source = test_runtime_dll();
    let dll = directory.join("keld_runtime_v1.dll");
    std::fs::copy(source, &dll).expect("test runtime DLL copy");
    let def = directory.join("runtime.def");
    std::fs::write(&def, RUNTIME_EXPORTS).expect("runtime export definition");
    let import = directory.join("libkeld_runtime_v1.dll.a");
    let dlltool = std::env::var_os("KELD_DLLTOOL").unwrap_or_else(|| "dlltool".into());
    let result = Command::new(dlltool)
        .args([
            "--input-def",
            def.to_str().expect("definition path"),
            "--dllname",
            "keld_runtime_v1.dll",
            "--output-lib",
        ])
        .arg(&import)
        .arg(&dll)
        .status()
        .expect("dlltool");
    assert!(result.success(), "dlltool failed: {result}");
    (dll, import)
}

fn request(
    output: &Path,
    dll: &Path,
    import: &Path,
    optimization: OptimizationLevel,
) -> BuildRequest {
    let llvm_prefix = std::env::var_os("LLVM_SYS_221_PREFIX").map_or_else(
        || workspace_root().join(".tools/llvm/22.1.8-mingw64"),
        PathBuf::from,
    );
    BuildRequest {
        output: output.to_path_buf(),
        optimization,
        llvm_prefix,
        gcc: std::env::var_os("KELD_MINGW_GCC").map(PathBuf::from),
        runtime_dll: dll.to_path_buf(),
        runtime_import_library: import.to_path_buf(),
    }
}

fn build_native(
    module: &Module,
    path: &Path,
    source: &SourceText,
    optimization: OptimizationLevel,
    directory: &Path,
) -> PathBuf {
    let (dll, import) = runtime_artifacts(directory);
    let output = directory.join(format!("program-{optimization:?}.exe"));
    build_executable(
        module,
        &SourceMetadata {
            path: path.to_path_buf(),
            source: source.clone(),
        },
        &request(&output, &dll, &import, optimization),
    )
    .expect("native differential build");
    output
}

fn interpreter_run(
    module: &Module,
    failures: &[(u32, AllocationPhase, u64)],
) -> (Observation, Vec<Event>) {
    let controls = TestControls::fail_allocation_schedule(failures.iter().copied());
    let mut interpreter = match Interpreter::with_controls_for_test(module, controls) {
        Ok(interpreter) => interpreter,
        Err(InterpreterFailure::Runtime(fault)) => {
            return (
                Observation::Fault {
                    kind: fault.kind,
                    span: fault.span,
                },
                Vec::new(),
            );
        }
        Err(InterpreterFailure::Internal(error)) => {
            return (Observation::Internal(error.to_string()), Vec::new());
        }
    };
    let result = interpreter.run_main();
    let events = interpreter
        .allocation_observations_for_test()
        .iter()
        .map(event_from_interpreter)
        .collect();
    let observation = match result {
        Ok(result) => match result.value.kind() {
            ValueKind::Int(value) => Observation::Returned(*value),
            other => Observation::Internal(format!("main returned {other:?}")),
        },
        Err(InterpreterFailure::Runtime(fault)) => Observation::Fault {
            kind: fault.kind,
            span: fault.span,
        },
        Err(InterpreterFailure::Internal(error)) => Observation::Internal(error.to_string()),
    };
    (observation, events)
}

fn event_from_interpreter(observation: &AllocationObservation) -> Event {
    Event {
        site_id: observation.site_id,
        phase: observation.phase,
        attempt: observation.attempt,
        allowed: observation.allowed,
    }
}

fn native_run(
    executable: &Path,
    directory: &Path,
    failures: &[(u32, AllocationPhase, u64)],
) -> NativeRun {
    let control = directory.join("control.txt");
    let observation = directory.join("observation.txt");
    let mut controls = String::new();
    for (site, phase, attempt) in failures {
        writeln!(
            controls,
            "site_id={site} phase={} attempt={attempt}",
            phase.as_str()
        )
        .expect("control schedule formatting");
    }
    std::fs::write(&control, controls).expect("control schedule");
    let _ = std::fs::remove_file(&observation);
    let output = Command::new(executable)
        .env("KELD_TEST_CONTROL", &control)
        .env("KELD_TEST_OBSERVATION", &observation)
        .output()
        .expect("native differential executable");
    let events = parse_events(&observation);
    NativeRun { output, events }
}

fn parse_events(path: &Path) -> Vec<Event> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != "allocation" {
                return None;
            }
            let site_id = fields.next()?.strip_prefix("site_id=")?.parse().ok()?;
            let phase = parse_phase(fields.next()?.strip_prefix("phase=")?)?;
            let attempt = fields.next()?.strip_prefix("attempt=")?.parse().ok()?;
            let allowed = fields.next()?.strip_prefix("allowed=")? == "1";
            Some(Event {
                site_id,
                phase,
                attempt,
                allowed,
            })
        })
        .collect()
}

fn parse_phase(value: &str) -> Option<AllocationPhase> {
    Some(match value {
        "context" => AllocationPhase::Context,
        "lifecycle" => AllocationPhase::Lifecycle,
        "text" => AllocationPhase::Text,
        "copy" => AllocationPhase::Copy,
        "concat" => AllocationPhase::Concat,
        "struct" => AllocationPhase::Struct,
        "entity" => AllocationPhase::Entity,
        "handle" => AllocationPhase::Handle,
        "list_growth_preferred" => AllocationPhase::ListGrowthPreferred,
        "list_growth_exact" => AllocationPhase::ListGrowthExact,
        _ => return None,
    })
}

fn expected_stderr(path: &Path, source: &SourceText, fault: &RuntimeFault) -> String {
    let (line, column) = source.line_col(fault.span.start()).unwrap_or((1, 1));
    format!(
        "{}:{line}:{column}: runtime[{}]: {}\n",
        path.display(),
        fault_name(fault.kind),
        fault_message(fault.kind)
    )
}

const fn fault_name(kind: RuntimeFaultKind) -> &'static str {
    match kind {
        RuntimeFaultKind::Arithmetic => "ArithmeticFault",
        RuntimeFaultKind::DivisionByZero => "DivisionByZeroFault",
        RuntimeFaultKind::Shift => "ShiftFault",
        RuntimeFaultKind::Allocation => "AllocationFault",
        RuntimeFaultKind::Capacity => "CapacityFault",
        RuntimeFaultKind::Bounds => "BoundsFault",
    }
}

const fn fault_message(kind: RuntimeFaultKind) -> &'static str {
    match kind {
        RuntimeFaultKind::Arithmetic => "checked integer arithmetic overflow",
        RuntimeFaultKind::DivisionByZero => "integer division or remainder by zero",
        RuntimeFaultKind::Shift => "invalid integer shift amount",
        RuntimeFaultKind::Allocation => "runtime allocation failed",
        RuntimeFaultKind::Capacity => "requested list capacity is impossible",
        RuntimeFaultKind::Bounds => "list index is out of bounds",
    }
}

fn fixture(name: &str) -> (PathBuf, String) {
    let path = workspace_root()
        .join("crates/keld-cli/tests/fixtures")
        .join(name);
    let source = std::fs::read_to_string(&path).expect("fixture source");
    (path, source)
}

fn assert_observation(
    observation: &Observation,
    native: &NativeRun,
    path: &Path,
    source: &SourceText,
) {
    match observation {
        Observation::Returned(value) => {
            assert_eq!(native.output.status.code(), Some(0));
            assert_eq!(native.output.stdout, format!("{value}\n").as_bytes());
            assert!(native.output.stderr.is_empty());
        }
        Observation::Fault { kind, span } => {
            let fault = RuntimeFault {
                kind: *kind,
                span: *span,
            };
            assert_eq!(native.output.status.code(), Some(2));
            assert!(native.output.stdout.is_empty());
            assert_eq!(
                native.output.stderr,
                expected_stderr(path, source, &fault).as_bytes()
            );
        }
        Observation::Internal(error) => panic!("interpreter observation is internal: {error}"),
    }
    assert_ne!(
        native.output.status.code(),
        Some(70),
        "native internal failure"
    );
}

fn run_differential_case(label: &str, path: &Path, source: &str) {
    let (module, source_text) = compile_fixture(source);
    run_differential_module(label, path, &module, &source_text);
}

fn discover_failure_schedules(module: &Module) -> Vec<Vec<Failure>> {
    let mut discovered = BTreeSet::new();
    let mut queued = vec![Vec::new()];
    let mut queued_set = BTreeSet::new();
    queued_set.insert(Vec::<Failure>::new());
    let mut schedules = BTreeSet::new();
    while let Some(schedule) = queued.pop() {
        if !schedule.is_empty() {
            schedules.insert(schedule.clone());
        }
        let (_, events) = interpreter_run(module, &schedule);
        for event in events {
            let failure = (event.site_id, event.phase, event.attempt);
            if !discovered.insert(failure) || schedule.contains(&failure) {
                continue;
            }
            let mut next = schedule.clone();
            next.push(failure);
            next.sort();
            if queued_set.insert(next.clone()) {
                queued.push(next);
            }
        }
    }
    let mut result = vec![Vec::new()];
    result.extend(schedules);
    result
}

fn run_differential_module(label: &str, path: &Path, module: &Module, source_text: &SourceText) {
    let (baseline, interpreter_events) = interpreter_run(module, &[]);
    assert!(!matches!(baseline, Observation::Internal(_)), "{label}");
    let schedules = discover_failure_schedules(module);
    let directory = temp_dir(label);
    for optimization in [OptimizationLevel::O0, OptimizationLevel::O2] {
        let executable = build_native(module, path, source_text, optimization, &directory);
        let native_baseline = native_run(&executable, &directory, &[]);
        assert_observation(&baseline, &native_baseline, path, source_text);
        assert_eq!(
            native_baseline.events, interpreter_events,
            "baseline {label}"
        );
        for failures in schedules.iter().skip(1) {
            let (interpreter, interpreter_events) = interpreter_run(module, failures);
            assert!(
                !matches!(interpreter, Observation::Internal(_)),
                "{label} internal"
            );
            let native = native_run(&executable, &directory, failures);
            assert_observation(&interpreter, &native, path, source_text);
            assert_eq!(
                native.events, interpreter_events,
                "failure {label} {failures:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(directory);
}

fn ir_span() -> keld_source::Span {
    keld_source::Span::new(SourceId(0), 0, 0).expect("empty IR fixture span")
}

fn ir_function(
    register_types: Vec<keld_ir::IrType>,
    register_storage: Vec<keld_ir::RegisterStorage>,
    return_type: keld_ir::IrType,
    blocks: Vec<keld_ir::IrBlock>,
) -> keld_ir::Function {
    keld_ir::Function {
        id: keld_semantics::FunctionId(0),
        span: ir_span(),
        parameters: Vec::new(),
        parameter_modes: Vec::new(),
        parameter_effects: Vec::new(),
        current_lifecycle: keld_ir::Register(0),
        register_types,
        register_storage,
        storage_scope_parents: vec![None],
        return_type,
        blocks,
        entry: keld_ir::IrBlockId(0),
    }
}

fn ir_block(
    id: u32,
    instructions: Vec<keld_ir::Instruction>,
    terminator: keld_ir::Terminator,
) -> keld_ir::IrBlock {
    keld_ir::IrBlock {
        id: keld_ir::IrBlockId(id),
        instructions,
        terminator,
    }
}

fn take_fixture() -> Module {
    let span = ir_span();
    let register_types = vec![
        keld_ir::IrType::Lifecycle,
        keld_ir::IrType::Text,
        keld_ir::IrType::Text,
        keld_ir::IrType::Int,
    ];
    let register_storage = vec![
        keld_ir::RegisterStorage::Trivial,
        keld_ir::RegisterStorage::Home {
            scope: keld_flow::StorageScopeId(0),
            conditional: false,
        },
        keld_ir::RegisterStorage::Home {
            scope: keld_flow::StorageScopeId(0),
            conditional: false,
        },
        keld_ir::RegisterStorage::Trivial,
    ];
    Module {
        definitions: Vec::new(),
        functions: vec![ir_function(
            register_types,
            register_storage,
            keld_ir::IrType::Int,
            vec![ir_block(
                0,
                vec![
                    keld_ir::Instruction::ConstText {
                        dst: keld_ir::Register(1),
                        value: "take".to_owned(),
                        span,
                    },
                    keld_ir::Instruction::Take {
                        dst: keld_ir::Register(2),
                        src: keld_ir::Register(1),
                        span,
                    },
                    keld_ir::Instruction::DropIfLive {
                        home: keld_ir::Register(2),
                        span,
                    },
                    keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(3),
                        value: 17,
                        span,
                    },
                ],
                keld_ir::Terminator::Return(Some(keld_ir::Register(3))),
            )],
        )],
        main: keld_semantics::FunctionId(0),
    }
}

fn cleanup_fixture() -> Module {
    let span = ir_span();
    Module {
        definitions: Vec::new(),
        functions: vec![ir_function(
            vec![
                keld_ir::IrType::Lifecycle,
                keld_ir::IrType::Text,
                keld_ir::IrType::Int,
            ],
            vec![
                keld_ir::RegisterStorage::Trivial,
                keld_ir::RegisterStorage::Home {
                    scope: keld_flow::StorageScopeId(0),
                    conditional: false,
                },
                keld_ir::RegisterStorage::Trivial,
            ],
            keld_ir::IrType::Int,
            vec![ir_block(
                0,
                vec![
                    keld_ir::Instruction::ConstText {
                        dst: keld_ir::Register(1),
                        value: "cleanup".to_owned(),
                        span,
                    },
                    keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(2),
                        value: 23,
                        span,
                    },
                    keld_ir::Instruction::CleanupTrackedScope {
                        scope: keld_flow::StorageScopeId(0),
                        span,
                    },
                ],
                keld_ir::Terminator::Return(Some(keld_ir::Register(2))),
            )],
        )],
        main: keld_semantics::FunctionId(0),
    }
}

fn direct_list_fixture() -> Module {
    let span = ir_span();
    Module {
        definitions: Vec::new(),
        functions: vec![ir_function(
            vec![
                keld_ir::IrType::Lifecycle,
                keld_ir::IrType::List(Box::new(keld_ir::IrType::Int)),
                keld_ir::IrType::Int,
                keld_ir::IrType::Int,
                keld_ir::IrType::Int,
            ],
            vec![
                keld_ir::RegisterStorage::Trivial,
                keld_ir::RegisterStorage::Home {
                    scope: keld_flow::StorageScopeId(0),
                    conditional: false,
                },
                keld_ir::RegisterStorage::Trivial,
                keld_ir::RegisterStorage::Trivial,
                keld_ir::RegisterStorage::Trivial,
            ],
            keld_ir::IrType::Int,
            vec![ir_block(
                0,
                vec![
                    keld_ir::Instruction::ListNew {
                        dst: keld_ir::Register(1),
                        span,
                    },
                    keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(2),
                        value: 7,
                        span,
                    },
                    keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(3),
                        value: 0,
                        span,
                    },
                    keld_ir::Instruction::ListPush {
                        list: keld_ir::Register(1),
                        value: keld_ir::Register(2),
                        span,
                    },
                    keld_ir::Instruction::ListRemove {
                        dst: keld_ir::Register(4),
                        list: keld_ir::Register(1),
                        index: keld_ir::Register(3),
                        span,
                    },
                    keld_ir::Instruction::DropHome {
                        home: keld_ir::Register(1),
                        span,
                    },
                ],
                keld_ir::Terminator::Return(Some(keld_ir::Register(4))),
            )],
        )],
        main: keld_semantics::FunctionId(0),
    }
}

fn phi_fixture() -> Module {
    let span = ir_span();
    Module {
        definitions: Vec::new(),
        functions: vec![ir_function(
            vec![
                keld_ir::IrType::Lifecycle,
                keld_ir::IrType::Bool,
                keld_ir::IrType::Int,
                keld_ir::IrType::Int,
                keld_ir::IrType::Int,
            ],
            vec![keld_ir::RegisterStorage::Trivial; 5],
            keld_ir::IrType::Int,
            vec![
                ir_block(
                    0,
                    vec![keld_ir::Instruction::ConstBool {
                        dst: keld_ir::Register(1),
                        value: true,
                        span,
                    }],
                    keld_ir::Terminator::Branch {
                        condition: keld_ir::Register(1),
                        then_block: keld_ir::IrBlockId(1),
                        else_block: keld_ir::IrBlockId(2),
                    },
                ),
                ir_block(
                    1,
                    vec![keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(2),
                        value: 31,
                        span,
                    }],
                    keld_ir::Terminator::Goto(keld_ir::IrBlockId(3)),
                ),
                ir_block(
                    2,
                    vec![keld_ir::Instruction::ConstInt {
                        dst: keld_ir::Register(3),
                        value: 41,
                        span,
                    }],
                    keld_ir::Terminator::Goto(keld_ir::IrBlockId(3)),
                ),
                ir_block(
                    3,
                    vec![keld_ir::Instruction::Phi {
                        dst: keld_ir::Register(4),
                        inputs: vec![
                            (keld_ir::IrBlockId(1), keld_ir::Register(2)),
                            (keld_ir::IrBlockId(2), keld_ir::Register(3)),
                        ],
                        span,
                    }],
                    keld_ir::Terminator::Return(Some(keld_ir::Register(4))),
                ),
            ],
        )],
        main: keld_semantics::FunctionId(0),
    }
}

fn fault_fixture() -> Module {
    let span = keld_source::Span::new(SourceId(0), 0, 1).expect("fault fixture span");
    Module {
        definitions: Vec::new(),
        functions: vec![ir_function(
            vec![keld_ir::IrType::Lifecycle, keld_ir::IrType::Int],
            vec![keld_ir::RegisterStorage::Trivial; 2],
            keld_ir::IrType::Int,
            vec![ir_block(
                0,
                Vec::new(),
                keld_ir::Terminator::Fault {
                    kind: keld_ir::FaultKind::Allocation,
                    span,
                },
            )],
        )],
        main: keld_semantics::FunctionId(0),
    }
}

fn ir_surface_fixtures() -> Vec<(&'static str, Module)> {
    vec![
        ("take_cleanup_ir", take_fixture()),
        ("cleanup_scope_ir", cleanup_fixture()),
        ("direct_list_ir", direct_list_fixture()),
        ("phi_ir", phi_fixture()),
        ("fault_ir", fault_fixture()),
    ]
}

fn ir_metadata(label: &str) -> (PathBuf, SourceText) {
    (
        PathBuf::from(format!("{label}.keld")),
        SourceText::from_str(SourceId(0), "x\n").expect("IR fixture source"),
    )
}

fn instruction_variant(instruction: &keld_ir::Instruction) -> &'static str {
    use keld_ir::Instruction::*;
    match instruction {
        ConstInt { .. } => "ConstInt",
        ConstBool { .. } => "ConstBool",
        ConstText { .. } => "ConstText",
        ConstNoneLink { .. } => "ConstNoneLink",
        Copy { .. } => "Copy",
        Take { .. } => "Take",
        InstallHome { .. } => "InstallHome",
        MoveHome { .. } => "MoveHome",
        DropHome { .. } => "DropHome",
        DropIfLive { .. } => "DropIfLive",
        DropSlot { .. } => "DropSlot",
        CleanupTrackedScope { .. } => "CleanupTrackedScope",
        ReplacePlace { .. } => "ReplacePlace",
        ReplaceField { .. } => "ReplaceField",
        ListNew { .. } => "ListNew",
        ListLength { .. } => "ListLength",
        ListPush { .. } => "ListPush",
        ListPushPlace { .. } => "ListPushPlace",
        ListRemove { .. } => "ListRemove",
        ListRemovePlace { .. } => "ListRemovePlace",
        ListIndex { .. } => "ListIndex",
        ListGet { .. } => "ListGet",
        ListReplace { .. } => "ListReplace",
        ListTryRemove { .. } => "ListTryRemove",
        ListClear { .. } => "ListClear",
        ListReserve { .. } => "ListReserve",
        ListTryReserve { .. } => "ListTryReserve",
        TextByteLength { .. } => "TextByteLength",
        TextIsEmpty { .. } => "TextIsEmpty",
        TextConcat { .. } => "TextConcat",
        CheckedUnaryInt { .. } => "CheckedUnaryInt",
        CheckedBinaryInt { .. } => "CheckedBinaryInt",
        Not { .. } => "Not",
        Compare { .. } => "Compare",
        Phi { .. } => "Phi",
        ConstructStruct { .. } => "ConstructStruct",
        ReadStructField { .. } => "ReadStructField",
        BeginLifecycle { .. } => "BeginLifecycle",
        EndLifecycle { .. } => "EndLifecycle",
        AllocateEntity { .. } => "AllocateEntity",
        EntityToLink { .. } => "EntityToLink",
        OpenView { .. } => "OpenView",
        ReadField { .. } => "ReadField",
        WriteField { .. } => "WriteField",
        CloseView { .. } => "CloseView",
        KeepEntity { .. } => "KeepEntity",
        RetireEntity { .. } => "RetireEntity",
        Call { .. } => "Call",
    }
}

fn terminator_variant(terminator: &keld_ir::Terminator) -> &'static str {
    use keld_ir::Terminator::*;
    match terminator {
        Goto(_) => "Goto",
        Branch { .. } => "Branch",
        ResolveLink { .. } => "ResolveLink",
        Return(_) => "Return",
        Fault { .. } => "Fault",
        Unreachable => "Unreachable",
    }
}

fn module_surface(module: &Module) -> (BTreeSet<&'static str>, BTreeSet<&'static str>) {
    let mut instructions = BTreeSet::new();
    let mut terminators = BTreeSet::new();
    for function in &module.functions {
        for block in &function.blocks {
            instructions.extend(block.instructions.iter().map(instruction_variant));
            terminators.insert(terminator_variant(&block.terminator));
        }
    }
    (instructions, terminators)
}

#[test]
fn every_source_fixture_has_a_shared_allocation_failure_schedule_at_o0_and_o2() {
    for fixture_name in [
        "cyclic_graph.keld",
        "keep_survives.keld",
        "stale_link.keld",
        "alias_distinct.keld",
        "broad_retirement.keld",
        "numeric_edges.keld",
        "runtime_div_zero.keld",
        "runtime_capacity.keld",
    ] {
        let (path, source) = fixture(fixture_name);
        run_differential_case(fixture_name.trim_end_matches(".keld"), &path, &source);
    }
}

#[test]
fn source_surface_fixtures_extend_the_same_differential_schedule() {
    for (label, source) in source_surface_cases() {
        let path = PathBuf::from(format!("{label}.keld"));
        run_differential_case(label, &path, source);
    }
}

fn source_surface_cases() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "text_surface",
            "fn main() -> Int { let value = \"K\" + \"한\"; if value == \"K한\" { if value.is_empty { return 0 } else { return value.byte_length } } else { return 0 } }\n",
        ),
        (
            "list_surface",
            "fn main() -> Int { let items: List[Int] = List(); items.reserve(3); items.push(7); let copied = items[0]; let _optional = items.get(0); items[0] = 8; let removed = items.remove(0); items.push(9); let ok = items.try_reserve(1); let _maybe_removed = items.try_remove(0); items.clear(); if ok { return copied + removed } else { return copied + removed } }\n",
        ),
        (
            "nested_projected_surface",
            "fn main() -> Int { let inner: List[Int] = List(); inner.push(1); let outer: List[List[Int]] = List(); outer.push(take inner); outer[0][0] = 7; outer[0].push(8); let removed = outer[0].remove(0); return removed + outer[0][0] }\n",
        ),
        (
            "struct_surface",
            "struct Holder { items: List[Int] }\nfn main() -> Int { var holder = Holder(items: List()); holder.items.push(1); let removed = holder.items.remove(0); return removed + holder.items.length }\n",
        ),
        (
            "entity_surface",
            "entity Holder { items: List[Int] }\nfn main() -> Int { lifecycle level { let holder = Holder(items: List()); holder.items.push(1); let copied = holder.items.copy(); let removed = holder.items.remove(0); return copied.length + removed + holder.items.length } }\n",
        ),
        (
            "call_surface",
            "fn consume(take items: List[Int]) -> Int { return items.length }\nfn unit(items: List[Int]) { items.push(2); return }\nfn main() -> Int { let items: List[Int] = List(); items.push(1); unit(items); return consume(take items) }\n",
        ),
        (
            "take_surface",
            "fn forward(take value: Text) -> Text { let copied = value.copy(); let moved = take value; return take moved }\nfn main() -> Int { let value: Text = \"Keld\"; let result = forward(take value); return result.byte_length }\n",
        ),
        (
            "replace_field_surface",
            "entity Holder { value: Text }\nfn main() -> Int { lifecycle level { let holder = Holder(value: \"old\"); let new: Text = \"new\"; holder.value = take new; return holder.value.byte_length } }\n",
        ),
        (
            "replace_place_surface",
            "struct Holder { value: Text }\nfn main() -> Int { var holder = Holder(value: \"old\"); let new: Text = \"new\"; holder.value = take new; return holder.value.byte_length }\n",
        ),
        (
            "not_surface",
            "fn main() -> Int { let value = !true; if value { return 1 } else { return 0 } }\n",
        ),
        (
            "normal_managed_list_parameter",
            "fn append(items: List[Int]) -> Int { let copied = items.copy(); let outer: List[List[Int]] = List(); outer.push(take copied); return outer[0].length }\nfn main() -> Int { let items: List[Int] = List(); items.push(1); return append(items) }\n",
        ),
        (
            "normal_managed_struct_parameter",
            "struct Holder { items: List[Int] }\nfn wrap(holder: Holder) -> Int { let copied = holder.items.copy(); let other = Holder(items: take copied); return other.items.length }\nfn main() -> Int { let items: List[Int] = List(); items.push(1); let holder = Holder(items: take items); return wrap(holder) }\n",
        ),
    ]
}

#[test]
fn ir_only_surface_fixtures_extend_the_same_differential_schedule() {
    for (label, module) in ir_surface_fixtures() {
        let diagnostics = keld_ir::validate(&module);
        assert!(
            diagnostics.is_empty(),
            "invalid fixture {label}: {diagnostics:?}"
        );
        let (path, source) = ir_metadata(label);
        run_differential_module(label, &path, &module, &source);
    }
}

#[test]
fn every_executable_ir_variant_is_in_a_real_differential_fixture() {
    const INSTRUCTIONS: &[&str] = &[
        "ConstInt",
        "ConstBool",
        "ConstText",
        "ConstNoneLink",
        "Copy",
        "Take",
        "InstallHome",
        "MoveHome",
        "DropHome",
        "DropIfLive",
        "DropSlot",
        "CleanupTrackedScope",
        "ReplacePlace",
        "ReplaceField",
        "ListNew",
        "ListLength",
        "ListPush",
        "ListPushPlace",
        "ListRemove",
        "ListRemovePlace",
        "ListIndex",
        "ListGet",
        "ListReplace",
        "ListTryRemove",
        "ListClear",
        "ListReserve",
        "ListTryReserve",
        "TextByteLength",
        "TextIsEmpty",
        "TextConcat",
        "CheckedUnaryInt",
        "CheckedBinaryInt",
        "Not",
        "Compare",
        "Phi",
        "ConstructStruct",
        "ReadStructField",
        "BeginLifecycle",
        "EndLifecycle",
        "AllocateEntity",
        "EntityToLink",
        "OpenView",
        "ReadField",
        "WriteField",
        "CloseView",
        "KeepEntity",
        "RetireEntity",
        "Call",
    ];
    const TERMINATORS: &[&str] = &[
        "Goto",
        "Branch",
        "ResolveLink",
        "Return",
        "Fault",
        "Unreachable",
    ];
    let mut instructions = BTreeSet::new();
    let mut terminators = BTreeSet::new();
    for fixture_name in [
        "cyclic_graph.keld",
        "keep_survives.keld",
        "stale_link.keld",
        "alias_distinct.keld",
        "broad_retirement.keld",
        "numeric_edges.keld",
        "runtime_div_zero.keld",
        "runtime_capacity.keld",
    ] {
        let (_, source) = fixture(fixture_name);
        let (module, _) = compile_fixture(&source);
        let (fixture_instructions, fixture_terminators) = module_surface(&module);
        instructions.extend(fixture_instructions);
        terminators.extend(fixture_terminators);
    }
    for (_, source) in source_surface_cases() {
        let (module, _) = compile_fixture(source);
        let (fixture_instructions, fixture_terminators) = module_surface(&module);
        instructions.extend(fixture_instructions);
        terminators.extend(fixture_terminators);
    }
    for (_, module) in ir_surface_fixtures() {
        let (fixture_instructions, fixture_terminators) = module_surface(&module);
        instructions.extend(fixture_instructions);
        terminators.extend(fixture_terminators);
    }
    for variant in INSTRUCTIONS {
        assert!(
            instructions.contains(variant),
            "missing executable fixture for {variant}"
        );
    }
    for variant in TERMINATORS {
        assert!(
            terminators.contains(variant),
            "missing executable fixture for {variant}"
        );
    }
}

#[test]
fn allocation_schedule_covers_preferred_and_exact_list_growth_failures() {
    let (_, source) = source_surface_cases()
        .iter()
        .find(|(label, _)| *label == "list_surface")
        .expect("list surface fixture");
    let (module, _) = compile_fixture(source);
    let schedules = discover_failure_schedules(&module);
    assert!(schedules.iter().any(|schedule| {
        let preferred = schedule
            .iter()
            .find(|(_, phase, _)| *phase == AllocationPhase::ListGrowthPreferred);
        let exact = schedule
            .iter()
            .find(|(_, phase, _)| *phase == AllocationPhase::ListGrowthExact);
        preferred.is_some_and(|(preferred_site, _, _)| {
            exact.is_some_and(|(exact_site, _, _)| exact_site != preferred_site)
        })
    }));
}

#[test]
fn allocation_schedule_includes_context_setup_failure() {
    let (path, source) = fixture("numeric_edges.keld");
    let (module, _) = compile_fixture(&source);
    let schedule = AllocationSchedule::from_module(&module);
    let main = module
        .functions
        .iter()
        .find(|function| function.id == module.main)
        .expect("main function");
    let base = schedule
        .base_id(main.id, main.entry, 0)
        .expect("context coordinate");
    let failure = (
        schedule.site_id(base, AllocationPhase::Context, 0),
        AllocationPhase::Context,
        1,
    );
    assert!(
        discover_failure_schedules(&module)
            .iter()
            .any(|candidate| candidate == &vec![failure]),
        "{path:?} must include context setup failure"
    );
}

const RUNTIME_EXPORTS: &str = "LIBRARY keld_runtime_v1.dll\nEXPORTS\nkeld_rt_v1_abi_version\nkeld_rt_v1_print_int\nkeld_rt_v1_print_fault\nkeld_rt_v1_context_new\nkeld_rt_v1_context_new_at\nkeld_rt_v1_test_site\nkeld_rt_v1_context_destroy\nkeld_rt_v1_context_status\nkeld_rt_v1_context_fault\nkeld_rt_v1_context_fault_parts\nkeld_rt_v1_context_root_lifecycle\nkeld_rt_v1_value_copy\nkeld_rt_v1_value_drop\nkeld_rt_v1_text_new\nkeld_rt_v1_text_byte_length\nkeld_rt_v1_text_is_empty\nkeld_rt_v1_text_equal\nkeld_rt_v1_text_concat\nkeld_rt_v1_list_new\nkeld_rt_v1_list_length\nkeld_rt_v1_list_push\nkeld_rt_v1_list_get\nkeld_rt_v1_list_remove\nkeld_rt_v1_list_replace\nkeld_rt_v1_list_try_remove\nkeld_rt_v1_list_clear\nkeld_rt_v1_list_reserve\nkeld_rt_v1_list_try_reserve\nkeld_rt_v1_struct_new\nkeld_rt_v1_struct_field\nkeld_rt_v1_place_resolve\nkeld_rt_v1_place_replace\nkeld_rt_v1_home_track\nkeld_rt_v1_home_untrack\nkeld_rt_v1_cleanup_scope\nkeld_rt_v1_begin_lifecycle\nkeld_rt_v1_end_lifecycle\nkeld_rt_v1_allocate_entity\nkeld_rt_v1_entity_to_link\nkeld_rt_v1_resolve_link\nkeld_rt_v1_entity_field\nkeld_rt_v1_replace_field\nkeld_rt_v1_keep_entity\nkeld_rt_v1_retire_entity\n";
