pub mod docs;
pub mod github;
pub mod hello;
pub mod items;
pub mod profile;

use std::sync::Arc;

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .merge(docs::router())
        .merge(github::router())
        .merge(hello::router())
        .merge(items::router())
        .merge(profile::router())
}
