from pathlib import Path

path = Path("crates/keld-flow/tests/evaluation_order.rs")
text = path.read_text()
if "fn local_compound_assignment_reads_updates_and_stores_the_local()" in text:
    raise RuntimeError("local compound assignment regression already exists")
text += r'''

#[test]
fn local_compound_assignment_reads_updates_and_stores_the_local() {
    let flow = lower_text_for_test("fn main() -> Int { var i = 1; i += 2; return i }\n").unwrap();
    let ops = flow.function_named("main").unwrap().linear_ops();

    let compound_store = ops
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| match operation {
            FlowOp::StoreLocal { local, .. } if local.0 == 0 => Some(index),
            _ => None,
        })
        .nth(1)
        .expect("compound assignment must store the updated local");
    let read = ops[..compound_store]
        .iter()
        .rposition(|operation| matches!(operation, FlowOp::CopyLocal { local, .. } if local.0 == 0))
        .expect("compound assignment must read the old local value");
    let calculation = ops[..compound_store]
        .iter()
        .rposition(|operation| matches!(operation, FlowOp::BinaryInt { .. }))
        .expect("compound assignment must calculate the updated value");

    assert!(read < calculation && calculation < compound_store);
}
'''
path.write_text(text)
print("added local compound-assignment Flow regression")
