use std::io::Cursor;

use axum::{
    body::{Body, Bytes},
    extract::{FromRequest, FromRequestParts, Request},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, request::Parts},
    response::Response,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    http::negotiation::{
        CBOR_MEDIA_TYPE, JSON_MEDIA_TYPE, Representation, negotiate_api_representation,
    },
    problem::{ensure_vary, problem_response},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseFormat(pub Representation);

impl<S> FromRequestParts<S> for ResponseFormat
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        negotiate_api_representation(&parts.headers, true)
            .map(Self)
            .ok_or_else(|| {
                problem_response(
                    StatusCode::NOT_ACCEPTABLE,
                    "no acceptable response representation",
                    &parts.headers,
                )
            })
    }
}

#[derive(Debug)]
pub struct BufferedBody(pub Bytes);

impl<S> FromRequest<S> for BufferedBody
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let headers = request.headers().clone();
        Bytes::from_request(request, state)
            .await
            .map(Self)
            .map_err(|error| {
                let status = error.status();
                if status == StatusCode::PAYLOAD_TOO_LARGE {
                    problem_response(status, "request body is too large", &headers)
                } else {
                    problem_response(StatusCode::BAD_REQUEST, "invalid request body", &headers)
                }
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestBodyDecodeError {
    Invalid,
    UnsupportedMediaType,
}

impl RequestBodyDecodeError {
    pub fn into_response(self, request_headers: &HeaderMap) -> Response {
        match self {
            Self::Invalid => problem_response(
                StatusCode::BAD_REQUEST,
                "invalid request body",
                request_headers,
            ),
            Self::UnsupportedMediaType => problem_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported request media type",
                request_headers,
            ),
        }
    }
}

pub fn success_response<T: Serialize>(
    status: StatusCode,
    format: ResponseFormat,
    body: &T,
) -> Response {
    success_response_with_headers(status, format, body, std::iter::empty())
}

pub fn no_content_response(
    extra_headers: impl IntoIterator<Item = (HeaderName, HeaderValue)>,
) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .expect("response should build");

    ensure_vary(response.headers_mut(), ["Origin", "Accept"]);
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
    let mut response = match format.0 {
        Representation::Cbor => {
            let mut payload = Vec::new();
            ciborium::into_writer(body, &mut payload)
                .expect("serializing success response to CBOR should succeed");
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, CBOR_MEDIA_TYPE)
                .body(Body::from(payload))
                .expect("response should build")
        }
        Representation::Json => {
            let payload = serde_json::to_vec(body)
                .expect("serializing success response to JSON should succeed");
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, JSON_MEDIA_TYPE)
                .body(Body::from(payload))
                .expect("response should build")
        }
    };

    ensure_vary(response.headers_mut(), ["Origin", "Accept"]);
    for (name, value) in extra_headers {
        response.headers_mut().insert(name, value);
    }
    response
}

pub fn decode_request_body<T>(
    request_headers: &HeaderMap,
    body: Bytes,
) -> Result<T, RequestBodyDecodeError>
where
    T: DeserializeOwned,
{
    if body.is_empty() {
        return Err(RequestBodyDecodeError::Invalid);
    }

    match request_body_format(request_headers)? {
        Representation::Json => {
            serde_json::from_slice(&body).map_err(|_| RequestBodyDecodeError::Invalid)
        }
        Representation::Cbor => {
            let mut reader = Cursor::new(body.as_ref());
            let value =
                ciborium::from_reader(&mut reader).map_err(|_| RequestBodyDecodeError::Invalid)?;
            if reader.position() != body.len() as u64 {
                return Err(RequestBodyDecodeError::Invalid);
            }
            Ok(value)
        }
    }
}

fn request_body_format(headers: &HeaderMap) -> Result<Representation, RequestBodyDecodeError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(RequestBodyDecodeError::UnsupportedMediaType)?;
    let mut parts = content_type.split(';');
    let media_type = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let parameters = parts
        .map(str::trim)
        .filter(|parameter| !parameter.is_empty())
        .collect::<Vec<_>>();

    match media_type.as_str() {
        JSON_MEDIA_TYPE if valid_json_content_type_parameters(&parameters) => {
            Ok(Representation::Json)
        }
        CBOR_MEDIA_TYPE if parameters.is_empty() => Ok(Representation::Cbor),
        _ => Err(RequestBodyDecodeError::UnsupportedMediaType),
    }
}

