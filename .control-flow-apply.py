from pathlib import Path

path = Path("crates/keld-storage/src/verify.rs")
text = path.read_text()
old = r'''        let mut outgoing = state.clone();
        if let Terminator::ExitScopes { storage_scopes, .. } = &block.terminator {
            cleanup::exit_scopes(&mut outgoing.cleanup_orders, storage_scopes);
        }
'''
new = r'''        let mut outgoing = state.clone();
        if let Terminator::ExitScopes { storage_scopes, .. } = &block.terminator {
            cleanup::exit_scopes(&mut outgoing.cleanup_orders, storage_scopes);
            for (index, scope) in function.local_scopes.iter().copied().enumerate() {
                if !storage_scopes.contains(&scope)
                    || flow.types.storage_class(function.local_types[index])
                        != StorageClass::SingleHome
                {
                    continue;
                }
                outgoing.homes[index] = Home::Empty(EmptyReason::Uninitialized);
                outgoing.borrowed.remove(&LocalId(
                    u32::try_from(index).expect("flow local index fits in u32"),
                ));
            }
        }
'''
count = text.count(old)
if count != 1:
    raise RuntimeError(f"expected one ExitScopes outgoing transfer, found {count}")
path.write_text(text.replace(old, new, 1))
print("reset exited lexical single-home locals in successor state")
