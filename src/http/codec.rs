use std::{collections::BTreeSet, convert::Infallible, fmt, io::Cursor};

use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{FromRequest, FromRequestParts, Request},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, request::Parts},
    response::Response,
};

pub const MAX_REQUEST_BODY_SIZE_BYTES: usize = 1_000_000;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, MapAccess, SeqAccess, Visitor},
};

use crate::{
    http::negotiation::{
        CBOR_MEDIA_TYPE, JSON_MEDIA_TYPE, Representation, decode_parameter,
        negotiate_api_representation, negotiate_json_representation, split_outside_quotes,
    },
    problem::{ProblemCode, ensure_vary, problem_response},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFormat(pub Representation);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonResponseFormat(pub Representation);

impl<S> FromRequestParts<S> for ResponseFormat
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        negotiate_api_representation(&parts.headers, true)
            .map(Self)
            .ok_or_else(|| problem_response(ProblemCode::NotAcceptable, &parts.headers))
    }
}

impl<S> FromRequestParts<S> for JsonResponseFormat
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        negotiate_json_representation(&parts.headers)
            .map(Self)
            .ok_or_else(|| problem_response(ProblemCode::NotAcceptable, &parts.headers))
    }
}

#[derive(Debug)]
pub struct BufferedBody(pub Bytes);

impl<S> FromRequest<S> for BufferedBody
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let headers = request.headers().clone();
        to_bytes(request.into_body(), MAX_REQUEST_BODY_SIZE_BYTES)
            .await
            .map(Self)
            .map_err(|_| problem_response(ProblemCode::PayloadTooLarge, &headers))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestBodyDecodeError {
    Invalid,
    UnsupportedMediaType,
    Validation,
}

impl RequestBodyDecodeError {
    #[must_use]
    pub fn into_response(self, request_headers: &HeaderMap) -> Response {
        problem_response(
            match self {
                Self::Invalid => ProblemCode::InvalidRequest,
                Self::UnsupportedMediaType => ProblemCode::UnsupportedMediaType,
                Self::Validation => ProblemCode::ValidationFailed,
            },
            request_headers,
        )
    }
}

pub fn success_response<T: Serialize>(
    status: StatusCode,
    format: ResponseFormat,
    body: &T,
) -> Response {
    success_response_with_headers(status, format, body, std::iter::empty())
}

pub fn json_success_response<T: Serialize>(
    status: StatusCode,
    format: JsonResponseFormat,
    body: &T,
) -> Response {
    success_response(status, ResponseFormat(format.0), body)
}

pub fn no_content_response(
    extra_headers: impl IntoIterator<Item = (HeaderName, HeaderValue)>,
) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::from_stream(futures_util::stream::empty::<
            Result<Bytes, Infallible>,
        >()))
        .expect("no-content response should build");
    for (name, value) in extra_headers {
        response.headers_mut().insert(name, value);
    }
    response
}

pub fn success_response_with_headers<T, I>(
    status: StatusCode,
    format: ResponseFormat,
    body: &T,
    extra_headers: I,
) -> Response
where
    T: Serialize,
    I: IntoIterator<Item = (HeaderName, HeaderValue)>,
{
    let payload = match format.0 {
        Representation::Cbor => {
            let mut payload = Vec::new();
            ciborium::into_writer(body, &mut payload)
                .expect("serializing a validated success response to CBOR should succeed");
            payload
        }
        Representation::Json | Representation::JsonUtf8 => serde_json::to_vec(body)
            .expect("serializing a validated success response to JSON should succeed"),
    };
    let mut response = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, format.0.success_content_type())
        .body(Body::from(payload))
        .expect("success response should build");
    ensure_vary(response.headers_mut(), ["Accept"]);
    for (name, value) in extra_headers {
        response.headers_mut().insert(name, value);
    }
    response
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "decoding is the ownership boundary for the buffered request body"
)]
pub fn decode_request_body<T>(
    request_headers: &HeaderMap,
    body: Bytes,
) -> Result<T, RequestBodyDecodeError>
where
    T: DeserializeOwned,
{
    validate_request_content_encoding(request_headers)?;
    let format = request_body_format(request_headers, body.is_empty())?;
    if body.is_empty() {
        return Err(RequestBodyDecodeError::Invalid);
    }

    match format {
        Representation::Json | Representation::JsonUtf8 => {
            let value = parse_strict_json(&body)?;
            serde_json::from_value(value).map_err(|_| RequestBodyDecodeError::Validation)
        }
        Representation::Cbor => {
            let mut reader = Cursor::new(body.as_ref());
            let value: ciborium::Value =
                ciborium::from_reader(&mut reader).map_err(|_| RequestBodyDecodeError::Invalid)?;
            if reader.position() != body.len() as u64 || has_duplicate_cbor_key(&value) {
                return Err(RequestBodyDecodeError::Invalid);
            }
            let mut canonical = Vec::new();
            ciborium::into_writer(&value, &mut canonical)
                .map_err(|_| RequestBodyDecodeError::Validation)?;
            ciborium::from_reader(canonical.as_slice())
                .map_err(|_| RequestBodyDecodeError::Validation)
        }
    }
}

