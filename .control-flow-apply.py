from pathlib import Path
import re

path = Path("crates/keld-native-backend/tests/native_int.rs")
text = path.read_text()
pattern = re.compile(r"(?m)^(\s*)parameters: ([^\n]+),\n\1parameter_modes:")
replacement = r"\1parameters: \2,\n\1locals: Vec::new(),\n\1parameter_modes:"
text, count = pattern.subn(replacement, text)
if count != 16:
    raise RuntimeError(f"expected 16 hand-built native Function literals, found {count}")
path.write_text(text)
print("annotated 16 native_int hand-built Functions with empty local-slot tables")