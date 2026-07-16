use std::sync::Arc;

use axum::{Json, Router, routing::get};
use utoipa::{
    Modify, OpenApi,
    openapi::security::{Http, HttpAuthScheme, SecurityScheme},
};
use utoipa_swagger_ui::{Config, SwaggerUi};

use crate::{
    http::{
        health::__path_health_handler,
        v1::{
            github::{
                __path_get_github_owner_handler, __path_get_github_repo_handler,
                __path_get_github_repo_languages_handler, __path_list_github_owner_repos_handler,
                __path_list_github_repo_activity_handler, __path_list_github_repo_tags_handler,
            },
            hello::{__path_create_hello_handler, __path_get_hello_handler},
            items::__path_list_items_handler,
            profile::{
                __path_create_profile_handler, __path_delete_profile_handler,
                __path_get_profile_handler, __path_update_profile_handler,
            },
        },
    },
    problem::ProblemResponse,
    state::AppState,
};

#[derive(OpenApi)]
#[openapi(
paths(
    health_handler,
    get_hello_handler,
    create_hello_handler,
    list_items_handler,
    create_profile_handler,
    get_profile_handler,
    update_profile_handler,
    delete_profile_handler,
    get_github_owner_handler,
    list_github_owner_repos_handler,
    get_github_repo_handler,
    list_github_repo_activity_handler,
    get_github_repo_languages_handler,
    list_github_repo_tags_handler
),
components(responses(ProblemResponse)),
modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/openapi", get(openapi_handler))
}

pub fn ui_router() -> Router<Arc<AppState>> {
    Router::new().merge(SwaggerUi::new("/api-docs").config(Config::from("/v1/openapi")))
}

async fn openapi_handler() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
