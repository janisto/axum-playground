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
pub use config::AppConfig;
pub use services::github::{
    Activity as GitHubActivity, ActivityPage as GitHubActivityPage, GitHubService,
    GitHubServiceError, GitHubUpstreamError, GitHubUpstreamErrorKind, Language as GitHubLanguage,
    MockGitHubService, Owner as GitHubOwner, Repo as GitHubRepo, RepoSummary as GitHubRepoSummary,
    Tag as GitHubTag, TagCommit as GitHubTagCommit,
};
pub use services::profile::{
    CreateProfileParams, MockProfileService, Profile, ProfileService, ProfileServiceError,
    UpdateProfileParams,
};
pub use state::AppState;
