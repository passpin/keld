from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
old = '''    fn finish(mut self) -> (String, BTreeMap<String, Vec<u8>>) {
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
        (self.lines.join("\\n"), self.text_literals)
    }
'''
new = '''    fn finish(mut self) -> (String, BTreeMap<String, Vec<u8>>) {
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

if new in text:
    raise SystemExit(0)
if old not in text:
    raise SystemExit("expected ScalarLowerer::finish snippet not found")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
