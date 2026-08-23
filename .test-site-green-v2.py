from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        if new in text:
            return text
        raise SystemExit(f"missing marker: {label}")
    return text.replace(old, new, 1)

backend = Path("crates/keld-native-backend/src/lib.rs")
text = backend.read_text(encoding="utf-8")

# Remove the temporary source-level audit. The permanent regression lives in
# tests/production_test_site.rs and inspects the linked production executable.
audit_marker = "\n#[cfg(test)]\nmod text_ir_audit {"
audit_start = text.find(audit_marker)
if audit_start >= 0:
    text = text[:audit_start].rstrip() + "\n"

text = replace_once(
    text,
    "    allocation_schedule: &'module AllocationSchedule,\n    lines: Vec<String>,\n",
    "    allocation_schedule: &'module AllocationSchedule,\n    emit_test_sites: bool,\n    lines: Vec<String>,\n",
    "ScalarLowerer field",
)
text = replace_once(
    text,
    "        allocation_schedule: &'module AllocationSchedule,\n    ) -> Self {\n",
    "        allocation_schedule: &'module AllocationSchedule,\n        emit_test_sites: bool,\n    ) -> Self {\n",
    "ScalarLowerer constructor signature",
)
text = replace_once(
    text,
    "            allocation_schedule,\n            lines: Vec::new(),\n",
    "            allocation_schedule,\n            emit_test_sites,\n            lines: Vec::new(),\n",
    "ScalarLowerer constructor field",
)
text = replace_once(
    text,
    "        if self.instruction_allocates(instruction) {\n            self.emit_test_site(block, instruction_index)?;\n        }\n",
    "        if self.emit_test_sites && self.instruction_allocates(instruction) {\n            self.emit_test_site(block, instruction_index)?;\n        }\n",
    "instruction test-site guard",
)

old_lower = '''#[allow(clippy::too_many_lines)]
fn lower_scalar_ir(module: &Module, metadata: &SourceMetadata) -> Result<String, BackendError> {
'''
new_lower = '''fn lower_scalar_ir(module: &Module, metadata: &SourceMetadata) -> Result<String, BackendError> {
    lower_scalar_ir_inner(module, metadata, false)
}

fn lower_scalar_ir_with_test_sites(
    module: &Module,
    metadata: &SourceMetadata,
) -> Result<String, BackendError> {
    lower_scalar_ir_inner(module, metadata, true)
}

#[allow(clippy::too_many_lines)]
fn lower_scalar_ir_inner(
    module: &Module,
    metadata: &SourceMetadata,
    emit_test_sites: bool,
) -> Result<String, BackendError> {
'''
text = replace_once(text, old_lower, new_lower, "lower_scalar_ir wrapper")
text = replace_once(
    text,
    "        let mut lowerer = ScalarLowerer::new(module, function, &locations, &allocation_schedule);\n",
    "        let mut lowerer = ScalarLowerer::new(\n            module,\n            function,\n            &locations,\n            &allocation_schedule,\n            emit_test_sites,\n        );\n",
    "ScalarLowerer construction",
)

old_decl = r'''    ir = ir.replace(
        "declare ptr @keld_rt_v1_context_new()\n",
        "declare ptr @keld_rt_v1_context_new()\ndeclare i32 @keld_rt_v1_test_site(ptr, i32)\n",
    );
'''
new_decl = r'''    if emit_test_sites {
        ir = ir.replace(
            "declare ptr @keld_rt_v1_context_new()\n",
            "declare ptr @keld_rt_v1_context_new()\ndeclare i32 @keld_rt_v1_test_site(ptr, i32)\n",
        );
    }
'''
text = replace_once(text, old_decl, new_decl, "test-site declaration")

