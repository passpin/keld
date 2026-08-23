from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
marker = "LLVM allocas execute dynamically at their insertion point"
if marker in text:
    print("native alloca hoist already present")
    raise SystemExit(0)

start_marker = "    fn finish(mut self) -> (String, BTreeMap<String, Vec<u8>>) {\n"
end_marker = "    }\n}\n\n#[allow(clippy::too_many_lines)]\nfn lower_scalar_ir"
start = text.find(start_marker)
end = text.find(end_marker, start)
if start < 0 or end < 0:
    raise SystemExit(f"finish boundaries not found: start={start} end={end}")

replacement = '''    fn finish(mut self) -> (String, BTreeMap<String, Vec<u8>>) {
        let faults = self.faults.iter().copied().collect::<Vec<_>>();
        for (kind, location) in faults {
            self.lines.push(format!("fault_{kind}_{location}:"));
            self.line(format!("store i32 {kind}, ptr %out_kind"));
            self.line(format!("store i32 {location}, ptr %out_location"));
            self.line("br label %fault_exit");
        }
        self.lines.push("fault_exit:".to_owned());
        self.line("ret i32 1");
        self.lines.push("internal_exit:".to_owned());
        self.line("ret i32 2");

        // LLVM allocas execute dynamically at their insertion point. Scratch slots emitted
        // inside a CFG loop would therefore grow the native stack on every back-edge.
        // All slot sizes are static, so allocate the complete frame once in the entry block
        // and keep only the loads/stores/calls at their original program points.
        let mut allocas = Vec::new();
        self.lines.retain(|line| {
            if line.contains(" = alloca ") {
                allocas.push(line.clone());
                false
            } else {
                true
            }
        });
        let insertion = self.lines.first().map_or(0, |_| 1);
        self.lines.splice(insertion..insertion, allocas);

        (self.lines.join("\\n"), self.text_literals)
    }
'''

# Preserve the impl-closing brace and lower_scalar_ir marker that follow finish.
text = text[:start] + replacement + text[end:]
path.write_text(text, encoding="utf-8")
print("native alloca hoist applied")
