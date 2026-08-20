use std::{error::Error, fmt};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

const CURSOR_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorDirection {
    Next,
    Prev,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    version: u8,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub limit: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub direction: CursorDirection,
    pub value: String,
}

impl Cursor {
    #[must_use]
    pub fn new(
        scope: &CursorScope<'_>,
        direction: CursorDirection,
        value: impl Into<String>,
    ) -> Self {
        Self {
            version: CURSOR_VERSION,
            operation: scope.operation.to_owned(),
            owner: scope.owner.map(str::to_owned),
            repository: scope.repository.map(str::to_owned),
            limit: scope.limit,
            category: scope.category.map(str::to_owned),
            direction,
            value: value.into(),
        }
    }

    #[must_use]
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(self).expect("cursor state serialization should be infallible"),
        )
    }

    #[must_use]
    pub fn belongs_to(&self, scope: &CursorScope<'_>) -> bool {
        self.version == CURSOR_VERSION
            && self.operation == scope.operation
            && self.owner.as_deref() == scope.owner
            && self.repository.as_deref() == scope.repository
            && self.limit == scope.limit
            && self.category.as_deref() == scope.category
            && !self.value.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CursorScope<'a> {
    pub operation: &'a str,
    pub owner: Option<&'a str>,
    pub repository: Option<&'a str>,
    pub limit: u16,
    pub category: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidCursor;

impl fmt::Display for InvalidCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid cursor")
    }
}

impl Error for InvalidCursor {}

pub fn validate_cursor_text(value: &str) -> Result<(), InvalidCursor> {
    if value.is_empty()
        || value.chars().count() > 2_048
        || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    {
        return Err(InvalidCursor);
    }
    Ok(())
}

pub fn decode_cursor(encoded: &str) -> Result<Cursor, InvalidCursor> {
    validate_cursor_text(encoded)?;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| InvalidCursor)?;
    let cursor: Cursor = serde_json::from_slice(&decoded).map_err(|_| InvalidCursor)?;
    if cursor.encode() != encoded {
        return Err(InvalidCursor);
    }
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::{Cursor, CursorDirection, CursorScope, decode_cursor, validate_cursor_text};

    fn scope<'a>() -> CursorScope<'a> {
        CursorScope {
            operation: "listItems",
            owner: None,
            repository: None,
            limit: 20,
            category: Some("tools"),
        }
    }

    #[test]
    fn cursor_round_trips_and_binds_every_scope_member() {
        let cursor = Cursor::new(&scope(), CursorDirection::Next, "item-020");
        let encoded = cursor.encode();
        assert_eq!(decode_cursor(&encoded), Ok(cursor.clone()));
        assert!(cursor.belongs_to(&scope()));

        let changed_limit = CursorScope {
            limit: 10,
            ..scope()
        };
        assert!(!cursor.belongs_to(&changed_limit));
    }

    #[test]
    fn public_cursor_text_has_exact_boundary() {
        assert_eq!(super::InvalidCursor.to_string(), "invalid cursor");
        assert!(validate_cursor_text("").is_err());
        assert!(validate_cursor_text("with space").is_err());
        assert!(validate_cursor_text("non-ascii-é").is_err());
        assert!(validate_cursor_text(&"x".repeat(2_048)).is_ok());
        assert!(validate_cursor_text(&"x".repeat(2_049)).is_err());
        assert!(decode_cursor("eyJ2ZXJzaW9uIjoyfQ").is_err());
    }
}
