from pathlib import Path

path = Path("crates/keld-native-runtime/src/lib.rs")
text = path.read_text(encoding="utf-8")
old = '''    pub fn text_concat(
        &mut self,
        lhs: KeldValue,
        rhs: KeldValue,
    ) -> Result<KeldValue, NativeValueError> {
        let left = self.text_bytes(lhs)?.to_vec();
        let right = self.text_bytes(rhs)?.to_vec();
        let length = left
            .len()
            .checked_add(right.len())
            .ok_or(NativeValueError::Allocation)?;
        if !self.allow_test_allocation(AllocationPhase::Concat) {
            return Err(NativeValueError::Allocation);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| NativeValueError::Allocation)?;
        bytes.extend_from_slice(&left);
        bytes.extend_from_slice(&right);
        self.allocate(NativePayload::Text(bytes))
    }
'''
new = '''    pub fn text_concat(
        &mut self,
        lhs: KeldValue,
        rhs: KeldValue,
    ) -> Result<KeldValue, NativeValueError> {
        let length = self
            .text_bytes(lhs)?
            .len()
            .checked_add(self.text_bytes(rhs)?.len())
            .ok_or(NativeValueError::Allocation)?;
        if !self.allow_test_allocation(AllocationPhase::Concat) {
            return Err(NativeValueError::Allocation);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| NativeValueError::Allocation)?;
        bytes.extend_from_slice(self.text_bytes(lhs)?);
        bytes.extend_from_slice(self.text_bytes(rhs)?);
        self.allocate(NativePayload::Text(bytes))
    }
'''
if old not in text:
    raise SystemExit("expected text_concat implementation not found")
text = text.replace(old, new, 1)
path.write_text(text, encoding="utf-8")
