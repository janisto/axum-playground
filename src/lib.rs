//! Reusable application composition and HTTP contracts for axum-playground.

#![forbid(unsafe_code)]

pub mod app;
pub mod auth;
pub mod config;
pub mod error;
pub mod http;
pub mod middleware;
pub mod pagination;
pub mod problem;
pub mod profile_migration;
pub mod services;
pub mod shutdown;
pub mod state;
pub mod telemetry;
pub mod validation;

pub use app::build_app;
pub use app::build_app_with_routes;
pub use auth::{
    AuthError, AuthVerifier, AuthenticatedUser, FirebaseUser, MockAuthVerifier,
    extract_bearer_token,
};
pub use config::{AppConfig, AppEnvironment};
pub use services::github::{
    Activity as GitHubActivity, GitHubService, GitHubServiceError, GitHubUpstreamError,
    GitHubUpstreamErrorKind, Language as GitHubLanguage, MockGitHubService, Owner as GitHubOwner,
    Repository as GitHubRepository, RepositorySummary as GitHubRepositorySummary, Tag as GitHubTag,
    TagCommit as GitHubTagCommit,
};
pub use services::profile::{
    CreateProfileParams, MockProfileService, Profile, ProfileBackendError, ProfileOperation,
    ProfileService, ProfileServiceError, UpdateProfileParams,
};
pub use state::AppState;
