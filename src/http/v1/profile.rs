use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{MethodFilter, on},
};
use serde::{Deserialize, Deserializer};
use utoipa::{ToResponse, ToSchema};

use crate::{
    auth::AuthenticatedUser,
    http::{
        codec::{
            BufferedBody, ResponseFormat, decode_request_body, no_content_response,
            success_response, success_response_with_headers,
        },
        extract::NoQuery,
    },
    problem::{ProblemCode, ProblemDetails, ProblemResponse, problem_response},
    services::profile::{CreateProfileParams, Profile, ProfileServiceError, UpdateProfileParams},
    state::AppState,
    validation::{normalize_contact_email, normalize_phone_number, valid_bounded_name},
};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileCreate {
    #[schema(min_length = 1, max_length = 100)]
    pub first_name: String,
    #[schema(min_length = 1, max_length = 100)]
    pub last_name: String,
    #[schema(max_length = 254)]
    pub contact_email: String,
    #[schema(pattern = r"^\+[1-9][0-9]{6,14}$")]
    pub phone_number: String,
    #[serde(default)]
    #[schema(default = false)]
    pub marketing_opt_in: bool,
    pub terms_accepted: bool,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileUpdate {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(min_length = 1, max_length = 100)]
    pub first_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(min_length = 1, max_length = 100)]
    pub last_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(max_length = 254)]
    pub contact_email: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(pattern = r"^\+[1-9][0-9]{6,14}$")]
    pub phone_number: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    pub marketing_opt_in: Option<bool>,
}

fn deserialize_optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, ToResponse)]
#[response(
    description = "Missing or invalid Firebase bearer authentication",
    headers(("WWW-Authenticate" = String, description = "Bearer authentication challenge"))
)]
pub enum UnauthorizedProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
}

#[derive(Debug, ToResponse)]
#[response(description = "Authentication or persistence dependency unavailable")]
pub enum DependencyUnavailableProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/profile",
        on(MethodFilter::GET, get_profile_handler)
            .on(MethodFilter::POST, create_profile_handler)
            .on(MethodFilter::PATCH, update_profile_handler)
            .on(MethodFilter::DELETE, delete_profile_handler),
    )
}

#[utoipa::path(
    post,
    path = "/v1/profile",
    operation_id = "createProfile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    request_body(content(
        (ProfileCreate = "application/json"),
        (ProfileCreate = "application/cbor")
    )),
    responses(
        (status = 201, description = "Created profile", headers(("Location" = String, description = "Canonical profile resource")), content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 409, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = DependencyUnavailableProblemResponse)
    )
)]
pub async fn create_profile_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    user: AuthenticatedUser,
    headers: HeaderMap,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<ProfileCreate>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };
    let Some(params) = parse_create(input) else {
        return problem_response(ProblemCode::ValidationFailed, &headers);
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
    operation_id = "getProfile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Current profile", content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = DependencyUnavailableProblemResponse)
    )
)]
pub async fn get_profile_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    user: AuthenticatedUser,
    headers: HeaderMap,
) -> Response {
    match state.profile_service.get(&user.0.uid).await {
        Ok(profile) => success_response(StatusCode::OK, format, &profile),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    patch,
    path = "/v1/profile",
    operation_id = "updateProfile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    request_body(content(
        (ProfileUpdate = "application/json"),
        (ProfileUpdate = "application/cbor")
    )),
    responses(
        (status = 200, description = "Updated profile", content((Profile = "application/json"), (Profile = "application/cbor"))),
        (status = 400, response = ProblemResponse),
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = DependencyUnavailableProblemResponse)
    )
)]
pub async fn update_profile_handler(
    State(state): State<Arc<AppState>>,
    format: ResponseFormat,
    _query: NoQuery,
    user: AuthenticatedUser,
    headers: HeaderMap,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<ProfileUpdate>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };
    let Some(params) = parse_update(input) else {
        return problem_response(ProblemCode::ValidationFailed, &headers);
    };

    match state.profile_service.update(&user.0.uid, params).await {
        Ok(profile) => success_response(StatusCode::OK, format, &profile),
        Err(error) => map_service_error(&headers, error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/profile",
    operation_id = "deleteProfile",
    tag = "Profile",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Deleted profile"),
        (status = 400, response = ProblemResponse),
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = DependencyUnavailableProblemResponse)
    )
)]
pub async fn delete_profile_handler(
    State(state): State<Arc<AppState>>,
    _query: NoQuery,
    user: AuthenticatedUser,
    headers: HeaderMap,
) -> Response {
    match state.profile_service.delete(&user.0.uid).await {
        Ok(()) => no_content_response(std::iter::empty()),
        Err(error) => map_service_error(&headers, error),
    }
}

