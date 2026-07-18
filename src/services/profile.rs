use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use firestore::{
    FirestoreDb, FirestoreDbOptions, FirestoreWritePrecondition, errors::FirestoreError,
};
use gcloud_sdk::{ExternalJwtFunctionSource, Token, TokenSourceType};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::OnceCell;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::config::AppConfig;

const PROFILES_COLLECTION: &str = "profiles";
const FIRESTORE_API_URL: &str = "https://firestore.googleapis.com";
const EMULATOR_BEARER_TOKEN: &str = "owner";
const EMULATOR_TOKEN_EXPIRY: &str = "9999-12-31T23:59:59Z";

#[derive(Clone, Debug)]
pub struct ProfileService {
    inner: Arc<ProfileServiceInner>,
}

#[derive(Clone, Debug)]
enum ProfileServiceInner {
    Firestore(Box<FirestoreProfileStore>),
    Mock(Box<MockProfileService>),
}

#[derive(Clone, Debug)]
struct FirestoreProfileStore {
    project_id: String,
    emulator_host: Option<String>,
    db: Arc<OnceCell<FirestoreDb>>,
}

#[derive(Clone, Debug, Default)]
pub struct MockProfileService {
    profiles: Arc<Mutex<BTreeMap<String, Profile>>>,
    error: Option<ProfileServiceError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub firstname: String,
    pub lastname: String,
    pub email: String,
    pub phone_number: String,
    pub marketing: bool,
    pub terms: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateProfileParams {
    pub firstname: String,
    pub lastname: String,
    pub email: String,
    pub phone_number: String,
    pub marketing: bool,
    pub terms: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateProfileParams {
    pub firstname: Option<String>,
    pub lastname: Option<String>,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub marketing: Option<bool>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileServiceError {
    #[error("profile not found")]
    NotFound,
    #[error("profile already exists")]
    AlreadyExists,
    #[error("{0}")]
    Backend(String),
}

impl ProfileService {
    #[must_use]
    pub fn firestore(config: &AppConfig) -> Self {
        let project_id = config.resolved_google_project_id().to_owned();

        Self {
            inner: Arc::new(ProfileServiceInner::Firestore(Box::new(
                FirestoreProfileStore {
                    project_id,
                    emulator_host: config.firestore_emulator_host.clone(),
                    db: Arc::new(OnceCell::new()),
                },
            ))),
        }
    }

    #[must_use]
    pub fn mock(mock: MockProfileService) -> Self {
        Self {
            inner: Arc::new(ProfileServiceInner::Mock(Box::new(mock))),
        }
    }

    pub async fn create(
        &self,
        user_id: &str,
        params: CreateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.create(user_id, params).await,
            ProfileServiceInner::Mock(store) => store.create(user_id, params).await,
        }
    }

    pub async fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.get(user_id).await,
            ProfileServiceInner::Mock(store) => store.get(user_id).await,
        }
    }

    pub async fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.update(user_id, params).await,
            ProfileServiceInner::Mock(store) => store.update(user_id, params).await,
        }
    }

    pub async fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.delete(user_id).await,
            ProfileServiceInner::Mock(store) => store.delete(user_id).await,
        }
    }
}

impl MockProfileService {
    #[must_use]
    pub fn with_error(mut self, error: ProfileServiceError) -> Self {
        self.error = Some(error);
        self
    }

    #[must_use]
    pub fn with_profile(self, profile: Profile) -> Self {
        self.profiles
            .lock()
            .expect("mock profile map lock should succeed")
            .insert(profile.id.clone(), profile);
        self
    }

    async fn create(
        &self,
        user_id: &str,
        params: CreateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        let mut profiles = self
            .profiles
            .lock()
            .expect("mock profile map lock should succeed");
        if profiles.contains_key(user_id) {
            return Err(ProfileServiceError::AlreadyExists);
        }

        let profile = build_profile(user_id, params);
        profiles.insert(user_id.to_owned(), profile.clone());
        Ok(profile)
    }

    async fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        self.profiles
            .lock()
            .expect("mock profile map lock should succeed")
            .get(user_id)
            .cloned()
            .ok_or(ProfileServiceError::NotFound)
    }

    async fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        let mut profiles = self
            .profiles
            .lock()
            .expect("mock profile map lock should succeed");
        let profile = profiles
            .get_mut(user_id)
            .ok_or(ProfileServiceError::NotFound)?;

        apply_update(profile, params);
        Ok(profile.clone())
    }

    async fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        let removed = self
            .profiles
            .lock()
            .expect("mock profile map lock should succeed")
            .remove(user_id);

        if removed.is_some() {
            Ok(())
        } else {
            Err(ProfileServiceError::NotFound)
        }
    }
}

