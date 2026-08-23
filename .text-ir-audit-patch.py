from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
marker = '''/// Small source-independent helper used by focused adapter tests.
#[must_use]
pub const fn accepts_validated_module(_module: &Module) -> bool {
    true
}
'''
if marker not in text:
    raise SystemExit("lib.rs tail marker not found")
addition = r'''

#[cfg(test)]
mod text_ir_audit {
    use super::*;
    use keld_source::{SourceId, SourceText};

    #[test]
    fn text_content_loop_reports_runtime_call_surface() {
        let source = r#"
fn main() -> Int {
    let a = "abcdefghijklmnopqrstuvwxyz"
    let b = "!"
    let expected = "abcdefghijklmnopqrstuvwxyz!"
    var i = 0
    var count = 0
    while i < 1000000 {
        if (a + b) == expected {
            count += 1
        } else {
            count += 7
        }
        i += 1
    }
    return count + i
}
"#;
        let source_text = SourceText::from_str(SourceId(0), source).expect("source text");
        let flow = keld_flow::lower_text_for_test(source).expect("flow lowering");
        let lifecycle = keld_lifecycle::verify(flow)
            .module
            .expect("lifecycle verification");
        let storage = keld_storage::verify(lifecycle)
            .module
            .expect("storage verification");
        let module = keld_ir::lower(&storage);
        assert!(keld_ir::validate(&module).is_empty());
        let ir = lower_scalar_ir(
            &module,
            &SourceMetadata {
                path: PathBuf::from("text-content-audit.keld"),
                source: source_text,
            },
        )
        .expect("LLVM IR lowering");

        let symbols = [
            "keld_rt_v1_text_new",
            "keld_rt_v1_text_concat",
            "keld_rt_v1_text_equal",
            "keld_rt_v1_home_track",
            "keld_rt_v1_home_untrack",
            "keld_rt_v1_cleanup_scope",
            "keld_rt_v1_value_drop",
            "keld_rt_v1_test_site",
        ];
        for symbol in symbols {
            let needle = format!("call i32 @{symbol}");
            println!("TEXTIR symbol={symbol} static_calls={}", ir.matches(&needle).count());
        }

        assert_eq!(ir.matches("call i32 @keld_rt_v1_text_concat").count(), 1);
        assert_eq!(ir.matches("call i32 @keld_rt_v1_text_equal").count(), 1);
    }
}
'''
text = text.replace(marker, marker + addition, 1)
path.write_text(text, encoding="utf-8")
'''