fn request_body_format(
    headers: &HeaderMap,
    body_is_empty: bool,
) -> Result<Representation, RequestBodyDecodeError> {
    let values = headers
        .get_all(header::CONTENT_TYPE)
        .iter()
        .collect::<Vec<_>>();
    if values.is_empty() {
        return if body_is_empty {
            Err(RequestBodyDecodeError::Invalid)
        } else {
            Err(RequestBodyDecodeError::UnsupportedMediaType)
        };
    }
    if values.len() != 1 {
        return Err(RequestBodyDecodeError::UnsupportedMediaType);
    }
    let value = values[0]
        .to_str()
        .map_err(|_| RequestBodyDecodeError::UnsupportedMediaType)?;
    parse_content_type(value).ok_or(RequestBodyDecodeError::UnsupportedMediaType)
}

fn parse_content_type(value: &str) -> Option<Representation> {
    if value.contains(',') {
        return None;
    }
    let parts = split_outside_quotes(value, ';')?;
    let media_type = parts.first()?.trim().to_ascii_lowercase();
    match media_type.as_str() {
        CBOR_MEDIA_TYPE if parts.len() == 1 => Some(Representation::Cbor),
        JSON_MEDIA_TYPE if parts.len() == 1 => Some(Representation::Json),
        JSON_MEDIA_TYPE if parts.len() == 2 => {
            let (name, value) = parts[1].trim().split_once('=')?;
            let value = decode_parameter(value.trim())?;
            (name.trim().eq_ignore_ascii_case("charset") && value.eq_ignore_ascii_case("utf-8"))
                .then_some(Representation::Json)
        }
        _ => None,
    }
}

fn validate_request_content_encoding(headers: &HeaderMap) -> Result<(), RequestBodyDecodeError> {
    let values = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    if values.len() != 1
        || !values[0].to_str().is_ok_and(|value| {
            !value.contains(',') && value.trim().eq_ignore_ascii_case("identity")
        })
    {
        return Err(RequestBodyDecodeError::UnsupportedMediaType);
    }
    Ok(())
}

pub(crate) fn parse_strict_json(body: &[u8]) -> Result<serde_json::Value, RequestBodyDecodeError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let StrictJsonValue(value) = StrictJsonValue::deserialize(&mut deserializer)
        .map_err(|_| RequestBodyDecodeError::Invalid)?;
    deserializer
        .end()
        .map_err(|_| RequestBodyDecodeError::Invalid)?;
    Ok(value)
}

struct StrictJsonValue(serde_json::Value);

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object names")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .map(StrictJsonValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(value.into()))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(StrictJsonValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictJsonValue(serde_json::Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom("duplicate JSON object name"));
            }
            let StrictJsonValue(value) = map.next_value()?;
            values.insert(key, value);
        }
        Ok(StrictJsonValue(serde_json::Value::Object(values)))
    }
}