impl FirestoreProfileStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        let profile = build_profile(user_id, params);
        let document_id = profile_document_id(user_id);
        let db = self.db().await?;

        match db
            .fluent()
            .insert()
            .into(PROFILES_COLLECTION)
            .document_id(&document_id)
            .object(&profile)
            .execute::<Profile>()
            .await
        {
            Ok(created) => {
                info!(operation = "profile.create", "profile mutation succeeded");
                Ok(created)
            }
            Err(error) => {
                let error = map_firestore_error(error, ProfileMutation::Create);
                warn!(
                    operation = "profile.create",
                    reason = profile_error_kind(&error),
                    "profile mutation failed"
                );
                Err(error)
            }
        }
    }

    async fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let profile: Option<Profile> = db
            .fluent()
            .select()
            .by_id_in(PROFILES_COLLECTION)
            .obj()
            .one(&document_id)
            .await
            .map_err(|error| map_firestore_error(error, ProfileMutation::Get))?;

        profile.ok_or(ProfileServiceError::NotFound)
    }

    async fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let profile: Option<Profile> = db
            .fluent()
            .select()
            .by_id_in(PROFILES_COLLECTION)
            .obj()
            .one(&document_id)
            .await
            .map_err(|error| map_firestore_error(error, ProfileMutation::Update))?;

        let Some(mut profile) = profile else {
            return Err(ProfileServiceError::NotFound);
        };

        let fields = update_field_mask(&params);
        apply_update(&mut profile, params);

        match db
            .fluent()
            .update()
            .fields(fields.iter().map(String::as_str))
            .in_col(PROFILES_COLLECTION)
            .precondition(FirestoreWritePrecondition::Exists(true))
            .document_id(&document_id)
            .object(&profile)
            .execute::<Profile>()
            .await
        {
            Ok(updated) => {
                info!(operation = "profile.update", "profile mutation succeeded");
                Ok(updated)
            }
            Err(error) => {
                let error = map_firestore_error(error, ProfileMutation::Update);
                warn!(
                    operation = "profile.update",
                    reason = profile_error_kind(&error),
                    "profile mutation failed"
                );
                Err(error)
            }
        }
    }

    async fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);

        match db
            .fluent()
            .delete()
            .from(PROFILES_COLLECTION)
            .document_id(&document_id)
            .precondition(FirestoreWritePrecondition::Exists(true))
            .execute()
            .await
        {
            Ok(_) => {
                info!(operation = "profile.delete", "profile mutation succeeded");
                Ok(())
            }
            Err(error) => {
                let error = map_firestore_error(error, ProfileMutation::Delete);
                warn!(
                    operation = "profile.delete",
                    reason = profile_error_kind(&error),
                    "profile mutation failed"
                );
                Err(error)
            }
        }
    }

    async fn db(&self) -> Result<&FirestoreDb, ProfileServiceError> {
        self.db
            .get_or_try_init(|| async {
                new_firestore_db(&self.project_id, self.emulator_host.as_deref()).await
            })
            .await
    }
}

async fn new_firestore_db(
    project_id: &str,
    emulator_host: Option<&str>,
) -> Result<FirestoreDb, ProfileServiceError> {
    let options = firestore_db_options(project_id, emulator_host);
    let db = if emulator_host.is_some() {
        // The emulator accepts an owner token; never forward real ADC credentials to localhost.
        let token_source = ExternalJwtFunctionSource::new(|| async {
            let expiry = EMULATOR_TOKEN_EXPIRY
                .parse()
                .expect("static emulator token expiry should be valid");
            Ok(Token::new(
                "Bearer".to_owned(),
                EMULATOR_BEARER_TOKEN.into(),
                expiry,
            ))
        });
        FirestoreDb::with_options_token_source(
            options,
            Vec::new(),
            TokenSourceType::ExternalSource(Box::new(token_source)),
        )
        .await
    } else {
        FirestoreDb::with_options(options).await
    };

    db.map_err(|error| ProfileServiceError::Backend(error.to_string()))
}

