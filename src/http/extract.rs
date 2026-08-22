use std::collections::BTreeMap;

use axum::{
    extract::{FromRequestParts, Path, rejection::PathRejection},
    http::request::Parts,
    response::Response,
};
use serde::de::DeserializeOwned;

use crate::problem::{ProblemCode, problem_response};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClosedQueryError;

#[derive(Debug)]
pub struct ProblemPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ProblemPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let headers = parts.headers.clone();
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|rejection| {
                let code = match rejection {
                    PathRejection::FailedToDeserializePathParams(_) => ProblemCode::InvalidRequest,
                    _ => ProblemCode::NotFound,
                };
                problem_response(code, &headers)
            })
    }
}

/// Rejects every decoded query member while accepting an absent or empty query string.
#[derive(Clone, Copy, Debug)]
pub struct NoQuery;

impl<S> FromRequestParts<S> for NoQuery
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parse_query(parts.uri.query())
            .map_err(|_| problem_response(ProblemCode::InvalidRequest, &parts.headers))?;
        if query.is_empty() {
            Ok(Self)
        } else {
            Err(problem_response(
                ProblemCode::InvalidRequest,
                &parts.headers,
            ))
        }
    }
}

/// A syntax-checked query whose operation handler still owns typed validation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StrictQuery(BTreeMap<String, String>);

impl StrictQuery {
    pub fn closed(self, allowed: &[&str]) -> Result<Self, ClosedQueryError> {
        if self.0.keys().all(|name| allowed.contains(&name.as_str())) {
            Ok(self)
        } else {
            Err(ClosedQueryError)
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl<S> FromRequestParts<S> for StrictQuery
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parse_query(parts.uri.query())
            .map(Self)
            .map_err(|_| problem_response(ProblemCode::InvalidRequest, &parts.headers))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InvalidQuery;

fn parse_query(raw: Option<&str>) -> Result<BTreeMap<String, String>, InvalidQuery> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    if raw.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut values = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            return Err(InvalidQuery);
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = decode_query_component(name).ok_or(InvalidQuery)?;
        let value = decode_query_component(value).ok_or(InvalidQuery)?;
        if name.is_empty() || values.insert(name, value).is_some() {
            return Err(InvalidQuery);
        }
    }
    Ok(values)
}

pub(crate) fn decode_query_component(raw: &str) -> Option<String> {
    let mut bytes = raw.bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                let high = bytes.next().and_then(hex)?;
                let low = bytes.next().and_then(hex)?;
                decoded.push(high * 16 + low);
            }
            b'+' => decoded.push(b' '),
            byte if byte.is_ascii() => decoded.push(byte),
            _ => return None,
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::{decode_query_component, parse_query};

    #[test]
    fn strict_query_rejects_ambiguous_and_malformed_input() {
        assert!(parse_query(Some("limit=10&limit=20")).is_err());
        assert!(parse_query(Some("cursor=%")).is_err());
        assert!(parse_query(Some("cursor=%FF")).is_err());
        assert!(parse_query(Some("limit=10&&cursor=x")).is_err());
        assert!(parse_query(Some("=value")).is_err());
    }

    #[test]
    fn strict_query_decodes_utf8_without_repair() {
        let query = parse_query(Some("name=Mar%C3%ADa+Jos%C3%A9")).expect("valid query");
        assert_eq!(query.get("name").map(String::as_str), Some("María José"));
    }

    #[test]
    fn component_decoder_covers_each_hex_case_and_rejects_raw_non_ascii() {
        for (raw, expected) in [
            ("plain", "plain"),
            ("a+b", "a b"),
            ("%41%4a%4F", "AJO"),
            ("%61%6a%6F", "ajo"),
            ("%C3%AD", "í"),
        ] {
            assert_eq!(
                decode_query_component(raw).as_deref(),
                Some(expected),
                "{raw}"
            );
        }
        for raw in ["%", "%0", "%G0", "%0G", "%FF", "é"] {
            assert_eq!(decode_query_component(raw), None, "{raw:?}");
        }
    }
}
