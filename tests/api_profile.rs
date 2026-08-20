mod common;

use std::{convert::Infallible, sync::Arc};

use axum::{
    body::{Body, Bytes, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_playground::{
    AuthError, FirebaseUser, MockAuthVerifier, MockGitHubService, MockProfileService, Profile,
    ProfileBackendError, ProfileOperation, ProfileServiceError, build_app,
    problem::{ProblemCode, ProblemDetails},
};
use futures_util::{StreamExt, stream};
use time::macros::datetime;
use tower::ServiceExt;

use crate::common::{read_cbor_body, read_json_body, state_with, test_state};

const CREATE: &str = r#"{"firstName":"Ada","lastName":"Lovelace","contactEmail":" Ada.Lovelace@EXAMPLE.COM\t","phoneNumber":" +358401234567 ","termsAccepted":true}"#;
const BODY_LIMIT: usize = 1_000_000;

fn authorized(method: Method, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri("/v1/profile")
        .header(header::AUTHORIZATION, "Bearer test-token")
        .body(body.into())
        .unwrap()
}

fn authorized_json(method: Method, body: impl Into<Body>) -> Request<Body> {
    let mut request = authorized(method, body);
    request
        .headers_mut()
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    request
}

async fn assert_problem(response: axum::response::Response, status: StatusCode, code: ProblemCode) {
    assert_eq!(response.status(), status);
    let problem: ProblemDetails = if response.headers()[header::CONTENT_TYPE] == "application/cbor"
    {
        read_cbor_body(response).await
    } else {
        read_json_body(response).await
    };
    assert_eq!(problem.status, status.as_u16());
    assert_eq!(problem.code, code);
    assert_eq!(problem.detail, code.detail());
}

fn app_with(profile: MockProfileService) -> axum::Router {
    build_app(state_with(
        MockAuthVerifier::test_user(),
        MockGitHubService::demo(),
        profile,
    ))
}

fn exact_body(size: usize) -> Vec<u8> {
    let mut value = CREATE.as_bytes().to_vec();
    assert!(value.len() <= size);
    value.resize(size, b' ');
    value
}

#[tokio::test]
async fn profile_crud_normalizes_contacts_and_enforces_timestamp_lifecycle() {
    let store = MockProfileService::default();
    let app = app_with(store.clone());

    let created = app
        .clone()
        .oneshot(authorized_json(Method::POST, CREATE))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(created.headers()[header::LOCATION], "/v1/profile");
    let created: Profile = read_json_body(created).await;
    assert_eq!(
        created,
        Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada.Lovelace@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-07-30T12:00:00.000Z".to_owned(),
            updated_at: "2026-07-30T12:00:00.000Z".to_owned(),
        }
    );
    assert_eq!(store.committed_write_count(), 1);

    let fetched = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(read_cbor_body::<Profile>(fetched).await, created);
    assert_eq!(store.committed_write_count(), 1);

    let no_op = app
        .clone()
        .oneshot(authorized_json(
            Method::PATCH,
            r#"{"contactEmail":"Ada.Lovelace@EXAMPLE.COM","marketingOptIn":false}"#,
        ))
        .await
        .unwrap();
    assert_eq!(read_json_body::<Profile>(no_op).await, created);
    assert_eq!(store.committed_write_count(), 1);

    store.set_now(datetime!(2026-07-30 12:05:00.999_999 UTC));
    let changed = app
        .clone()
        .oneshot(authorized_json(
            Method::PATCH,
            r#"{"firstName":"Grace","marketingOptIn":true}"#,
        ))
        .await
        .unwrap();
    let changed: Profile = read_json_body(changed).await;
    assert_eq!(changed.first_name, "Grace");
    assert!(changed.marketing_opt_in);
    assert_eq!(changed.last_name, "Lovelace");
    assert_eq!(changed.created_at, created.created_at);
    assert_eq!(changed.updated_at, "2026-07-30T12:05:00.999Z");
    assert_eq!(store.committed_write_count(), 2);

    store.set_now(datetime!(2025-01-01 0:00 UTC));
    let monotonic = app
        .clone()
        .oneshot(authorized_json(Method::PATCH, r#"{"lastName":"Hopper"}"#))
        .await
        .unwrap();
    let monotonic: Profile = read_json_body(monotonic).await;
    assert_eq!(monotonic.updated_at, "2026-07-30T12:05:01.000Z");

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::ACCEPT, "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(!deleted.headers().contains_key(header::CONTENT_TYPE));
    assert!(!deleted.headers().contains_key(header::CONTENT_LENGTH));
    assert!(to_bytes(deleted.into_body(), 1).await.unwrap().is_empty());

    let missing = app
        .oneshot(authorized(Method::GET, Body::empty()))
        .await
        .unwrap();
    assert_problem(missing, StatusCode::NOT_FOUND, ProblemCode::ProfileNotFound).await;
}

#[tokio::test]
async fn profile_accepts_equivalent_cbor_input_and_response() {
    let mut payload = Vec::new();
    ciborium::into_writer(
        &serde_json::json!({
            "firstName":"Ada", "lastName":"Lovelace",
            "contactEmail":"Ada@EXAMPLE.COM", "phoneNumber":"+358401234567",
            "marketingOptIn":true, "termsAccepted":true
        }),
        &mut payload,
    )
    .unwrap();
    let response = app_with(MockProfileService::default())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::CONTENT_TYPE, "application/cbor")
                .header(header::ACCEPT, "application/cbor")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/cbor");
    let profile: Profile = read_cbor_body(response).await;
    assert_eq!(profile.contact_email, "Ada@example.com");
    assert!(profile.marketing_opt_in);
}

#[tokio::test]
async fn profile_rejects_bearer_field_ambiguity_before_verification_or_persistence() {
    let auth = MockAuthVerifier::test_user();
    let store = MockProfileService::default();
    let app = build_app(state_with(
        auth.clone(),
        MockGitHubService::demo(),
        store.clone(),
    ));

    let simple_invalid = [
        None,
        Some(""),
        Some("Basic abc"),
        Some("Bearer"),
        Some("Bearer "),
        Some("Bearer\ttoken"),
        Some("Bearer token extra"),
        Some("Bearer token,"),
        Some("Bearer token, Bearer other"),
    ];
    for value in simple_invalid {
        let mut request = Request::builder().method(Method::GET).uri("/v1/profile");
        if let Some(value) = value {
            request = request.header(header::AUTHORIZATION, value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{value:?}");
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    }

    let repeated = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Bearer one")
                .header(header::AUTHORIZATION, "Bearer two")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(repeated.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(auth.call_count(), 0);
    assert_eq!(store.operation_count(ProfileOperation::Get), 0);
}

#[tokio::test]
async fn profile_distinguishes_invalid_identity_from_auth_dependency_failure() {
    let invalid_errors = [
        AuthError::InvalidToken,
        AuthError::TokenExpired,
        AuthError::TokenRevoked,
        AuthError::UserDisabled,
    ];
    for error in invalid_errors {
        let auth = MockAuthVerifier::test_user().with_error(error);
        let store = MockProfileService::default();
        let response = build_app(state_with(
            auth.clone(),
            MockGitHubService::demo(),
            store.clone(),
        ))
        .oneshot(authorized(Method::GET, Body::empty()))
        .await
        .unwrap();
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert_problem(
            response,
            StatusCode::UNAUTHORIZED,
            ProblemCode::Unauthorized,
        )
        .await;
        assert_eq!(auth.call_count(), 1);
        assert_eq!(store.operation_count(ProfileOperation::Get), 0);
    }

    for error in [AuthError::CertificateFetch, AuthError::ServiceUnavailable] {
        let auth = MockAuthVerifier::test_user().with_error(error);
        let store = MockProfileService::default();
        let response = build_app(state_with(
            auth.clone(),
            MockGitHubService::demo(),
            store.clone(),
        ))
        .oneshot(authorized(Method::GET, Body::empty()))
        .await
        .unwrap();
        assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        assert_problem(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            ProblemCode::DependencyUnavailable,
        )
        .await;
        assert_eq!(auth.call_count(), 1);
        assert_eq!(store.operation_count(ProfileOperation::Get), 0);
    }
}

#[tokio::test]
async fn profile_route_query_negotiation_size_and_auth_order_is_fail_closed() {
    let auth = MockAuthVerifier::test_user();
    let store = MockProfileService::default();
    let app = build_app(state_with(
        auth.clone(),
        MockGitHubService::demo(),
        store.clone(),
    ));

    for target in [
        "/v1/profile?owner=other",
        "/v1/profile?x=1&x=2",
        "/v1/profile?x=%",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(target)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{target}");
    }
    assert_eq!(auth.call_count(), 0);

    let unacceptable = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/profile")
                .header(header::ACCEPT, "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unacceptable.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(auth.call_count(), 0);

    let declared = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/profile")
                .header(header::CONTENT_LENGTH, BODY_LIMIT + 1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(declared.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(auth.call_count(), 0);

    let auth_failure = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/profile")
                .header(header::AUTHORIZATION, "Basic invalid")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(auth_failure.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(store.operation_count(ProfileOperation::Create), 0);
}

#[tokio::test]
async fn profile_validation_and_parse_failures_do_not_reach_persistence() {
    let schema_cases = [
        r#"{}"#,
        r#"{"firstName":null}"#,
        r#"{"firstName":3}"#,
        r#"{"firstName":" Ada","lastName":"Lovelace","contactEmail":"a@example.com","phoneNumber":"+358401234567","termsAccepted":true}"#,
        r#"{"firstName":"Ada","lastName":"Lovelace","contactEmail":"a@example","phoneNumber":"+358401234567","termsAccepted":true}"#,
        r#"{"firstName":"Ada","lastName":"Lovelace","contactEmail":"a@example.com","phoneNumber":"+01234567","termsAccepted":true}"#,
        r#"{"firstName":"Ada","lastName":"Lovelace","contactEmail":"a@example.com","phoneNumber":"+358401234567","termsAccepted":false}"#,
        r#"{"firstName":"Ada","lastName":"Lovelace","contactEmail":"a@example.com","phoneNumber":"+358401234567","termsAccepted":true,"id":"other"}"#,
    ];
    for body in schema_cases {
        let store = MockProfileService::default();
        let response = app_with(store.clone())
            .oneshot(authorized_json(Method::POST, body))
            .await
            .unwrap();
        assert_problem(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
        )
        .await;
        assert_eq!(store.operation_count(ProfileOperation::Create), 0);
    }

    for body in [
        r#"{"firstName":"Ada","firstName":"Grace"}"#,
        r#"{"firstName":}"#,
        r#"{} null"#,
    ] {
        let store = MockProfileService::default();
        let response = app_with(store.clone())
            .oneshot(authorized_json(Method::POST, body))
            .await
            .unwrap();
        assert_problem(
            response,
            StatusCode::BAD_REQUEST,
            ProblemCode::InvalidRequest,
        )
        .await;
        assert_eq!(store.operation_count(ProfileOperation::Create), 0);
    }

    let store = MockProfileService::default();
    let app = app_with(store.clone());
    assert_eq!(
        app.clone()
            .oneshot(authorized_json(Method::POST, CREATE))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let writes = store.committed_write_count();
    for body in [
        "{}",
        r#"{"firstName":null}"#,
        r#"{"termsAccepted":true}"#,
        r#"{"updatedAt":"2026-01-01T00:00:00.000Z"}"#,
        r#"{"contactEmail":"not-an-email"}"#,
    ] {
        let response = app
            .clone()
            .oneshot(authorized_json(Method::PATCH, body))
            .await
            .unwrap();
        assert_problem(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
        )
        .await;
    }
    assert_eq!(store.committed_write_count(), writes);
    assert_eq!(store.operation_count(ProfileOperation::Update), 0);
}

#[tokio::test]
async fn profile_body_limits_preserve_authentication_then_suppress_persistence() {
    for size in [999_999, BODY_LIMIT] {
        let response = app_with(MockProfileService::default())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/profile")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::CONTENT_LENGTH, size)
                    .body(Body::from(exact_body(size)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "size {size}");
    }

    let auth = MockAuthVerifier::test_user();
    let store = MockProfileService::default();
    let overflow =
        stream::once(async { Ok::<Bytes, Infallible>(Bytes::from(vec![b'x'; BODY_LIMIT + 1])) })
            .chain(stream::once(async {
                panic!("profile body reader polled after overflow");
                #[allow(unreachable_code)]
                Ok::<Bytes, Infallible>(Bytes::new())
            }));
    let response = build_app(state_with(
        auth.clone(),
        MockGitHubService::demo(),
        store.clone(),
    ))
    .oneshot(
        Request::builder()
            .method(Method::POST)
            .uri("/v1/profile")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from_stream(overflow))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_problem(
        response,
        StatusCode::PAYLOAD_TOO_LARGE,
        ProblemCode::PayloadTooLarge,
    )
    .await;
    assert_eq!(auth.call_count(), 1);
    assert_eq!(store.operation_count(ProfileOperation::Create), 0);
    assert_eq!(store.committed_write_count(), 0);
}

#[tokio::test]
async fn profile_concurrent_creates_and_deletes_have_one_atomic_winner() {
    let store = MockProfileService::default();
    let app = app_with(store.clone());
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let spawn_create =
        |app: axum::Router, barrier: Arc<tokio::sync::Barrier>, name: &'static str| {
            tokio::spawn(async move {
                barrier.wait().await;
                app.oneshot(authorized_json(Method::POST, CREATE.replace("Ada", name)))
                    .await
                    .unwrap()
            })
        };
    let left = spawn_create(app.clone(), Arc::clone(&barrier), "Ada");
    let right = spawn_create(app.clone(), Arc::clone(&barrier), "Grace");
    barrier.wait().await;
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|response| response.status() == StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|response| response.status() == StatusCode::CONFLICT)
            .count(),
        1
    );
    let winner = store
        .stored_profile("user-123")
        .expect("winner should be committed");
    assert!(matches!(winner.first_name.as_str(), "Ada" | "Grace"));
    assert_eq!(store.committed_write_count(), 1);
    assert_eq!(store.operation_count(ProfileOperation::Create), 2);

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let spawn_delete = |app: axum::Router, barrier: Arc<tokio::sync::Barrier>| {
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(authorized(Method::DELETE, Body::empty()))
                .await
                .unwrap()
        })
    };
    let left = spawn_delete(app.clone(), Arc::clone(&barrier));
    let right = spawn_delete(app.clone(), Arc::clone(&barrier));
    barrier.wait().await;
    let outcomes = [left.await.unwrap(), right.await.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|response| response.status() == StatusCode::NO_CONTENT)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|response| response.status() == StatusCode::NOT_FOUND)
            .count(),
        1
    );
    assert!(store.stored_profile("user-123").is_none());
}

#[tokio::test]
async fn profile_patch_delete_race_never_recreates_a_deleted_profile() {
    let store = MockProfileService::default();
    let app = app_with(store.clone());
    assert_eq!(
        app.clone()
            .oneshot(authorized_json(Method::POST, CREATE))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let patch_app = app.clone();
    let patch_barrier = Arc::clone(&barrier);
    let patch = tokio::spawn(async move {
        patch_barrier.wait().await;
        patch_app
            .oneshot(authorized_json(Method::PATCH, r#"{"marketingOptIn":true}"#))
            .await
            .unwrap()
    });
    let delete_app = app.clone();
    let delete_barrier = Arc::clone(&barrier);
    let delete = tokio::spawn(async move {
        delete_barrier.wait().await;
        delete_app
            .oneshot(authorized(Method::DELETE, Body::empty()))
            .await
            .unwrap()
    });
    barrier.wait().await;
    let patch = patch.await.unwrap();
    let delete = delete.await.unwrap();
    assert!(matches!(
        patch.status(),
        StatusCode::OK | StatusCode::NOT_FOUND
    ));
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);
    assert!(store.stored_profile("user-123").is_none());
    let read = app
        .oneshot(authorized(Method::GET, Body::empty()))
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn profile_principals_are_isolated_and_input_cannot_select_ownership() {
    let store = MockProfileService::default();
    let first = build_app(state_with(
        MockAuthVerifier::allow(FirebaseUser::new("tenant/user", "", false)),
        MockGitHubService::demo(),
        store.clone(),
    ));
    let second = build_app(state_with(
        MockAuthVerifier::allow(FirebaseUser::new("other-user", "", false)),
        MockGitHubService::demo(),
        store.clone(),
    ));
    let a: Profile = read_json_body(
        first
            .oneshot(authorized_json(Method::POST, CREATE))
            .await
            .unwrap(),
    )
    .await;
    let b: Profile = read_json_body(
        second
            .oneshot(authorized_json(
                Method::POST,
                CREATE.replace("Ada", "Grace"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(a.id, "tenant/user");
    assert_eq!(b.id, "other-user");
    assert_eq!(store.committed_write_count(), 2);
}

#[tokio::test]
async fn profile_persistence_errors_and_timestamp_exhaustion_map_without_leaks_or_writes() {
    for (error, status, code) in [
        (
            ProfileServiceError::Unavailable(ProfileBackendError::new(
                ProfileOperation::Get,
                std::io::Error::other("secret unavailable"),
            )),
            StatusCode::SERVICE_UNAVAILABLE,
            ProblemCode::DependencyUnavailable,
        ),
        (
            ProfileServiceError::Backend(ProfileBackendError::new(
                ProfileOperation::Get,
                std::io::Error::other("secret backend"),
            )),
            StatusCode::INTERNAL_SERVER_ERROR,
            ProblemCode::InternalError,
        ),
    ] {
        let store = MockProfileService::default().with_error(error);
        let response = app_with(store.clone())
            .oneshot(authorized(Method::GET, Body::empty()))
            .await
            .unwrap();
        assert_problem(response, status, code).await;
        assert_eq!(store.committed_write_count(), 0);
    }

    let max = Profile {
        id: "user-123".to_owned(),
        first_name: "Ada".to_owned(),
        last_name: "Lovelace".to_owned(),
        contact_email: "Ada@example.com".to_owned(),
        phone_number: "+358401234567".to_owned(),
        marketing_opt_in: false,
        terms_accepted: true,
        created_at: "9999-12-31T23:59:59.999Z".to_owned(),
        updated_at: "9999-12-31T23:59:59.999Z".to_owned(),
    };
    let store = MockProfileService::default().with_profile(max.clone());
    let response = app_with(store.clone())
        .oneshot(authorized_json(Method::PATCH, r#"{"marketingOptIn":true}"#))
        .await
        .unwrap();
    assert_problem(
        response,
        StatusCode::INTERNAL_SERVER_ERROR,
        ProblemCode::InternalError,
    )
    .await;
    assert_eq!(store.committed_write_count(), 0);
    assert_eq!(store.stored_profile("user-123"), Some(max));
}

#[tokio::test]
async fn profile_body_free_and_method_boundaries_are_exact() {
    let app = build_app(test_state());
    let body = Body::from_stream(stream::once(async {
        panic!("profile GET polled request content");
        #[allow(unreachable_code)]
        Ok::<Bytes, Infallible>(Bytes::new())
    }));
    let response = app
        .clone()
        .oneshot(authorized(Method::GET, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let method = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/v1/profile")
                .body(Body::from(vec![0; BODY_LIMIT + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(method.headers()[header::ALLOW], "GET, POST, PATCH, DELETE");
}
