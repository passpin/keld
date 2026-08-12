use keld_storage::{LocalStorage, ValueStorage, verify_text_for_test};

#[test]
fn plan_distinguishes_owned_temporary_local_home_and_loan() {
    let verified = verify_text_for_test(
        "fn inspect(items: List[Int]) -> Int { return items.length }\nfn forward(take items: List[Int]) -> List[Int] { let copied = items.copy(); return take copied }\nfn main() -> Int { return 0 }\n",
    );
    let module = verified.module.expect("storage verifies");
    let inspect = &module.annotations.functions[0];
    let forward = &module.annotations.functions[1];

    assert!(matches!(inspect.locals[0], LocalStorage::Loan));
    assert!(matches!(forward.locals[0], LocalStorage::Home { .. }));
    assert!(
        forward
            .values
            .iter()
            .any(|value| matches!(value, ValueStorage::OwnedTemporary { .. }))
    );
}
