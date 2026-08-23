from pathlib import Path

path = Path("crates/keld-ir/src/validate.rs")
text = path.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one match, found {count}: {old[:80]!r}")
    text = text.replace(old, new, 1)


replace_once(
    '''    fn register_storage(&self, register: Register) -> Option<&RegisterStorage> {
        self.function.register_storage.get(register.0 as usize)
    }

    fn require_live_storage_use(
''',
    '''    fn register_storage(&self, register: Register) -> Option<&RegisterStorage> {
        self.function.register_storage.get(register.0 as usize)
    }

    fn is_scalar_source_local(&self, register: Register) -> bool {
        self.function.locals.contains(&register)
            && self
                .function
                .register_types
                .get(register.0 as usize)
                .is_some_and(|ty| {
                    matches!(ty, IrType::Bool | IrType::Int | IrType::Lifecycle)
                })
    }

    fn validate_scalar_local_instruction_uses(&mut self, instruction: &Instruction) {
        let span = use_contract_span(instruction_span(instruction));
        if let Instruction::Phi { inputs, .. } = instruction {
            for (_, register) in inputs {
                if self.is_scalar_source_local(*register) {
                    self.sink.error(
                        REGISTER_ERROR,
                        span,
                        "scalar source local must be read through Copy",
                    );
                }
            }
            return;
        }
        for register in instruction_uses(instruction) {
            let allowed_copy_source =
                matches!(instruction, Instruction::Copy { src, .. } if *src == register);
            if self.is_scalar_source_local(register) && !allowed_copy_source {
                self.sink.error(
                    REGISTER_ERROR,
                    span,
                    "scalar source local must be read through Copy",
                );
            }
        }
    }

    fn validate_scalar_local_terminator_uses(&mut self, terminator: &Terminator, span: Span) {
        for register in terminator_uses(terminator) {
            if self.is_scalar_source_local(register) {
                self.sink.error(
                    REGISTER_ERROR,
                    use_contract_span(span),
                    "scalar source local must be read through Copy",
                );
            }
        }
    }

    fn require_live_storage_use(
''',
)

replace_once(
    '''        for instruction in &block.instructions {
            let span = instruction_span(instruction);
            if matches!(instruction, Instruction::Phi { .. }) {
''',
    '''        for instruction in &block.instructions {
            let span = instruction_span(instruction);
            self.validate_scalar_local_instruction_uses(instruction);
            if matches!(instruction, Instruction::Phi { .. }) {
''',
)

replace_once(
    '''        let terminator_span = terminator_span(&block.terminator, self.function.span);
        if !views.is_empty() {
''',
    '''        let terminator_span = terminator_span(&block.terminator, self.function.span);
        self.validate_scalar_local_terminator_uses(&block.terminator, terminator_span);
        if !views.is_empty() {
''',
)

replace_once(
    '''        match instruction {
            Instruction::Take { dst, src, .. } => {
''',
    '''        match instruction {
            Instruction::Copy { dst, .. } => {
                if matches!(role(*dst), Some(RegisterStorage::Home { .. })) {
                    require_empty(self.sink, *dst, "copy destination is already live");
                }
            }
            Instruction::Take { dst, src, .. } => {
''',
)

path.write_text(text)
Path(__file__).unlink()
