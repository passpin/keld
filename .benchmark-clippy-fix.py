from pathlib import Path

path = Path("crates/keld-native-backend/tests/language_runtime_compare.rs")
text = path.read_text(encoding="utf-8")
text = text.replace('const KELD_SCALAR: &str = r#"', 'const KELD_SCALAR: &str = r"', 1)
text = text.replace('\n    return acc\n}\n"#;\n\nconst KELD_TEXT', '\n    return acc\n}\n";\n\nconst KELD_TEXT', 1)
needle = '#[test]\n#[ignore = "manual cross-language runtime benchmark on Windows"]\nfn compare_keld_with_common_languages()'
replacement = '#[allow(clippy::too_many_lines)]\n#[test]\n#[ignore = "manual cross-language runtime benchmark on Windows"]\nfn compare_keld_with_common_languages()'
if needle not in text:
    raise SystemExit("benchmark driver marker not found")
text = text.replace(needle, replacement, 1)
path.write_text(text, encoding="utf-8")
