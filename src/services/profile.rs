use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use firestore::{FirestoreDb, FirestoreWritePrecondition, errors::FirestoreError};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::OnceCell;
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::config::AppConfig;

const PROFILES_COLLECTION: &str = "profiles";

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
    pub fn firestore(config: &AppConfig) -> Self {
        let project_id = config
            .resolved_google_project_id()
            .unwrap_or(config.firebase_project_id.as_str())
            .to_string();

        Self {
            inner: Arc::new(ProfileServiceInner::Firestore(Box::new(
                FirestoreProfileStore {
                    project_id,
                    db: Arc::new(OnceCell::new()),
                },
            ))),
        }
    }

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
    pub fn with_error(mut self, error: ProfileServiceError) -> Self {
        self.error = Some(error);
        self
    }

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
        profiles.insert(user_id.to_string(), profile.clone());
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
                FirestoreDb::new(&self.project_id)
                    .await
                    .map_err(|error| ProfileServiceError::Backend(error.to_string()))
            })
            .await
    }
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
        id: user_id.to_string(),
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
        fields.push("firstname".to_string());
    }
    if params.lastname.is_some() {
        fields.push("lastname".to_string());
    }
    if params.email.is_some() {
        fields.push("email".to_string());
    }
    if params.phone_number.is_some() {
        fields.push("phoneNumber".to_string());
    }
    if params.marketing.is_some() {
        fields.push("marketing".to_string());
    }
    fields.push("updatedAt".to_string());
    fields
}

fn normalize_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_phone(value: &str) -> String {
    value.trim().to_string()
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
    use super::{
        CreateProfileParams, MockProfileService, ProfileService, ProfileServiceError,
        UpdateProfileParams, profile_document_id,
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

    #[tokio::test]
    async fn mock_service_normalizes_email_and_phone() {
        let service = ProfileService::mock(MockProfileService::default());
        let profile = service
            .create(
                "user-123",
                CreateProfileParams {
                    firstname: "John".to_string(),
                    lastname: "Doe".to_string(),
                    email: "  JOHN@EXAMPLE.COM  ".to_string(),
                    phone_number: "  +358401234567  ".to_string(),
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
                    firstname: "John".to_string(),
                    lastname: "Doe".to_string(),
                    email: "john@example.com".to_string(),
                    phone_number: "+358401234567".to_string(),
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
                    firstname: "Jane".to_string(),
                    lastname: "Doe".to_string(),
                    email: "jane@example.com".to_string(),
                    phone_number: "+358401234567".to_string(),
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
                    firstname: "John".to_string(),
                    lastname: "Doe".to_string(),
                    email: "john@example.com".to_string(),
                    phone_number: "+358401234567".to_string(),
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
                    firstname: Some("Jane".to_string()),
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
}
