mod common;

use std::convert::Infallible;

use axum::{
    body::{Body, Bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    MockAuthVerifier, MockGitHubService, MockProfileService, build_app,
    problem::{ProblemCode, ProblemDetails},
};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, state_with, test_state};

const BODY_LIMIT: usize = 1_000_000;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Greeting {
    message: String,
}

async fn problem(response: axum::response::Response, status: StatusCode, code: ProblemCode) {
    assert_eq!(response.status(), status);
    let body: ProblemDetails = if response.headers()[header::CONTENT_TYPE] == "application/cbor" {
        read_cbor_body(response).await
    } else {
        read_json_body(response).await
    };
    assert_eq!(body.status, status.as_u16());
    assert_eq!(body.code, code);
    assert_eq!(body.title, code.title());
    assert_eq!(body.detail, code.detail());
}

fn post(body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/hello")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.into())
        .expect("request should build")
}

fn exact_json_body(size: usize) -> Vec<u8> {
    let mut body = br#"{"name":"A"}"#.to_vec();
    assert!(body.len() <= size);
    body.resize(size, b' ');
    body
}

#[tokio::test]
async fn hello_successes_are_exact_and_preserve_the_validated_name() {
    let get = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/v1/hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.headers()[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        read_json_body::<Greeting>(get).await,
        Greeting {
            message: "Hello, World!".to_owned()
        }
    );

    let json = build_app(test_state())
        .oneshot(post(r#"{"name":"María José"}"#))
        .await
        .unwrap();
    assert_eq!(json.status(), StatusCode::OK);
    assert!(!json.headers().contains_key(header::LOCATION));
    assert_eq!(
        read_json_body::<Greeting>(json).await,
        Greeting {
            message: "Hello, María José!".to_owned()
        }
    );

    let mut payload = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name":"CBOR"}), &mut payload).unwrap();
    let cbor = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/cbor")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cbor.status(), StatusCode::OK);
    assert_eq!(cbor.headers()[header::CONTENT_TYPE], "application/cbor");
    assert_eq!(
        read_cbor_body::<Greeting>(cbor).await,
        Greeting {
            message: "Hello, CBOR!".to_owned()
        }
    );
}

#[tokio::test]
async fn hello_well_formed_schema_and_name_violations_are_422() {
    let cases = [
        r#"{}"#.to_owned(),
        r#"{"name":null}"#.to_owned(),
        r#"{"name":3}"#.to_owned(),
        r#"{"name":"","extra":true}"#.to_owned(),
        r#"{"name":""}"#.to_owned(),
        r#"{"name":" Ada"}"#.to_owned(),
        r#"{"name":"Ada "}"#.to_owned(),
        serde_json::to_string(&serde_json::json!({"name":"Ada\nLovelace"})).unwrap(),
        serde_json::to_string(&serde_json::json!({"name":"x".repeat(101)})).unwrap(),
    ];
    for body in cases {
        let response = build_app(test_state()).oneshot(post(body)).await.unwrap();
        problem(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
        )
        .await;
    }
}

#[tokio::test]
async fn hello_malformed_json_and_cbor_are_400() {
    let json_cases: Vec<Vec<u8>> = vec![
        br#"{"name":"Ada","name":"Grace"}"#.to_vec(),
        br#"{"name":"Ada"} null"#.to_vec(),
        br#"{"name":}"#.to_vec(),
        [vec![0xef, 0xbb, 0xbf], br#"{"name":"Ada"}"#.to_vec()].concat(),
        [br#"{"name":""#.to_vec(), vec![0xff], br#""}"#.to_vec()].concat(),
    ];
    for body in json_cases {
        let response = build_app(test_state()).oneshot(post(body)).await.unwrap();
        problem(
            response,
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
    }

    let duplicate_key = vec![
        0xa2, 0x64, b'n', b'a', b'm', b'e', 0x63, b'A', b'd', b'a', 0x64, b'n', b'a', b'm', b'e',
        0x65, b'G', b'r', b'a', b'c', b'e',
    ];
    let mut trailing_item = Vec::new();
    ciborium::into_writer(&serde_json::json!({"name":"Ada"}), &mut trailing_item).unwrap();
    trailing_item.push(0xf6);
    for body in [duplicate_key, trailing_item] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, "application/cbor")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        problem(
            response,
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
    }
}

#[tokio::test]
async fn hello_media_metadata_and_accept_precedence_are_exact() {
    let empty_missing = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        empty_missing,
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;

    let empty_supported = build_app(test_state())
        .oneshot(post(Body::empty()))
        .await
        .unwrap();
    problem(
        empty_supported,
        StatusCode::BAD_REQUEST,
        ProblemCode::InvalidRequest,
    )
    .await;

    let missing_nonempty = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        missing_nonempty,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ProblemCode::UnsupportedMediaType,
    )
    .await;

    for content_type in [
        "text/plain",
        "application/example+cbor",
        "application/cbor; charset=utf-8",
        "application/json, application/cbor",
    ] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(br#"{"name":"Ada"}"#.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();
        problem(
            response,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ProblemCode::UnsupportedMediaType,
        )
        .await;
    }

    let repeated = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_TYPE, "application/cbor")
                .body(Body::from(br#"{"name":"Ada"}"#.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        repeated,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ProblemCode::UnsupportedMediaType,
    )
    .await;

    for encoding in ["gzip", "identity, gzip"] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_ENCODING, encoding)
                    .body(Body::from(br#"{"name":"Ada"}"#.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();
        problem(
            response,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ProblemCode::UnsupportedMediaType,
        )
        .await;
    }

    let charset = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json; charset=\"UTF-8\"")
                .header(header::CONTENT_ENCODING, "identity")
                .body(Body::from(br#"{"name":"Ada"}"#.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(charset.status(), StatusCode::OK);

    let negotiated = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json;q=0, */*;q=1")
                .body(Body::from(br#"{"name":"Ada"}"#.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(negotiated.status(), StatusCode::OK);
    assert_eq!(
        negotiated.headers()[header::CONTENT_TYPE],
        "application/cbor"
    );

    let malformed = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json;q=0, */*;q=1")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        malformed.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );

    let unacceptable = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "text/html")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        unacceptable,
        StatusCode::NOT_ACCEPTABLE,
        ProblemCode::NotAcceptable,
    )
    .await;
}

#[tokio::test]
async fn hello_enforces_the_decimal_body_boundary_and_stops_after_overflow() {
    for size in [999_999, BODY_LIMIT] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, size)
                    .body(Body::from(exact_json_body(size)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "size {size}");
    }

    let declared = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, BODY_LIMIT + 1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        declared,
        StatusCode::PAYLOAD_TOO_LARGE,
        ProblemCode::PayloadTooLarge,
    )
    .await;

    for invalid in ["-1", "+1", "1.0", "18446744073709551616"] {
        let response = build_app(test_state())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/hello")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, invalid)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        problem(
            response,
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
    }

    let overflow =
        stream::once(async { Ok::<Bytes, Infallible>(Bytes::from(vec![b'x'; BODY_LIMIT + 1])) })
            .chain(stream::once(async {
                panic!("body reader polled beyond the first over-limit chunk");
                #[allow(unreachable_code)]
                Ok::<Bytes, Infallible>(Bytes::new())
            }));
    let streamed = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/hello")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(overflow))
                .unwrap(),
        )
        .await
        .unwrap();
    problem(
        streamed,
        StatusCode::PAYLOAD_TOO_LARGE,
        ProblemCode::PayloadTooLarge,
    )
    .await;
}

#[tokio::test]
async fn hello_closed_query_method_and_body_free_boundaries_suppress_dependencies() {
    let auth = MockAuthVerifier::test_user();
    let github = MockGitHubService::demo();
    let profile = MockProfileService::default();
    let app = build_app(state_with(auth.clone(), github.clone(), profile.clone()));

    for target in ["/v1/hello?x=1", "/v1/hello?x=1&x=2", "/v1/hello?x=%"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(target).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{target}");
    }

    let unpolled = Body::from_stream(stream::once(async {
        panic!("GET hello polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));
    assert_eq!(
        app.clone()
            .oneshot(Request::builder().uri("/v1/hello").body(unpolled).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let method = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/v1/hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(method.headers()[header::ALLOW], "GET, POST");
    assert_eq!(auth.call_count(), 0);
    assert_eq!(github.call_count(), 0);
    assert_eq!(profile.committed_write_count(), 0);
}
