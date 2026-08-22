from pathlib import Path

# Expose executable local-slot metadata on IR functions.
path = Path("crates/keld-ir/src/module.rs")
text = path.read_text()
old = """    pub parameters: Vec<Register>,\n    pub parameter_modes: Vec<ParameterMode>,\n"""
new = """    pub parameters: Vec<Register>,\n    /// Registers that represent mutable source-level local slots rather than SSA temporaries.\n    pub locals: Vec<Register>,\n    pub parameter_modes: Vec<ParameterMode>,\n"""
if text.count(old) != 1:
    raise RuntimeError("Function local-slot metadata insertion point changed")
text = text.replace(old, new, 1)
old = """                parameters: self\n                    .parameters\n                    .into_iter()\n                    .map(|(register, _)| register)\n                    .collect(),\n                parameter_modes: vec![ParameterMode::Loan; parameter_count],\n"""
new = """                parameters: self\n                    .parameters\n                    .into_iter()\n                    .map(|(register, _)| register)\n                    .collect(),\n                locals: Vec::new(),\n                parameter_modes: vec![ParameterMode::Loan; parameter_count],\n"""
if text.count(old) != 1:
    raise RuntimeError("TestModuleBuilder Function insertion point changed")
path.write_text(text.replace(old, new, 1))

# Preserve the Flow local-register table in executable IR.
path = Path("crates/keld-ir/src/lower.rs")
text = path.read_text()
old = """                .map(|local| self.registers.local(*local))\n                .collect(),\n            parameter_modes: self.function.parameter_modes.clone(),\n"""
new = """                .map(|local| self.registers.local(*local))\n                .collect(),\n            locals: self.registers.locals.clone(),\n            parameter_modes: self.function.parameter_modes.clone(),\n"""
if text.count(old) != 1:
    raise RuntimeError("IR Function lowering insertion point changed")
path.write_text(text.replace(old, new, 1))

# Validate local metadata, but exempt only Copy writes into declared local slots
# from the SSA single-definition rule.
path = Path("crates/keld-ir/src/validate.rs")
text = path.read_text()
old = """        self.validate_parameter_storage_roles();\n        self.validate_register_storage_roles();\n"""
new = """        let mut locals = BTreeSet::new();\n        for local in &self.function.locals {\n            self.check_register(*local, self.function.span);\n            if !locals.insert(*local) {\n                self.sink.error(\n                    REGISTER_ERROR,\n                    self.function.span,\n                    \"function local register is duplicated\",\n                );\n            }\n        }\n        self.validate_parameter_storage_roles();\n        self.validate_register_storage_roles();\n"""
if text.count(old) != 1:
    raise RuntimeError("validator local metadata insertion point changed")
text = text.replace(old, new, 1)
old = """        for block in &self.function.blocks {\n            for instruction in &block.instructions {\n                if let Some(destination) = instruction_destination(instruction)\n                    && !defined.insert(destination)\n                {\n                    self.sink.error(\n                        REGISTER_ERROR,\n                        instruction_span(instruction),\n                        \"register is defined more than once\",\n                    );\n                }\n            }\n"""
new = """        for block in &self.function.blocks {\n            for instruction in &block.instructions {\n                if let Instruction::Copy { dst, .. } = instruction\n                    && self.function.locals.contains(dst)\n                {\n                    // Local slots are mutable storage. Copy into one is a write, not an SSA definition.\n                    defined.insert(*dst);\n                    continue;\n                }\n                if let Some(destination) = instruction_destination(instruction)\n                    && !defined.insert(destination)\n                {\n                    self.sink.error(\n                        REGISTER_ERROR,\n                        instruction_span(instruction),\n                        \"register is defined more than once\",\n                    );\n                }\n            }\n"""
if text.count(old) != 1:
    raise RuntimeError("unique-definition validator block changed")
path.write_text(text.replace(old, new, 1))

# Update the hand-built cycle and protect the ordinary SSA duplicate check.
path = Path("crates/keld-ir/tests/control_flow.rs")
text = path.read_text()
old = """            parameters: Vec::new(),\n            parameter_modes: Vec::new(),\n"""
new = """            parameters: Vec::new(),\n            locals: Vec::new(),\n            parameter_modes: Vec::new(),\n"""
if text.count(old) != 1:
    raise RuntimeError("hand-built cycle Function literal changed")
text = text.replace(old, new, 1)
if "validator_still_rejects_duplicate_ssa_definitions" in text:
    raise RuntimeError("duplicate SSA regression already exists")
text += r'''

#[test]
fn validator_still_rejects_duplicate_ssa_definitions() {
    let span = Span::new(SourceId(0), 0, 0).expect("empty test span is valid");
    let module = Module {
        definitions: Vec::new(),
        functions: vec![Function {
            id: FunctionId(0),
            span,
            parameters: Vec::new(),
            locals: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_effects: Vec::new(),
            current_lifecycle: Register(0),
            register_types: vec![IrType::Lifecycle, IrType::Int],
            register_storage: vec![RegisterStorage::Trivial, RegisterStorage::Trivial],
            storage_scope_parents: vec![None],
            return_type: IrType::Int,
            blocks: vec![IrBlock {
                id: IrBlockId(0),
                instructions: vec![
                    Instruction::ConstInt { dst: Register(1), value: 1, span },
                    Instruction::ConstInt { dst: Register(1), value: 2, span },
                ],
                terminator: Terminator::Return(Some(Register(1))),
            }],
            entry: IrBlockId(0),
        }],
        main: FunctionId(0),
    };

    let diagnostics = validate(&module);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code.0 == "KLD9002"
            && diagnostic.primary.message.contains("defined more than once")
    }), "{diagnostics:#?}");
}
'''
path.write_text(text)
print("declared IR local slots and limited mutable-write exemption to those slots")