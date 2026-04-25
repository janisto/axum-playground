use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    problem::negotiate::select_format,
    problem::{ensure_vary, problem_response},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestBodyDecodeError;

impl RequestBodyDecodeError {
    pub fn into_response(self, request_headers: &HeaderMap) -> Response {
        problem_response(
            StatusCode::BAD_REQUEST,
            "invalid request body",
            request_headers,
        )
    }
}

pub fn success_response<T: Serialize>(
    status: StatusCode,
    request_headers: &HeaderMap,
    body: &T,
) -> Response {
    success_response_with_headers(status, request_headers, body, std::iter::empty())
}

pub fn no_content_response(
    _request_headers: &HeaderMap,
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
    request_headers: &HeaderMap,
    body: &T,
    extra_headers: I,
) -> Response
where
    T: Serialize,
    I: IntoIterator<Item = (HeaderName, HeaderValue)>,
{
    let prefers_cbor = select_format(
        request_headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
    );

    let mut response = if prefers_cbor {
        let mut payload = Vec::new();
        ciborium::into_writer(body, &mut payload)
            .expect("serializing success response to CBOR should succeed");
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/cbor")
            .body(Body::from(payload))
            .expect("response should build")
    } else {
        let payload =
            serde_json::to_vec(body).expect("serializing success response to JSON should succeed");
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .expect("response should build")
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
    if request_body_prefers_cbor(request_headers) {
        ciborium::from_reader(body.as_ref()).map_err(|_| RequestBodyDecodeError)
    } else {
        serde_json::from_slice(&body).map_err(|_| RequestBodyDecodeError)
    }
}

fn request_body_prefers_cbor(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let media_type = value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();

            media_type == "application/cbor" || media_type.ends_with("+cbor")
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Bytes,
        http::{HeaderMap, HeaderValue, StatusCode, header},
    };
    use serde::{Deserialize, Serialize};

    use super::{decode_request_body, no_content_response, success_response};

    #[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
    struct Payload {
        message: String,
    }

    #[tokio::test]
    async fn success_response_defaults_to_json() {
        let response = success_response(
            StatusCode::OK,
            &HeaderMap::new(),
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
        let response = no_content_response(&HeaderMap::new(), std::iter::empty());

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
    fn decode_request_body_supports_json_and_cbor() {
        let json = decode_request_body::<Payload>(
            &HeaderMap::new(),
            Bytes::from_static(br#"{"message":"json"}"#),
        )
        .expect("json payload should decode");
        assert_eq!(json.message, "json");

        let mut headers = HeaderMap::new();
        headers.insert(
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

        let cbor = decode_request_body::<Payload>(&headers, Bytes::from(payload))
            .expect("CBOR payload should decode");
        assert_eq!(cbor.message, "cbor");
    }
}