fn parse_create(input: ProfileCreate) -> Option<CreateProfileParams> {
    if !valid_bounded_name(&input.first_name)
        || !valid_bounded_name(&input.last_name)
        || !input.terms_accepted
    {
        return None;
    }
    Some(CreateProfileParams {
        first_name: input.first_name,
        last_name: input.last_name,
        contact_email: normalize_contact_email(&input.contact_email)?,
        phone_number: normalize_phone_number(&input.phone_number)?,
        marketing_opt_in: input.marketing_opt_in,
        terms_accepted: true,
    })
}

fn parse_update(input: ProfileUpdate) -> Option<UpdateProfileParams> {
    if input.first_name.is_none()
        && input.last_name.is_none()
        && input.contact_email.is_none()
        && input.phone_number.is_none()
        && input.marketing_opt_in.is_none()
    {
        return None;
    }
    if input
        .first_name
        .as_deref()
        .is_some_and(|value| !valid_bounded_name(value))
        || input
            .last_name
            .as_deref()
            .is_some_and(|value| !valid_bounded_name(value))
    {
        return None;
    }
    Some(UpdateProfileParams {
        first_name: input.first_name,
        last_name: input.last_name,
        contact_email: match input.contact_email {
            Some(value) => Some(normalize_contact_email(&value)?),
            None => None,
        },
        phone_number: match input.phone_number {
            Some(value) => Some(normalize_phone_number(&value)?),
            None => None,
        },
        marketing_opt_in: input.marketing_opt_in,
    })
}

fn map_service_error(headers: &HeaderMap, error: ProfileServiceError) -> Response {
    match error {
        ProfileServiceError::NotFound => problem_response(ProblemCode::ProfileNotFound, headers),
        ProfileServiceError::AlreadyExists => problem_response(ProblemCode::ProfileExists, headers),
        ProfileServiceError::Unavailable(error) => {
            tracing::warn!(operation = %error.operation(), reason = "unavailable", "profile operation failed");
            problem_response(ProblemCode::DependencyUnavailable, headers)
        }
        ProfileServiceError::Backend(error) => {
            tracing::warn!(operation = %error.operation(), reason = "backend", "profile operation failed");
            problem_response(ProblemCode::InternalError, headers)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProfileCreate, ProfileUpdate, parse_create, parse_update};

    #[test]
    fn create_normalizes_only_contact_fields() {
        let parsed = parse_create(ProfileCreate {
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: " Ada@EXAMPLE.COM\t".to_owned(),
            phone_number: " +358401234567 ".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
        })
        .expect("valid create");
        assert_eq!(parsed.first_name, "Ada");
        assert_eq!(parsed.contact_email, "Ada@example.com");
        assert_eq!(parsed.phone_number, "+358401234567");
    }

    #[test]
    fn update_requires_a_member_and_rejects_noncanonical_names() {
        assert!(parse_update(ProfileUpdate::default()).is_none());
        assert!(
            parse_update(ProfileUpdate {
                first_name: Some(" Ada".to_owned()),
                ..ProfileUpdate::default()
            })
            .is_none()
        );
        assert!(
            parse_update(ProfileUpdate {
                marketing_opt_in: Some(true),
                ..ProfileUpdate::default()
            })
            .is_some()
        );
    }
}