fn valid_json_content_type_parameters(parameters: &[&str]) -> bool {
    match parameters {
        [] => true,
        [parameter] => parameter.split_once('=').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("charset") && valid_utf8_charset(value)
        }),
        _ => false,
    }
}

fn valid_utf8_charset(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("utf-8")
        || value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .is_some_and(|value| value.eq_ignore_ascii_case("utf-8"))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderValue, StatusCode, header},
    };
    use serde::{Deserialize, Serialize};

    use super::{
        RequestBodyDecodeError, ResponseFormat, decode_request_body, no_content_response,
        success_response,
    };
    use crate::http::negotiation::Representation;

    #[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
    struct Payload {
        message: String,
    }

    #[tokio::test]
    async fn success_response_uses_selected_format() {
        let response = success_response(
            StatusCode::OK,
            ResponseFormat(Representation::Json),
            &Payload {
                message: "hello".to_string(),
            },
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        assert_eq!(body, Bytes::from_static(br#"{"message":"hello"}"#));
    }

    #[test]
    fn no_content_response_sets_vary_headers() {
        let response = no_content_response(std::iter::empty());

        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let vary_values = response
            .headers()
            .get_all(header::VARY)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();

        assert_eq!(vary_values, vec!["Origin", "Accept"]);
    }

    #[test]
    fn decode_request_body_requires_an_owned_media_type() {
        let body = Bytes::from_static(br#"{"message":"json"}"#);
        assert_eq!(
            decode_request_body::<Payload>(&HeaderMap::new(), body.clone()),
            Err(RequestBodyDecodeError::UnsupportedMediaType)
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/example+cbor"),
        );
        assert_eq!(
            decode_request_body::<Payload>(&headers, body),
            Err(RequestBodyDecodeError::UnsupportedMediaType)
        );
    }

    #[test]
    fn decode_request_body_supports_json_and_one_cbor_item() {
        let mut json_headers = HeaderMap::new();
        json_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=UTF-8"),
        );
        let json = decode_request_body::<Payload>(
            &json_headers,
            Bytes::from_static(br#"{"message":"json"}"#),
        )
        .expect("json payload should decode");
        assert_eq!(json.message, "json");

        let mut cbor_headers = HeaderMap::new();
        cbor_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/cbor"),
        );
        let mut payload = Vec::new();
        ciborium::into_writer(
            &Payload {
                message: "cbor".to_string(),
            },
            &mut payload,
        )
        .expect("CBOR payload should serialize");

        let cbor = decode_request_body::<Payload>(&cbor_headers, Bytes::from(payload.clone()))
            .expect("CBOR payload should decode");
        assert_eq!(cbor.message, "cbor");

        payload.push(0xf6);
        assert_eq!(
            decode_request_body::<Payload>(&cbor_headers, Bytes::from(payload)),
            Err(RequestBodyDecodeError::Invalid)
        );
    }

    #[test]
    fn decode_request_body_requires_balanced_json_charset_quotes() {
        for content_type in [
            "application/json; charset=utf-8",
            "application/json; charset=\"UTF-8\"",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).expect("content type should be valid"),
            );
            assert!(
                decode_request_body::<Payload>(
                    &headers,
                    Bytes::from_static(br#"{"message":"json"}"#),
                )
                .is_ok()
            );
        }

        for content_type in [
            "application/json; charset=\"utf-8",
            "application/json; charset=utf-8\"",
            "application/json; charset=\"\"utf-8\"\"",
            "application/json; charset=iso-8859-1",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type).expect("content type should be valid"),
            );
            assert_eq!(
                decode_request_body::<Payload>(
                    &headers,
                    Bytes::from_static(br#"{"message":"json"}"#),
                ),
                Err(RequestBodyDecodeError::UnsupportedMediaType),
                "{content_type} should be rejected"
            );
        }
    }
}
