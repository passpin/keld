use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EmptyReasonKind {
    Uninitialized,
    Moved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmptyReason {
    Uninitialized,
    Moved,
    Multiple(BTreeSet<EmptyReasonKind>),
}

impl EmptyReason {
    #[must_use]
    pub fn kinds(&self) -> BTreeSet<EmptyReasonKind> {
        match self {
            Self::Uninitialized => [EmptyReasonKind::Uninitialized].into_iter().collect(),
            Self::Moved => [EmptyReasonKind::Moved].into_iter().collect(),
            Self::Multiple(kinds) => kinds.clone(),
        }
    }

    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        let mut kinds = self.kinds();
        kinds.extend(other.kinds());
        match kinds.len() {
            1 if kinds.contains(&EmptyReasonKind::Moved) => Self::Moved,
            0 | 1 => Self::Uninitialized,
            _ => Self::Multiple(kinds),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Home {
    Empty(EmptyReason),
    Live,
    MaybeLive,
}

impl Home {
    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Live, Self::Live) => Self::Live,
            (Self::Empty(left), Self::Empty(right)) => Self::Empty(left.join(right)),
            _ => Self::MaybeLive,
        }
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }
}
