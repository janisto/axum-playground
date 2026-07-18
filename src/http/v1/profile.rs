use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Deserializer};
use utoipa::{
    ToResponse, ToSchema,
    openapi::schema::{Object, ObjectBuilder, Type},
};

use crate::{
    auth::AuthenticatedUser,
    http::codec::{
        BufferedBody, ResponseFormat, decode_request_body, no_content_response, success_response,
        success_response_with_headers,
    },
    problem::{ProblemDetails, ProblemResponse, problem_response},
    services::profile::{CreateProfileParams, Profile, ProfileServiceError, UpdateProfileParams},
    state::AppState,
    validation::{valid_email, valid_name, valid_phone_number},
};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileBody {
    #[schema(required = true, value_type = String, min_length = 1, max_length = 100)]
    pub firstname: Option<String>,
    #[schema(required = true, value_type = String, min_length = 1, max_length = 100)]
    pub lastname: Option<String>,
    #[schema(required = true, value_type = String)]
    pub email: Option<String>,
    #[schema(required = true, value_type = String, pattern = r"^\+[1-9][0-9]{6,14}$")]
    pub phone_number: Option<String>,
    #[serde(default)]
    #[schema(value_type = bool, default = false)]
    pub marketing: Option<bool>,
    #[schema(required = true, schema_with = accepted_terms_schema)]
    pub terms: Option<bool>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProfileBody {
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(required = false, value_type = String, min_length = 1, max_length = 100)]
    pub firstname: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(required = false, value_type = String, min_length = 1, max_length = 100)]
    pub lastname: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(required = false, value_type = String)]
    pub email: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(required = false, value_type = String, pattern = r"^\+[1-9][0-9]{6,14}$")]
    pub phone_number: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_non_null")]
    #[schema(required = false, value_type = bool)]
    pub marketing: Option<bool>,
}

fn deserialize_optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn accepted_terms_schema() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::Boolean)
        .enum_values(Some([true]))
        .description(Some("Must be true to accept the terms"))
        .build()
}

#[derive(Debug, ToResponse)]
#[response(
    description = "Missing or invalid bearer authentication",
    headers(("WWW-Authenticate" = String, description = "Bearer authentication challenge"))
)]
pub enum UnauthorizedProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
}

#[derive(Debug, ToResponse)]
#[response(
    description = "Authentication dependency temporarily unavailable",
    headers(("Retry-After" = String, description = "May indicate when certificate retrieval can be retried"))
)]
pub enum AuthenticationUnavailableProblemResponse {
    Json(#[content("application/problem+json")] ProblemDetails),
    Cbor(#[content("application/cbor")] ProblemDetails),
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
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 409, response = ProblemResponse),
        (status = 413, response = ProblemResponse),
        (status = 415, response = ProblemResponse),
        (status = 422, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = AuthenticationUnavailableProblemResponse)
    )
)]
pub async fn create_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    user: AuthenticatedUser,
    format: ResponseFormat,
    headers: HeaderMap,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<CreateProfileBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    let Ok(params) = parse_create_body(input) else {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
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
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 406, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = AuthenticationUnavailableProblemResponse)
    )
)]
pub async fn get_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    user: AuthenticatedUser,
    format: ResponseFormat,
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
    tag = "Profile",
    security(("bearerAuth" = [])),
    request_body(content(
        (UpdateProfileBody = "application/json"),
        (UpdateProfileBody = "application/cbor")
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
        (status = 503, response = AuthenticationUnavailableProblemResponse)
    )
)]
pub async fn update_profile_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    user: AuthenticatedUser,
    format: ResponseFormat,
    headers: HeaderMap,
    BufferedBody(body): BufferedBody,
) -> Response {
    let input = match decode_request_body::<UpdateProfileBody>(&headers, body) {
        Ok(input) => input,
        Err(error) => return error.into_response(&headers),
    };

    let Ok(params) = parse_update_body(input) else {
        return problem_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation error",
            &headers,
        );
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
        (status = 401, response = UnauthorizedProblemResponse),
        (status = 404, response = ProblemResponse),
        (status = 500, response = ProblemResponse),
        (status = 503, response = AuthenticationUnavailableProblemResponse)
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

#[allow(
    clippy::needless_pass_by_value,
    reason = "the handler transfers ownership of the service failure"
)]
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

#[cfg(test)]
mod tests {
    use super::{CreateProfileBody, UpdateProfileBody, parse_create_body, parse_update_body};

    #[test]
    fn create_profile_validates_each_required_business_rule_independently() {
        let invalid_inputs = [
            CreateProfileBody {
                firstname: Some(String::new()),
                ..valid_create_body()
            },
            CreateProfileBody {
                lastname: Some(String::new()),
                ..valid_create_body()
            },
            CreateProfileBody {
                email: Some("not-an-email".to_owned()),
                ..valid_create_body()
            },
            CreateProfileBody {
                phone_number: Some("12345".to_owned()),
                ..valid_create_body()
            },
            CreateProfileBody {
                terms: Some(false),
                ..valid_create_body()
            },
        ];

        for input in invalid_inputs {
            assert!(parse_create_body(input).is_err());
        }
    }

    #[test]
    fn update_profile_accepts_each_single_field_and_rejects_each_invalid_value() {
        let valid_updates = [
            UpdateProfileBody {
                firstname: Some("Jane".to_owned()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                lastname: Some("Smith".to_owned()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                email: Some("jane@example.com".to_owned()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                phone_number: Some("+358401234567".to_owned()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                marketing: Some(true),
                ..UpdateProfileBody::default()
            },
        ];
        for input in valid_updates {
            assert!(parse_update_body(input).is_ok());
        }

        assert!(parse_update_body(UpdateProfileBody::default()).is_err());
        for input in [
            UpdateProfileBody {
                firstname: Some(String::new()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                lastname: Some(String::new()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                email: Some("not-an-email".to_owned()),
                ..UpdateProfileBody::default()
            },
            UpdateProfileBody {
                phone_number: Some("12345".to_owned()),
                ..UpdateProfileBody::default()
            },
        ] {
            assert!(parse_update_body(input).is_err());
        }
    }

    fn valid_create_body() -> CreateProfileBody {
        CreateProfileBody {
            firstname: Some("John".to_owned()),
            lastname: Some("Doe".to_owned()),
            email: Some("john@example.com".to_owned()),
            phone_number: Some("+358401234567".to_owned()),
            marketing: Some(false),
            terms: Some(true),
        }
    }
}
