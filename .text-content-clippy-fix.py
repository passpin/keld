from pathlib import Path

path = Path("crates/keld-native-backend/tests/language_text_content_compare.rs")
text = path.read_text(encoding="utf-8")
for name in ("CPP_SOURCE", "JAVA_SOURCE", "PYTHON_SOURCE", "SWIFT_SOURCE"):
    marker = f'const {name}: &str = r#"'
    if marker not in text:
        raise SystemExit(f"missing start marker for {name}")
    start = text.index(marker)
    body_start = start + len(marker)
    end = text.index('\n"#;', body_start)
    body = text[body_start:end]
    if '"' in body:
        raise SystemExit(f"{name} contains a quote and cannot use r\"...\"")
    text = text[:start] + f'const {name}: &str = r"' + body + '\n";' + text[end + len('\n"#;'):]
path.write_text(text, encoding="utf-8")