old_context_marker = r'''    let context_status_with_site = format!(
        "  %context_site_status = call i32 @keld_rt_v1_test_site(ptr null, i32 {context_site})\n  %context_site_ok = icmp eq i32 %context_site_status, 0\n  br i1 %context_site_ok, label %context_site_cont, label %internal_main\ncontext_site_cont:\n{context_status_call}"
    );
    ir = ir.replace(&context_status_call, &context_status_with_site);
'''
new_context_marker = r'''    if emit_test_sites {
        let context_status_with_site = format!(
            "  %context_site_status = call i32 @keld_rt_v1_test_site(ptr null, i32 {context_site})\n  %context_site_ok = icmp eq i32 %context_site_status, 0\n  br i1 %context_site_ok, label %context_site_cont, label %internal_main\ncontext_site_cont:\n{context_status_call}"
        );
        ir = ir.replace(&context_status_call, &context_status_with_site);
    }
'''
text = replace_once(text, old_context_marker, new_context_marker, "context test-site marker")

old_build = '''#[allow(clippy::too_many_lines)]
pub fn build_executable(
    module: &Module,
    metadata: &SourceMetadata,
    request: &BuildRequest,
) -> Result<NativeArtifact, BackendError> {
'''
new_build = '''pub fn build_executable(
    module: &Module,
    metadata: &SourceMetadata,
    request: &BuildRequest,
) -> Result<NativeArtifact, BackendError> {
    build_executable_inner(module, metadata, request, false)
}

/// Test-control build path used only by differential/native runtime tests.
/// Production callers must use [`build_executable`].
///
/// # Errors
///
/// Returns the same validation, toolchain, lowering, linking, and filesystem
/// errors as [`build_executable`].
#[doc(hidden)]
pub fn build_executable_with_test_controls(
    module: &Module,
    metadata: &SourceMetadata,
    request: &BuildRequest,
) -> Result<NativeArtifact, BackendError> {
    build_executable_inner(module, metadata, request, true)
}

#[allow(clippy::too_many_lines)]
fn build_executable_inner(
    module: &Module,
    metadata: &SourceMetadata,
    request: &BuildRequest,
    emit_test_sites: bool,
) -> Result<NativeArtifact, BackendError> {
'''
text = replace_once(text, old_build, new_build, "build_executable split")
text = replace_once(
    text,
    "    let generated_ir = lower_scalar_ir(module, metadata)?;\n",
    "    let generated_ir = if emit_test_sites {\n        lower_scalar_ir_with_test_sites(module, metadata)?\n    } else {\n        lower_scalar_ir(module, metadata)?\n    };\n",
    "generated IR mode",
)
backend.write_text(text, encoding="utf-8")

# Differential tests always require deterministic allocation-site markers.
diff = Path("crates/keld-native-backend/tests/differential.rs")
text = diff.read_text(encoding="utf-8")
text = replace_once(
    text,
    "use keld_native_backend::{BuildRequest, OptimizationLevel, SourceMetadata, build_executable};\n",
    "use keld_native_backend::{\n    BuildRequest, OptimizationLevel, SourceMetadata, build_executable_with_test_controls,\n};\n",
    "differential import",
)
text = replace_once(
    text,
    "    build_executable(\n        module,\n",
    "    build_executable_with_test_controls(\n        module,\n",
    "differential build call",
)
diff.write_text(text, encoding="utf-8")

# Only the helper that links the test-control DLL opts into markers.
native = Path("crates/keld-native-backend/tests/native_int.rs")
text = native.read_text(encoding="utf-8")
text = replace_once(
    text,
    "    BackendError, BuildRequest, NativeArtifact, OptimizationLevel, SourceMetadata, build_executable,\n",
    "    BackendError, BuildRequest, NativeArtifact, OptimizationLevel, SourceMetadata, build_executable,\n    build_executable_with_test_controls,\n",
    "native_int import",
)
old_run = '''    let artifact = build_executable(
        module,
        metadata,
        &request(&output, &runtime_dll, &import_library, optimization),
    )
    .expect("native test-runtime build");
'''
new_run = '''    let artifact = build_executable_with_test_controls(
        module,
        metadata,
        &request(&output, &runtime_dll, &import_library, optimization),
    )
    .expect("native test-runtime build");
'''
text = replace_once(text, old_run, new_run, "native test-runtime build call")
native.write_text(text, encoding="utf-8")
