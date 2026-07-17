use axum::{
    extract::{FromRequestParts, Path, Query},
    http::{StatusCode, request::Parts},
    response::Response,
};
use serde::de::DeserializeOwned;

use crate::problem::problem_response;

#[derive(Debug)]
pub struct ProblemPath<T>(pub T);

impl<S, T> FromRequestParts<S> for ProblemPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let headers = parts.headers.clone();
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| {
                problem_response(StatusCode::BAD_REQUEST, "invalid path parameters", &headers)
            })
    }
}

#[derive(Debug)]
pub struct ProblemQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ProblemQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let headers = parts.headers.clone();
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| {
                problem_response(
                    StatusCode::BAD_REQUEST,
                    "invalid query parameters",
                    &headers,
                )
            })
    }
}
