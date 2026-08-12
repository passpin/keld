const INT_MIN_MAGNITUDE: u64 = 1_u64 << 63;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsedIntLiteral {
    Value(i64),
    IntMinMagnitude,
    OutOfRange,
}

#[must_use]
pub fn parse_int_literal(raw: &str) -> ParsedIntLiteral {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return ParsedIntLiteral::OutOfRange;
    }

    let mut magnitude = 0_u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'_' {
            let valid = index > 0
                && index + 1 < bytes.len()
                && bytes[index - 1].is_ascii_digit()
                && bytes[index + 1].is_ascii_digit();
            if !valid {
                return ParsedIntLiteral::OutOfRange;
            }
            continue;
        }
        if !byte.is_ascii_digit() {
            return ParsedIntLiteral::OutOfRange;
        }

        let Some(next) = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(byte - b'0')))
        else {
            return ParsedIntLiteral::OutOfRange;
        };
        if next > INT_MIN_MAGNITUDE {
            return ParsedIntLiteral::OutOfRange;
        }
        magnitude = next;
    }

    if magnitude == INT_MIN_MAGNITUDE {
        ParsedIntLiteral::IntMinMagnitude
    } else {
        i64::try_from(magnitude).map_or(ParsedIntLiteral::OutOfRange, ParsedIntLiteral::Value)
    }
}
