from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text()

# Give scalar source locals stable addressable storage without changing SSA temp names.
old = r'''fn managed_slot_name(function: &keld_ir::Function, register: Register) -> String {
    if function.parameters.contains(&register)
        && function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    {
        register_name(register)
    } else {
        format!("%slot{}", register.0)
    }
}
'''
new = r'''fn managed_slot_name(function: &keld_ir::Function, register: Register) -> String {
    if function.parameters.contains(&register)
        && function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    {
        register_name(register)
    } else {
        format!("%slot{}", register.0)
    }
}

fn scalar_local_slot_name(register: Register) -> String {
    format!("%local{}", register.0)
}
'''
if text.count(old) != 1:
    raise RuntimeError("managed_slot_name insertion point changed")
text = text.replace(old, new, 1)

# Add a helper matching the IR's explicit local-slot table.
old = r'''    fn is_managed_register(&self, register: Register) -> bool {
        self.function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    }

    fn is_home_register(&self, register: Register) -> bool {
'''
new = r'''    fn is_managed_register(&self, register: Register) -> bool {
        self.function
            .register_types
            .get(register.0 as usize)
            .is_some_and(is_managed_type)
    }

    fn is_scalar_local(&self, register: Register) -> bool {
        self.function.locals.contains(&register)
            && self
                .function
                .register_types
                .get(register.0 as usize)
                .and_then(scalar_type)
                .is_some()
    }

    fn is_home_register(&self, register: Register) -> bool {
'''
if text.count(old) != 1:
    raise RuntimeError("ScalarLowerer helper insertion point changed")
text = text.replace(old, new, 1)

# Allocate scalar local slots at function entry. Scalar parameters keep their
# incoming SSA argument name but are immediately installed into the source-local slot.
old = r'''    fn emit_managed_slots(&mut self) {
        self.slots_emitted = true;
        for (index, ty) in self.function.register_types.iter().enumerate() {
'''
new = r'''    fn emit_managed_slots(&mut self) {
        self.slots_emitted = true;
        for local in &self.function.locals {
            let Some(ty) = self
                .function
                .register_types
                .get(local.0 as usize)
                .and_then(scalar_type)
            else {
                continue;
            };
            let slot = scalar_local_slot_name(*local);
            self.line(format!("{slot} = alloca {ty}"));
            if self.function.parameters.contains(local) {
                self.line(format!(
                    "store {ty} {}, ptr {slot}",
                    register_name(*local)
                ));
            }
        }
        for (index, ty) in self.function.register_types.iter().enumerate() {
'''
if text.count(old) != 1:
    raise RuntimeError("emit_managed_slots prologue changed")
text = text.replace(old, new, 1)

# Scalar Copy is the executable IR boundary for source-local reads/writes.
old = r'''                } else {
                    let ty = self.value_type(*src)?;
                    if ty == "i64" {
                        self.line(format!(
                            "{} = add i64 0, {}",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    } else if ty == "i1" {
                        self.line(format!(
                            "{} = xor i1 {}, false",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    } else {
                        self.line(format!(
                            "{} = select i1 true, {ty} {}, {ty} zeroinitializer",
                            register_name(*dst),
                            register_name(*src)
                        ));
                    }
                }
            }
'''
new = r'''                } else {
                    let ty = self.value_type(*src)?;
                    let source = if self.is_scalar_local(*src) {
                        let loaded = format!("%local_load_{}_{}", src.0, self.lines.len());
                        self.line(format!(
                            "{loaded} = load {ty}, ptr {}",
                            scalar_local_slot_name(*src)
                        ));
                        loaded
                    } else {
                        register_name(*src)
                    };
                    if self.is_scalar_local(*dst) {
                        self.line(format!(
                            "store {ty} {source}, ptr {}",
                            scalar_local_slot_name(*dst)
                        ));
                    } else if ty == "i64" {
                        self.line(format!(
                            "{} = add i64 0, {source}",
                            register_name(*dst)
                        ));
                    } else if ty == "i1" {
                        self.line(format!(
                            "{} = xor i1 {source}, false",
                            register_name(*dst)
                        ));
                    } else {
                        self.line(format!(
                            "{} = select i1 true, {ty} {source}, {ty} zeroinitializer",
                            register_name(*dst)
                        ));
                    }
                }
            }
'''
if text.count(old) != 1:
    raise RuntimeError("scalar Copy lowering changed")
text = text.replace(old, new, 1)

path.write_text(text)
print("lowered mutable scalar locals through entry allocas and Copy load/store")