from pathlib import Path

path = Path("crates/keld-interpreter/src/frame.rs")
text = path.read_text()
old = r'''    pub fn set_loan(&mut self, register: Register, place: RuntimePlace) -> Result<(), ()> {
        let destination = self.registers.get_mut(register.0 as usize).ok_or(())?;
        if !matches!(destination, RegisterSlot::Empty) {
            return Err(());
        }
        *destination = RegisterSlot::Loan(place);
        Ok(())
    }
'''
new = r'''    pub fn set_loan(&mut self, register: Register, place: RuntimePlace) -> Result<(), ()> {
        let destination = self.registers.get_mut(register.0 as usize).ok_or(())?;
        match destination {
            RegisterSlot::Empty | RegisterSlot::Loan(_) => {
                // A static SSA loan definition can execute again after a CFG back-edge.
                // The previous dynamic loan value is dead at that redefinition point.
                *destination = RegisterSlot::Loan(place);
                Ok(())
            }
            RegisterSlot::Owned(_) | RegisterSlot::DropSlot(_) => Err(()),
        }
    }
'''
if text.count(old) != 1:
    raise RuntimeError("Frame::set_loan implementation changed")
path.write_text(text.replace(old, new, 1))
print("allowed dynamic re-execution of validated loan definitions across back-edges")