fn has_duplicate_cbor_key(value: &ciborium::Value) -> bool {
    match value {
        ciborium::Value::Map(entries) => {
            let mut keys = BTreeSet::<ciborium::value::CanonicalValue>::new();
            let duplicate = entries
                .iter()
                .any(|(key, _)| !keys.insert(key.clone().into()));
            duplicate
                || entries.iter().any(|(key, value)| {
                    has_duplicate_cbor_key(key) || has_duplicate_cbor_key(value)
                })
        }
        ciborium::Value::Array(values) => values.iter().any(has_duplicate_cbor_key),
        ciborium::Value::Tag(_, value) => has_duplicate_cbor_key(value),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderValue, header},
    };
    use serde::Deserialize;

    use super::{
        MAX_REQUEST_BODY_SIZE_BYTES, Representation, RequestBodyDecodeError, StrictJsonVisitor,
        decode_request_body, has_duplicate_cbor_key, parse_content_type,
    };

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Payload {
        name: String,
    }

    fn json_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers
    }

    #[test]
    fn strict_json_separates_syntax_from_schema_failures() {
        let headers = json_headers();
        assert_eq!(
            decode_request_body::<Payload>(
                &headers,
                Bytes::from_static(br#"{"name":"Ada","name":"Grace"}"#)
            ),
            Err(RequestBodyDecodeError::Invalid)
        );
        assert_eq!(
            decode_request_body::<Payload>(
                &headers,
                Bytes::from_static(br#"{"name":"Ada","extra":true}"#)
            ),
            Err(RequestBodyDecodeError::Validation)
        );
        assert_eq!(
            decode_request_body::<Payload>(&headers, Bytes::from_static(br#"{"name":"Ada"} null"#)),
            Err(RequestBodyDecodeError::Invalid)
        );
    }

    #[test]
    fn empty_and_missing_media_follow_the_exact_precedence() {
        assert_eq!(
            decode_request_body::<Payload>(&HeaderMap::new(), Bytes::new()),
            Err(RequestBodyDecodeError::Invalid)
        );
        assert_eq!(
            decode_request_body::<Payload>(&HeaderMap::new(), Bytes::from_static(b"{}")),
            Err(RequestBodyDecodeError::UnsupportedMediaType)
        );
    }

    #[test]
    fn content_type_parser_accepts_only_the_owned_exact_forms() {
        for (value, expected) in [
            ("application/json", Representation::Json),
            ("Application/JSON", Representation::Json),
            ("application/json;charset=utf-8", Representation::Json),
            ("application/json; CHARSET=\"UTF-8\"", Representation::Json),
            ("application/json;charset=\"utf\\-8\"", Representation::Json),
            ("application/cbor", Representation::Cbor),
        ] {
            assert_eq!(parse_content_type(value), Some(expected), "{value}");
        }

        for value in [
            "application/cbor;charset=utf-8",
            "application/json;charset=utf-8;version=1",
            "application/json;boundary=utf-8",
            "application/json;charset=latin1",
            "application/json;charset=",
            "application/json;charset=\"\"",
            "application/json;charset=\"utf-8",
            "application/json;charset=\"utf-8\\\"",
            "application/json;charset=\"utf\"-8\"",
            "application/json;charset=\"utf\n-8\"",
            "application/json,application/cbor",
        ] {
            assert_eq!(parse_content_type(value), None, "{value}");
        }
    }

    #[test]
    fn strict_json_diagnostic_and_nested_cbor_duplicate_detection_are_stable() {
        let error = <serde::de::value::Error as serde::de::Error>::invalid_type(
            serde::de::Unexpected::Unit,
            &StrictJsonVisitor,
        );
        assert_eq!(
            error.to_string(),
            "invalid type: unit value, expected a JSON value without duplicate object names"
        );

        let duplicate = ciborium::Value::Map(vec![
            (
                ciborium::Value::Text("key".to_owned()),
                ciborium::Value::Bool(true),
            ),
            (
                ciborium::Value::Text("key".to_owned()),
                ciborium::Value::Bool(false),
            ),
        ]);
        assert!(has_duplicate_cbor_key(&ciborium::Value::Array(vec![
            duplicate.clone()
        ])));
        assert!(has_duplicate_cbor_key(&ciborium::Value::Tag(
            42,
            Box::new(duplicate)
        )));
        assert!(has_duplicate_cbor_key(&ciborium::Value::Map(vec![(
            ciborium::Value::Text("outer".to_owned()),
            ciborium::Value::Map(vec![
                (
                    ciborium::Value::Text("nested".to_owned()),
                    ciborium::Value::Bool(true),
                ),
                (
                    ciborium::Value::Text("nested".to_owned()),
                    ciborium::Value::Bool(false),
                ),
            ]),
        )])));
        assert!(!has_duplicate_cbor_key(&ciborium::Value::Array(vec![
            ciborium::Value::Text("unique".to_owned())
        ])));
    }

    #[test]
    fn large_cbor_maps_do_not_require_pairwise_duplicate_scans() {
        const ENTRY_COUNT: u64 = 16_384;

        let mut entries = (0..ENTRY_COUNT)
            .map(|key| (ciborium::Value::Integer(key.into()), ciborium::Value::Null))
            .collect::<Vec<_>>();
        let unique = ciborium::Value::Map(entries.clone());
        assert!(!has_duplicate_cbor_key(&unique));

        let mut body = Vec::new();
        ciborium::into_writer(&unique, &mut body).expect("CBOR map");
        assert!(body.len() < MAX_REQUEST_BODY_SIZE_BYTES);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/cbor"),
        );
        assert_eq!(
            decode_request_body::<Payload>(&headers, Bytes::from(body)),
            Err(RequestBodyDecodeError::Validation)
        );

        entries.push((
            ciborium::Value::Integer((ENTRY_COUNT - 1).into()),
            ciborium::Value::Bool(true),
        ));
        assert!(has_duplicate_cbor_key(&ciborium::Value::Map(entries)));
    }
}
