use keld_interpreter::{Value, run_text_for_test};

#[test]
fn calls_map_source_order_arguments_to_parameter_registers() {
    let result = run_text_for_test(
        "fn subtract(a: Int, b: Int) -> Int { return a - b }\nfn main() -> Int { return subtract(9, 4) }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(5));
}

#[test]
fn structs_and_identity_branches_execute_with_declared_layout() {
    let result = run_text_for_test(
        "struct Pair {\nleft: Int\nright: Int\n}\nentity Enemy {\nhealth: Int\n}\nfn choose(a: Enemy, b: Enemy, pair: Pair) -> Int { if a == b { return pair.left } else { return pair.right } }\nfn main() -> Int { lifecycle level { let enemy = Enemy(health: 0); let pair = Pair(right: 2, left: 1); return choose(enemy, enemy, pair) } }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(1));
}

#[test]
fn nested_struct_arguments_are_copied_without_recursive_host_clone() {
    let result = run_text_for_test(
        "struct Inner {\nvalue: Int\n}\nstruct Outer {\ninner: Inner\n}\nfn read(value: Outer) -> Int { return value.inner.value }\nfn main() -> Int { let inner = Inner(value: 12); let outer = Outer(inner: inner); return read(outer) }\n",
    )
    .unwrap();

    assert_eq!(result.value, Value::Int(12));
}
