#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BytePos(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    source: SourceId,
    start: BytePos,
    end: BytePos,
}

impl Span {
    #[must_use]
    pub const fn new(source: SourceId, start: u32, end: u32) -> Option<Self> {
        if start <= end {
            Some(Self {
                source,
                start: BytePos(start),
                end: BytePos(end),
            })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    #[must_use]
    pub const fn start(self) -> BytePos {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> BytePos {
        self.end
    }

    #[must_use]
    pub fn cover(self, other: Self) -> Option<Self> {
        if self.source != other.source {
            return None;
        }

        Some(Self {
            source: self.source,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        })
    }
}
