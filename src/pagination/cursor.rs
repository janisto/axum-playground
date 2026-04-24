use std::{error::Error, fmt};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub kind: String,
    pub value: String,
}

impl Cursor {
    pub fn new(kind: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
        }
    }

    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}:{}", self.kind, self.value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidCursor;

impl fmt::Display for InvalidCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid cursor format")
    }
}

impl Error for InvalidCursor {}

pub fn decode_cursor(encoded: &str) -> Result<Cursor, InvalidCursor> {
    if encoded.is_empty() {
        return Ok(Cursor::new("", ""));
    }

    let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| InvalidCursor)?;
    let decoded = String::from_utf8(decoded).map_err(|_| InvalidCursor)?;
    let (kind, value) = decoded.split_once(':').ok_or(InvalidCursor)?;

    Ok(Cursor::new(kind, value))
}

#[cfg(test)]
mod tests {
    use super::{Cursor, InvalidCursor, decode_cursor};

    #[test]
    fn cursor_round_trips_for_common_values() {
        let cursor = Cursor::new("item", "2024-01-15T10:30:00.000Z");
        let encoded = cursor.encode();
        let decoded = decode_cursor(&encoded).expect("cursor should decode");

        assert_eq!(decoded, cursor);
    }

    #[test]
    fn empty_cursor_decodes_to_zero_value() {
        let cursor = decode_cursor("").expect("empty cursor should decode");
        assert_eq!(cursor, Cursor::new("", ""));
    }

    #[test]
    fn invalid_cursor_rejects_bad_base64_and_missing_separator() {
        assert_eq!(decode_cursor("!!!invalid!!!"), Err(InvalidCursor));
        assert_eq!(decode_cursor("dGVzdA"), Err(InvalidCursor));
    }

    #[test]
    fn encoded_cursor_is_url_safe() {
        let cursor = Cursor::new("item", "value+with/special=chars");
        let encoded = cursor.encode();

        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
    }

    #[test]
    fn cursor_preserves_additional_colons_in_value() {
        let cursor = Cursor::new("composite", "a:b:c:d");
        let encoded = cursor.encode();
        let decoded = decode_cursor(&encoded).expect("cursor should decode");

        assert_eq!(decoded, cursor);
    }
}
