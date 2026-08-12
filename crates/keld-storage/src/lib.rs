mod state;
mod verify;

pub use state::{EmptyReason, EmptyReasonKind, Home};
pub use verify::{
    FunctionStorageSummary, LoanEffect, StorageAnnotations, Verification, VerifiedStorageModule,
    verify, verify_text_for_test,
};

#[cfg(test)]
mod tests {
    use super::{EmptyReason, EmptyReasonKind, Home};

    #[test]
    fn home_join_preserves_the_single_home_lattice() {
        assert_eq!(Home::Live.join(&Home::Live), Home::Live);
        assert_eq!(
            Home::Empty(EmptyReason::Moved).join(&Home::Empty(EmptyReason::Uninitialized)),
            Home::Empty(EmptyReason::Multiple(
                [EmptyReasonKind::Moved, EmptyReasonKind::Uninitialized]
                    .into_iter()
                    .collect(),
            ))
        );
        assert_eq!(
            Home::Live.join(&Home::Empty(EmptyReason::Moved)),
            Home::MaybeLive
        );
    }
}
