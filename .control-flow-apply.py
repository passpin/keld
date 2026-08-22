from pathlib import Path

path = Path("crates/keld-flow/src/lower.rs")
text = path.read_text()
old = r'''            HirStmtKind::CompoundAssign { target, op, value } => {
                let Some((base, base_ty, mut place, final_projection)) =
                    self.lower_target_prefix(target)
                else {
                    return;
                };
'''
new = r'''            HirStmtKind::CompoundAssign { target, op, value } => {
                if target.projections.is_empty() {
                    let old = self.new_value(TypeStore::INT);
                    self.emit(FlowOp::CopyLocal {
                        dst: old,
                        local: target.base,
                        span: statement.span,
                    });
                    if let Some(rhs) = self.lower_expression(value) {
                        let result = self.new_value(TypeStore::INT);
                        self.emit(FlowOp::BinaryInt {
                            dst: result,
                            op: *op,
                            lhs: old,
                            rhs,
                            span: statement.span,
                        });
                        self.emit(FlowOp::StoreLocal {
                            local: target.base,
                            value: result,
                            span: statement.span,
                        });
                    }
                    return;
                }
                let Some((base, base_ty, mut place, final_projection)) =
                    self.lower_target_prefix(target)
                else {
                    return;
                };
'''
count = text.count(old)
if count != 1:
    raise RuntimeError(f"expected one compound assignment arm, found {count}")
path.write_text(text.replace(old, new, 1))
print("lowered projection-free compound assignments through CopyLocal/BinaryInt/StoreLocal")