fn firestore_db_options(project_id: &str, emulator_host: Option<&str>) -> FirestoreDbOptions {
    let api_url = emulator_host
        .map(|host| format!("http://{host}"))
        .unwrap_or_else(|| FIRESTORE_API_URL.to_owned());

    FirestoreDbOptions::new(project_id.to_owned()).with_firebase_api_url(api_url)
}

fn profile_error_kind(error: &ProfileServiceError) -> &'static str {
    match error {
        ProfileServiceError::NotFound => "not_found",
        ProfileServiceError::AlreadyExists => "already_exists",
        ProfileServiceError::Backend(_) => "backend",
    }
}

fn profile_document_id(user_id: &str) -> String {
    format!("uid_{}", URL_SAFE_NO_PAD.encode(user_id))
}

#[derive(Clone, Copy, Debug)]
enum ProfileMutation {
    Create,
    Get,
    Update,
    Delete,
}

fn build_profile(user_id: &str, params: CreateProfileParams) -> Profile {
    let timestamp = timestamp_now();
    Profile {
        id: user_id.to_owned(),
        firstname: params.firstname,
        lastname: params.lastname,
        email: normalize_email(&params.email),
        phone_number: normalize_phone(&params.phone_number),
        marketing: params.marketing,
        terms: params.terms,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

fn apply_update(profile: &mut Profile, params: UpdateProfileParams) {
    if let Some(firstname) = params.firstname {
        profile.firstname = firstname;
    }
    if let Some(lastname) = params.lastname {
        profile.lastname = lastname;
    }
    if let Some(email) = params.email {
        profile.email = normalize_email(&email);
    }
    if let Some(phone_number) = params.phone_number {
        profile.phone_number = normalize_phone(&phone_number);
    }
    if let Some(marketing) = params.marketing {
        profile.marketing = marketing;
    }
    profile.updated_at = timestamp_now();
}

fn update_field_mask(params: &UpdateProfileParams) -> Vec<String> {
    let mut fields = Vec::new();
    if params.firstname.is_some() {
        fields.push("firstname".to_owned());
    }
    if params.lastname.is_some() {
        fields.push("lastname".to_owned());
    }
    if params.email.is_some() {
        fields.push("email".to_owned());
    }
    if params.phone_number.is_some() {
        fields.push("phoneNumber".to_owned());
    }
    if params.marketing.is_some() {
        fields.push("marketing".to_owned());
    }
    fields.push("updatedAt".to_owned());
    fields
}

fn normalize_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_phone(value: &str) -> String {
    value.trim().to_owned()
}

fn timestamp_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("timestamp should format as rfc3339")
}

fn map_firestore_error(error: FirestoreError, mutation: ProfileMutation) -> ProfileServiceError {
    match error {
        FirestoreError::DataConflictError(_) => match mutation {
            ProfileMutation::Create => ProfileServiceError::AlreadyExists,
            ProfileMutation::Update | ProfileMutation::Delete => ProfileServiceError::NotFound,
            ProfileMutation::Get => ProfileServiceError::Backend(error.to_string()),
        },
        FirestoreError::DataNotFoundError(_) => ProfileServiceError::NotFound,
        other => ProfileServiceError::Backend(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    use super::{
        CreateProfileParams, FIRESTORE_API_URL, MockProfileService, Profile, ProfileService,
        ProfileServiceError, UpdateProfileParams, firestore_db_options, profile_document_id,
        profile_error_kind, timestamp_now, update_field_mask,
    };

    #[test]
    fn firestore_document_id_safely_encodes_opaque_firebase_uids() {
        assert_eq!(profile_document_id("tenant/user"), "uid_dGVuYW50L3VzZXI");
        assert_eq!(profile_document_id("."), "uid_Lg");
        assert_eq!(profile_document_id(".."), "uid_Li4");

        for user_id in ["tenant/user", ".", "..", "\u{ffff}"] {
            let document_id = profile_document_id(user_id);
            assert!(!document_id.contains('/'));
            assert_ne!(document_id, ".");
            assert_ne!(document_id, "..");
            assert!(!document_id.starts_with("__"));
        }
    }

    #[test]
    fn firestore_endpoint_is_derived_only_from_validated_config() {
        let production = firestore_db_options("project", None);
        assert_eq!(
            production.firebase_api_url.as_deref(),
            Some(FIRESTORE_API_URL)
        );

        let emulator = firestore_db_options("project", Some("127.0.0.1:8085"));
        assert_eq!(
            emulator.firebase_api_url.as_deref(),
            Some("http://127.0.0.1:8085")
        );
    }

    #[tokio::test]
    async fn mock_service_normalizes_email_and_phone() {
        let service = ProfileService::mock(MockProfileService::default());
        let profile = service
            .create(
                "user-123",
                CreateProfileParams {
                    firstname: "John".to_owned(),
                    lastname: "Doe".to_owned(),
                    email: "  JOHN@EXAMPLE.COM  ".to_owned(),
                    phone_number: "  +358401234567  ".to_owned(),
                    marketing: true,
                    terms: true,
                },
            )
            .await
            .expect("profile should be created");

        assert_eq!(profile.email, "john@example.com");
        assert_eq!(profile.phone_number, "+358401234567");
    }

    #[tokio::test]
    async fn mock_service_rejects_duplicate_create() {
        let service = ProfileService::mock(MockProfileService::default());

        service
            .create(
                "user-123",
                CreateProfileParams {
                    firstname: "John".to_owned(),
                    lastname: "Doe".to_owned(),
                    email: "john@example.com".to_owned(),
                    phone_number: "+358401234567".to_owned(),
                    marketing: false,
                    terms: true,
                },
            )
            .await
            .expect("first create should succeed");

        let error = service
            .create(
                "user-123",
                CreateProfileParams {
                    firstname: "Jane".to_owned(),
                    lastname: "Doe".to_owned(),
                    email: "jane@example.com".to_owned(),
                    phone_number: "+358401234567".to_owned(),
                    marketing: false,
                    terms: true,
                },
            )
            .await
            .expect_err("duplicate create should fail");

        assert_eq!(error, ProfileServiceError::AlreadyExists);
    }

    #[tokio::test]
    async fn mock_service_updates_selected_fields() {
        let service = ProfileService::mock(MockProfileService::default());
        service
            .create(
                "user-123",
                CreateProfileParams {
                    firstname: "John".to_owned(),
                    lastname: "Doe".to_owned(),
                    email: "john@example.com".to_owned(),
                    phone_number: "+358401234567".to_owned(),
                    marketing: false,
                    terms: true,
                },
            )
            .await
            .expect("create should succeed");

        let updated = service
            .update(
                "user-123",
                UpdateProfileParams {
                    firstname: Some("Jane".to_owned()),
                    marketing: Some(true),
                    ..UpdateProfileParams::default()
                },
            )
            .await
            .expect("update should succeed");

        assert_eq!(updated.firstname, "Jane");
        assert!(updated.marketing);
        assert_eq!(updated.lastname, "Doe");
    }

    #[test]
    fn firestore_update_mask_contains_only_changed_fields_and_audit_timestamp() {
        let mask = update_field_mask(&UpdateProfileParams {
            firstname: Some("Jane".to_owned()),
            email: Some("jane@example.com".to_owned()),
            marketing: Some(true),
            ..UpdateProfileParams::default()
        });

        assert_eq!(mask, ["firstname", "email", "marketing", "updatedAt"]);
    }

    #[test]
    fn generated_profile_timestamps_are_rfc3339() {
        let timestamp = timestamp_now();
        assert!(OffsetDateTime::parse(&timestamp, &Rfc3339).is_ok());
    }

    #[test]
    fn profile_log_categories_are_stable_and_non_sensitive() {
        assert_eq!(
            profile_error_kind(&ProfileServiceError::NotFound),
            "not_found"
        );
        assert_eq!(
            profile_error_kind(&ProfileServiceError::AlreadyExists),
            "already_exists"
        );
        assert_eq!(
            profile_error_kind(&ProfileServiceError::Backend("secret detail".to_owned())),
            "backend"
        );
    }

    #[tokio::test]
    async fn mock_service_can_seed_an_existing_profile() {
        let profile = Profile {
            id: "user-123".to_owned(),
            firstname: "Jane".to_owned(),
            lastname: "Doe".to_owned(),
            email: "jane@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing: false,
            terms: true,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let service = ProfileService::mock(MockProfileService::default().with_profile(profile));

        let loaded = service
            .get("user-123")
            .await
            .expect("seeded profile should load");
        assert_eq!(loaded.firstname, "Jane");
        assert_eq!(loaded.email, "jane@example.com");
    }
}
