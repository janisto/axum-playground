use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    auth::AuthenticatedUser,
    http::codec::{
        BufferedBody, ResponseFormat, decode_request_body, no_content_response, success_response,
        success_response_with_headers,
    },
    problem::{ProblemResponse, problem_response},
    services::profile::{CreateProfileParams, Profile, ProfileServiceError, UpdateProfileParams},
    state::AppState,
    validation::{valid_email, valid_name, valid_phone_number},
};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileBody {
    pub firstname: Option<String>,
    pub lastname: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    #[serde(default)]
    pub marketing: Option<bool>,
    pub terms: Option<bool>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileBody {
    pub firstname: Option<String>,
    pub lastname: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub marketing: Option<bool>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/profile",
        get(get_profile_handler)
            .post(create_profile_handler)
            .patch(update_profile_handler)
            .delete(delete_profile_handler),
    )
}

#[utoipa::path(
    post,
    path = "/v1/profile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    request_body(content(
        (CreateProfileBody = "application/json"),
        (CreateProfileBody = "application/cbor")
    )),
    responses(
        (status = 201, description = "Created profile", headers(("Location" = String, description = "Canonical profile resource")), content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 401, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 409, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn create_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    format: ResponseFormat,
    headers: HeaderMap,
    user: AuthenticatedUser,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<CreateProfileBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    let params = match parse_create_body(input) {
        Ok(params) => params,
        Err(()) => {
            return problem_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation error",
                &headers,
            );
        }
    };

    match state.profile_service.create(&user.0.uid, params).await {
        Ok(profile) => success_response_with_headers(
            StatusCode::CREATED,
            format,
            &profile,
            [(header::LOCATION, HeaderValue::from_static("/v1/profile"))],
        ),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/profile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Current profile", content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 401, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn get_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    format: ResponseFormat,
    headers: HeaderMap,
    user: AuthenticatedUser,
) -> Response {
    match state.profile_service.get(&user.0.uid).await {
        Ok(profile) => success_response(StatusCode::OK, format, &profile),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    patch,
    path = "/v1/profile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    request_body(content(
        (UpdateProfileBody = "application/json"),
        (UpdateProfileBody = "application/cbor")
    )),
    responses(
        (status = 200, description = "Updated profile", content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 401, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn update_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    format: ResponseFormat,
    headers: HeaderMap,
    user: AuthenticatedUser,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<UpdateProfileBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    let params = match parse_update_body(input) {
        Ok(params) => params,
        Err(()) => {
            return problem_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation error",
                &headers,
            );
        }
    };

    match state.profile_service.update(&user.0.uid, params).await {
        Ok(profile) => success_response(StatusCode::OK, format, &profile),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/profile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Deleted profile"),
        (status = 401, response = ProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 500, response = ProblemResponse)
    )
)]
pub async fn delete_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    user: AuthenticatedUser,
) -> Response {
    match state.profile_service.delete(&user.0.uid).await {
        Ok(()) => no_content_response(std::iter::empty()),
        Err(error) => map_service_error(&headers, error),
    }
}

fn parse_create_body(input: CreateProfileBody) -> Result<CreateProfileParams, ()> {
    let firstname = input.firstname.ok_or(())?;
    let lastname = input.lastname.ok_or(())?;
    let email = input.email.ok_or(())?;
    let phone_number = input.phone_number.ok_or(())?;
    let terms = input.terms.ok_or(())?;

    if !valid_name(&firstname)
        || !valid_name(&lastname)
        || !valid_email(&email)
        || !valid_phone_number(&phone_number)
        || !terms
    {
        return Err(());
    }

    Ok(CreateProfileParams {
        firstname,
        lastname,
        email,
        phone_number,
        marketing: input.marketing.unwrap_or(false),
        terms,
    })
}

fn parse_update_body(input: UpdateProfileBody) -> Result<UpdateProfileParams, ()> {
    if input.firstname.is_none()
        && input.lastname.is_none()
        && input.email.is_none()
        && input.phone_number.is_none()
        && input.marketing.is_none()
    {
        return Err(());
    }

    if input
        .firstname
        .as_deref()
        .is_some_and(|value| !valid_name(value))
        || input
            .lastname
            .as_deref()
            .is_some_and(|value| !valid_name(value))
        || input
            .email
            .as_deref()
            .is_some_and(|value| !valid_email(value))
        || input
            .phone_number
            .as_deref()
            .is_some_and(|value| !valid_phone_number(value))
    {
        return Err(());
    }

    Ok(UpdateProfileParams {
        firstname: input.firstname,
        lastname: input.lastname,
        email: input.email,
        phone_number: input.phone_number,
        marketing: input.marketing,
    })
}

fn map_service_error(headers: &HeaderMap, error: ProfileServiceError) -> Response {
    match error {
        ProfileServiceError::NotFound => {
            problem_response(StatusCode::NOT_FOUND, "profile not found", headers)
        }
        ProfileServiceError::AlreadyExists => {
            problem_response(StatusCode::CONFLICT, "profile already exists", headers)
        }
        ProfileServiceError::Backend(_) => problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error",
            headers,
        ),
    }
}
