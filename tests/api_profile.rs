mod common;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    AuthError, MockAuthVerifier, MockProfileService, Profile, ProfileService, ProfileServiceError,
    build_app, problem::ProblemDetails,
};
use tower::ServiceExt;

use crate::common::{
    read_cbor_body, read_json_body, read_text_body, test_state, test_state_with_auth_and_profile,
};

fn authorized_request(method: Method, uri: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer valid-token")
        .body(body)
        .expect("request should build")
}

fn authorized_json_request(method: Method, uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer valid-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request should build")
}

#[tokio::test]
async fn profile_routes_require_bearer_auth() {
    let missing_auth = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        missing_auth
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer")
    );

    let invalid_auth = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(invalid_auth.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        invalid_auth
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer")
    );
}

#[tokio::test]
async fn profile_crud_flow_matches_contract() {
    let state = test_state();

    let create_response = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::POST,
            "/v1/profile",
            r#"{"firstname":"John","lastname":"Doe","email":"JOHN@EXAMPLE.COM","phoneNumber":"+358401234567","marketing":true,"terms":true}"#,
        ))
        .await
        .expect("request should succeed");

    assert_eq!(create_response.status(), StatusCode::CREATED);
    assert_eq!(
        create_response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/v1/profile")
    );
    let created: Profile = read_json_body(create_response).await;
    assert_eq!(created.id, "user-123");
    assert_eq!(created.email, "john@example.com");
    assert_eq!(created.phone_number, "+358401234567");
    assert!(created.marketing);
    assert!(created.terms);

    let get_response = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer valid-token")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(get_response.status(), StatusCode::OK);
    assert_eq!(
        get_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/cbor")
    );
    let fetched: Profile = read_cbor_body(get_response).await;
    assert_eq!(fetched.id, "user-123");
    assert_eq!(fetched.firstname, "John");

    let update_response = build_app(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer valid-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"firstname":"Jane","marketing":false}"#))
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    assert_eq!(update_response.status(), StatusCode::OK);
    let updated: Profile = read_json_body(update_response).await;
    assert_eq!(updated.firstname, "Jane");
    assert!(!updated.marketing);
    assert_eq!(updated.lastname, "Doe");

    let delete_response = build_app(state.clone())
        .oneshot(authorized_request(
            Method::DELETE,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
    let vary_values = delete_response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>();
    assert!(vary_values.contains(&"Origin"));
    assert!(vary_values.contains(&"Accept"));

    let missing_response = build_app(state)
        .oneshot(authorized_request(
            Method::GET,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");

    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_validation_and_conflict_errors_map_correctly() {
    let state = test_state();

    let invalid_create = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::POST,
            "/v1/profile",
            r#"{"lastname":"Doe","email":"john@example.com","phoneNumber":"+358401234567","terms":true}"#,
        ))
        .await
        .expect("request should succeed");

    assert_eq!(invalid_create.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_problem: ProblemDetails = read_json_body(invalid_create).await;
    assert_eq!(
        invalid_problem.status,
        StatusCode::UNPROCESSABLE_ENTITY.as_u16()
    );

    let invalid_terms = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::POST,
            "/v1/profile",
            r#"{"firstname":"John","lastname":"Doe","email":"john@example.com","phoneNumber":"+358401234567","terms":false}"#,
        ))
        .await
        .expect("request should succeed");

    assert_eq!(invalid_terms.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let create = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::POST,
            "/v1/profile",
            r#"{"firstname":"John","lastname":"Doe","email":"john@example.com","phoneNumber":"+358401234567","terms":true}"#,
        ))
        .await
        .expect("request should succeed");

    assert_eq!(create.status(), StatusCode::CREATED);

    let null_patch = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::PATCH,
            "/v1/profile",
            r#"{"firstname":null,"marketing":true}"#,
        ))
        .await
        .expect("request should succeed");
    assert_eq!(null_patch.status(), StatusCode::BAD_REQUEST);

    let unchanged = build_app(state.clone())
        .oneshot(authorized_request(
            Method::GET,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");
    let unchanged: Profile = read_json_body(unchanged).await;
    assert_eq!(unchanged.firstname, "John");
    assert!(!unchanged.marketing);

    let duplicate = build_app(state.clone())
        .oneshot(authorized_json_request(
            Method::POST,
            "/v1/profile",
            r#"{"firstname":"John","lastname":"Doe","email":"john@example.com","phoneNumber":"+358401234567","terms":true}"#,
        ))
        .await
        .expect("request should succeed");

    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let empty_patch = build_app(state)
        .oneshot(authorized_json_request(Method::PATCH, "/v1/profile", "{}"))
        .await
        .expect("request should succeed");

    assert_eq!(empty_patch.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn profile_auth_certificate_fetch_failure_returns_503() {
    let state = test_state_with_auth_and_profile(
        axum_playground::AuthVerifier::mock(
            MockAuthVerifier::test_user().with_error(AuthError::CertificateFetch),
        ),
        ProfileService::mock(MockProfileService::default()),
    );

    let response = build_app(state)
        .oneshot(authorized_request(
            Method::GET,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("30")
    );
}

#[tokio::test]
async fn profile_auth_lookup_failure_returns_503_without_retry_hint() {
    let state = test_state_with_auth_and_profile(
        axum_playground::AuthVerifier::mock(
            MockAuthVerifier::test_user().with_error(AuthError::ServiceUnavailable),
        ),
        ProfileService::mock(MockProfileService::default()),
    );

    let response = build_app(state)
        .oneshot(authorized_request(
            Method::GET,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers().get(header::RETRY_AFTER), None);
}

#[tokio::test]
async fn profile_backend_errors_return_500() {
    let state = test_state_with_auth_and_profile(
        axum_playground::AuthVerifier::mock(MockAuthVerifier::test_user()),
        ProfileService::mock(MockProfileService::default().with_error(
            ProfileServiceError::Backend("unexpected database error".to_string()),
        )),
    );

    let response = build_app(state)
        .oneshot(authorized_request(
            Method::GET,
            "/v1/profile",
            Body::empty(),
        ))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn openapi_includes_profile_path() {
    let response = build_app(test_state())
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/openapi")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should succeed");

    let body = read_text_body(response).await;
    let document: serde_json::Value =
        serde_json::from_str(&body).expect("OpenAPI document should be JSON");
    let post = &document["paths"]["/v1/profile"]["post"];

    assert_eq!(post["security"][0]["bearerAuth"], serde_json::json!([]));
    assert_eq!(
        document["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
    assert_eq!(
        post["requestBody"]["content"]
            .as_object()
            .expect("request content should be an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["application/cbor", "application/json"]
    );

    let create_schema = &document["components"]["schemas"]["CreateProfileBody"];
    assert_eq!(
        create_schema["required"],
        serde_json::json!(["firstname", "lastname", "email", "phoneNumber", "terms"])
    );
    assert_eq!(create_schema["properties"]["firstname"]["type"], "string");
    assert_eq!(
        create_schema["properties"]["phoneNumber"]["pattern"],
        r"^\+[1-9][0-9]{6,14}$"
    );
    assert_eq!(
        create_schema["properties"]["terms"]["enum"],
        serde_json::json!([true])
    );

    let update_schema = &document["components"]["schemas"]["UpdateProfileBody"];
    assert!(update_schema.get("required").is_none());
    assert_eq!(update_schema["properties"]["firstname"]["type"], "string");

    for method in ["get", "post", "patch", "delete"] {
        let responses = &document["paths"]["/v1/profile"][method]["responses"];
        assert_eq!(
            responses["401"]["$ref"],
            "#/components/responses/UnauthorizedProblemResponse"
        );
        assert_eq!(
            responses["503"]["$ref"],
            "#/components/responses/AuthenticationUnavailableProblemResponse"
        );
    }
    assert_eq!(
        document["components"]["responses"]["UnauthorizedProblemResponse"]["headers"]["WWW-Authenticate"]
            ["schema"]["type"],
        "string"
    );
    assert_eq!(
        document["components"]["responses"]["AuthenticationUnavailableProblemResponse"]["headers"]
            ["Retry-After"]["schema"]["type"],
        "string"
    );
}
