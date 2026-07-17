mod common;

use axum::{body::Body, http::Request};
use axum_playground::build_app;
use serde_json::Value;
use tower::ServiceExt;

use crate::common::{read_json_body, test_state};

#[tokio::test]
async fn every_local_openapi_reference_resolves() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .uri("/v1/openapi")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");
    let document: Value = read_json_body(response).await;

    assert_local_references_resolve(&document, &document);
}

fn assert_local_references_resolve(value: &Value, document: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                && let Some(pointer) = reference.strip_prefix('#')
            {
                assert!(
                    document.pointer(pointer).is_some(),
                    "OpenAPI reference does not resolve: {reference}"
                );
            }

            for child in object.values() {
                assert_local_references_resolve(child, document);
            }
        }
        Value::Array(array) => {
            for child in array {
                assert_local_references_resolve(child, document);
            }
        }
        _ => {}
    }
}
