from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
marker = "#[cfg(test)]\nmod text_ir_audit {"
start = text.find(marker)
if start < 0:
    raise SystemExit("text_ir_audit module not found")
replacement = r'''#[cfg(test)]
mod text_ir_audit {
    use super::*;
    use keld_ir::TestModuleBuilder;
    use keld_source::SourceId;

    #[test]
    fn production_ir_has_no_test_site_calls() {
        let module = TestModuleBuilder::new().finish();
        assert!(validate(&module).is_empty());
        let source = SourceText::from_str(SourceId(0), "test-site audit").expect("source");
        let ir = lower_scalar_ir(
            &module,
            &SourceMetadata {
                path: PathBuf::from("test-site-audit.keld"),
                source,
            },
        )
        .expect("LLVM IR lowering");
        assert_eq!(
            ir.matches("call i32 @keld_rt_v1_test_site").count(),
            0,
            "production codegen must not cross the test-control ABI"
        );
    }
}
'''
path.write_text(text[:start] + replacement, encoding="utf-8")
