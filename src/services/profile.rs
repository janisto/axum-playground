use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use firestore::{
    FirestoreDb, FirestoreDbOptions, FirestoreWritePrecondition, errors::FirestoreError,
};
use gcloud_sdk::{ExternalJwtFunctionSource, Token, TokenSourceType};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, macros::datetime};
use tokio::sync::OnceCell;
use tracing::info;
use utoipa::ToSchema;

use crate::{
    config::AppConfig,
    error::StartupError,
    validation::{
        canonical_clock_timestamp, next_timestamp, normalize_contact_email, normalize_phone_number,
        normalize_timestamp, valid_bounded_name, valid_opaque_id,
    },
};

pub(crate) const PROFILES_COLLECTION: &str = "profiles";
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

#[derive(Clone, Debug)]
pub struct MockProfileService {
    state: Arc<Mutex<MockProfileState>>,
    committed_writes: Arc<AtomicUsize>,
    operations: Arc<Mutex<Vec<ProfileOperation>>>,
    now: Arc<Mutex<OffsetDateTime>>,
    error: Option<ProfileServiceError>,
}

#[derive(Clone, Debug, Default)]
struct MockProfileState {
    profiles: BTreeMap<String, Profile>,
}

impl Default for MockProfileService {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockProfileState::default())),
            committed_writes: Arc::new(AtomicUsize::new(0)),
            operations: Arc::new(Mutex::new(Vec::new())),
            now: Arc::new(Mutex::new(datetime!(2026-07-30 12:00 UTC))),
            error: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub contact_email: String,
    pub phone_number: String,
    pub marketing_opt_in: bool,
    pub terms_accepted: bool,
    #[schema(format = DateTime)]
    pub created_at: String,
    #[schema(format = DateTime)]
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProfile {
    id: String,
    first_name: String,
    last_name: String,
    contact_email: String,
    phone_number: String,
    marketing_opt_in: bool,
    terms_accepted: bool,
    created_at: String,
    updated_at: String,
}

impl From<StoredProfile> for Profile {
    fn from(value: StoredProfile) -> Self {
        Self {
            id: value.id,
            first_name: value.first_name,
            last_name: value.last_name,
            contact_email: value.contact_email,
            phone_number: value.phone_number,
            marketing_opt_in: value.marketing_opt_in,
            terms_accepted: value.terms_accepted,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateProfileParams {
    pub first_name: String,
    pub last_name: String,
    pub contact_email: String,
    pub phone_number: String,
    pub marketing_opt_in: bool,
    pub terms_accepted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateProfileParams {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub contact_email: Option<String>,
    pub phone_number: Option<String>,
    pub marketing_opt_in: Option<bool>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ProfileServiceError {
    #[error("profile not found")]
    NotFound,
    #[error("profile already exists")]
    AlreadyExists,
    #[error("profile service unavailable")]
    Unavailable(ProfileBackendError),
    #[error(transparent)]
    Backend(#[from] ProfileBackendError),
}

#[derive(Clone)]
pub struct ProfileBackendError {
    operation: ProfileOperation,
    source: Arc<dyn Error + Send + Sync>,
}

impl ProfileBackendError {
    pub fn new(operation: ProfileOperation, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            operation,
            source: Arc::new(source),
        }
    }

    #[must_use]
    pub const fn operation(&self) -> ProfileOperation {
        self.operation
    }
}

impl fmt::Debug for ProfileBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileBackendError")
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ProfileBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "profile {} backend error", self.operation)
    }
}

impl Error for ProfileBackendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileOperation {
    Initialize,
    Create,
    Get,
    Update,
    Delete,
}

impl fmt::Display for ProfileOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Initialize => "initialize",
            Self::Create => "create",
            Self::Get => "get",
            Self::Update => "update",
            Self::Delete => "delete",
        })
    }
}

impl ProfileService {
    pub fn firestore(config: &AppConfig) -> Result<Self, StartupError> {
        if let Some(host) = config.firestore_emulator_host.as_deref()
            && !config.emulator_host_is_loopback(host)
        {
            return Err(StartupError::UnsafeEmulatorHost {
                variable: "FIRESTORE_EMULATOR_HOST",
                host: host.to_owned(),
            });
        }
        Ok(Self {
            inner: Arc::new(ProfileServiceInner::Firestore(Box::new(
                FirestoreProfileStore {
                    project_id: config.resolved_google_project_id().to_owned(),
                    emulator_host: config.firestore_emulator_host.clone(),
                    db: Arc::new(OnceCell::new()),
                },
            ))),
        })
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
            ProfileServiceInner::Mock(store) => store.create(user_id, params),
        }
    }

    pub async fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.get(user_id).await,
            ProfileServiceInner::Mock(store) => store.get(user_id),
        }
    }

    pub async fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.update(user_id, params).await,
            ProfileServiceInner::Mock(store) => store.update(user_id, params),
        }
    }

    pub async fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        match self.inner.as_ref() {
            ProfileServiceInner::Firestore(store) => store.delete(user_id).await,
            ProfileServiceInner::Mock(store) => store.delete(user_id),
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
        self.state
            .lock()
            .expect("mock profile state lock should succeed")
            .profiles
            .insert(profile.id.clone(), profile);
        self
    }

    #[must_use]
    pub fn with_now(self, now: OffsetDateTime) -> Self {
        *self.now.lock().expect("mock clock lock should succeed") = now;
        self
    }

    pub fn set_now(&self, now: OffsetDateTime) {
        *self.now.lock().expect("mock clock lock should succeed") = now;
    }

    #[must_use]
    pub fn committed_write_count(&self) -> usize {
        self.committed_writes.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn operation_count(&self, operation: ProfileOperation) -> usize {
        self.operations
            .lock()
            .expect("mock profile operation lock should succeed")
            .iter()
            .filter(|value| **value == operation)
            .count()
    }

    #[must_use]
    pub fn stored_profile(&self, user_id: &str) -> Option<Profile> {
        self.state
            .lock()
            .expect("mock profile state lock should succeed")
            .profiles
            .get(user_id)
            .cloned()
    }

    fn create(
        &self,
        user_id: &str,
        params: CreateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        self.record(ProfileOperation::Create);
        self.fail_if_configured()?;
        let now = *self.now.lock().expect("mock clock lock should succeed");
        let mut state = self
            .state
            .lock()
            .expect("mock profile state lock should succeed");
        if state.profiles.contains_key(user_id) {
            return Err(ProfileServiceError::AlreadyExists);
        }
        let profile = build_profile(user_id, params, now)?;
        state.profiles.insert(user_id.to_owned(), profile.clone());
        self.committed_writes.fetch_add(1, Ordering::SeqCst);
        Ok(profile)
    }

    fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        self.record(ProfileOperation::Get);
        self.fail_if_configured()?;
        let profile = self
            .state
            .lock()
            .expect("mock profile state lock should succeed")
            .profiles
            .get(user_id)
            .cloned()
            .ok_or(ProfileServiceError::NotFound)?;
        validate_stored_profile(&profile, user_id, ProfileOperation::Get)?;
        Ok(profile)
    }

    fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        self.record(ProfileOperation::Update);
        self.fail_if_configured()?;
        let now = *self.now.lock().expect("mock clock lock should succeed");
        let mut state = self
            .state
            .lock()
            .expect("mock profile state lock should succeed");
        let current = state
            .profiles
            .get(user_id)
            .cloned()
            .ok_or(ProfileServiceError::NotFound)?;
        validate_stored_profile(&current, user_id, ProfileOperation::Update)?;
        let Some(updated) = updated_profile(&current, params, now)? else {
            return Ok(current);
        };
        state.profiles.insert(user_id.to_owned(), updated.clone());
        self.committed_writes.fetch_add(1, Ordering::SeqCst);
        Ok(updated)
    }

    fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        self.record(ProfileOperation::Delete);
        self.fail_if_configured()?;
        let mut state = self
            .state
            .lock()
            .expect("mock profile state lock should succeed");
        if state.profiles.remove(user_id).is_none() {
            return Err(ProfileServiceError::NotFound);
        }
        self.committed_writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn fail_if_configured(&self) -> Result<(), ProfileServiceError> {
        self.error.clone().map_or(Ok(()), Err)
    }

    fn record(&self, operation: ProfileOperation) {
        self.operations
            .lock()
            .expect("mock profile operation lock should succeed")
            .push(operation);
    }
}

#[derive(Clone, Debug)]
enum TransactionOutcome {
    Profile(Profile),
    Exists,
    NotFound,
    NoChange(Profile),
    TimestampExhausted,
    Deleted,
}

fn create_transaction_result(outcome: TransactionOutcome) -> Result<Profile, ProfileServiceError> {
    match outcome {
        TransactionOutcome::Profile(profile) => Ok(profile),
        TransactionOutcome::Exists => Err(ProfileServiceError::AlreadyExists),
        _ => Err(internal_profile_error(ProfileOperation::Create)),
    }
}

fn update_transaction_result(outcome: TransactionOutcome) -> Result<Profile, ProfileServiceError> {
    match outcome {
        TransactionOutcome::Profile(profile) | TransactionOutcome::NoChange(profile) => Ok(profile),
        TransactionOutcome::NotFound => Err(ProfileServiceError::NotFound),
        _ => Err(internal_profile_error(ProfileOperation::Update)),
    }
}

fn delete_transaction_result(outcome: &TransactionOutcome) -> Result<(), ProfileServiceError> {
    match outcome {
        TransactionOutcome::Deleted => Ok(()),
        TransactionOutcome::NotFound => Err(ProfileServiceError::NotFound),
        _ => Err(internal_profile_error(ProfileOperation::Delete)),
    }
}

impl FirestoreProfileStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let user_id = user_id.to_owned();
        let now = OffsetDateTime::now_utc();
        let profile = build_profile(&user_id, params, now)?;
        let result = db
            .run_transaction(|db, transaction| {
                let document_id = document_id.clone();
                let profile = profile.clone();
                Box::pin(async move {
                    let existing: Option<StoredProfile> = db
                        .fluent()
                        .select()
                        .by_id_in(PROFILES_COLLECTION)
                        .obj()
                        .one(&document_id)
                        .await?;
                    if existing.is_some() {
                        return Ok(TransactionOutcome::Exists);
                    }
                    db.fluent()
                        .update()
                        .in_col(PROFILES_COLLECTION)
                        .precondition(FirestoreWritePrecondition::Exists(false))
                        .document_id(&document_id)
                        .object(&profile)
                        .add_to_transaction(transaction)?;
                    Ok(TransactionOutcome::Profile(profile))
                })
            })
            .await
            .map_err(|error| map_firestore_error(error, ProfileOperation::Create))?;
        let profile = create_transaction_result(result)?;
        info!(operation = "profile.create", "profile mutation succeeded");
        Ok(profile)
    }

    async fn get(&self, user_id: &str) -> Result<Profile, ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let profile: Option<StoredProfile> = db
            .fluent()
            .select()
            .by_id_in(PROFILES_COLLECTION)
            .obj()
            .one(&document_id)
            .await
            .map_err(|error| map_firestore_error(error, ProfileOperation::Get))?;
        let profile: Profile = profile.ok_or(ProfileServiceError::NotFound)?.into();
        validate_stored_profile(&profile, user_id, ProfileOperation::Get)?;
        Ok(profile)
    }

    async fn update(
        &self,
        user_id: &str,
        params: UpdateProfileParams,
    ) -> Result<Profile, ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let user_id = user_id.to_owned();
        let now = OffsetDateTime::now_utc();
        let result = db
            .run_transaction(|db, transaction| {
                let document_id = document_id.clone();
                let user_id = user_id.clone();
                let params = params.clone();
                Box::pin(async move {
                    let current: Option<StoredProfile> = db
                        .fluent()
                        .select()
                        .by_id_in(PROFILES_COLLECTION)
                        .obj()
                        .one(&document_id)
                        .await?;
                    let Some(current) = current.map(Profile::from) else {
                        return Ok(TransactionOutcome::NotFound);
                    };
                    if validate_stored_profile(&current, &user_id, ProfileOperation::Update)
                        .is_err()
                    {
                        return Ok(TransactionOutcome::TimestampExhausted);
                    }
                    let updated = match updated_profile(&current, params, now) {
                        Ok(Some(updated)) => updated,
                        Ok(None) => return Ok(TransactionOutcome::NoChange(current)),
                        Err(_) => return Ok(TransactionOutcome::TimestampExhausted),
                    };
                    db.fluent()
                        .update()
                        .in_col(PROFILES_COLLECTION)
                        .precondition(FirestoreWritePrecondition::Exists(true))
                        .document_id(&document_id)
                        .object(&updated)
                        .add_to_transaction(transaction)?;
                    Ok(TransactionOutcome::Profile(updated))
                })
            })
            .await
            .map_err(|error| map_firestore_error(error, ProfileOperation::Update))?;
        update_transaction_result(result)
    }

    async fn delete(&self, user_id: &str) -> Result<(), ProfileServiceError> {
        let db = self.db().await?;
        let document_id = profile_document_id(user_id);
        let result = db
            .run_transaction(|db, transaction| {
                let document_id = document_id.clone();
                Box::pin(async move {
                    let current: Option<StoredProfile> = db
                        .fluent()
                        .select()
                        .by_id_in(PROFILES_COLLECTION)
                        .obj()
                        .one(&document_id)
                        .await?;
                    if current.is_none() {
                        return Ok(TransactionOutcome::NotFound);
                    }
                    db.fluent()
                        .delete()
                        .from(PROFILES_COLLECTION)
                        .document_id(&document_id)
                        .precondition(FirestoreWritePrecondition::Exists(true))
                        .add_to_transaction(transaction)?;
                    Ok(TransactionOutcome::Deleted)
                })
            })
            .await
            .map_err(|error| map_firestore_error(error, ProfileOperation::Delete))?;
        delete_transaction_result(&result)
    }

    async fn db(&self) -> Result<&FirestoreDb, ProfileServiceError> {
        self.db
            .get_or_try_init(|| async {
                new_firestore_db(&self.project_id, self.emulator_host.as_deref()).await
            })
            .await
    }
}

pub(crate) async fn new_firestore_db(
    project_id: &str,
    emulator_host: Option<&str>,
) -> Result<FirestoreDb, ProfileServiceError> {
    let options = firestore_db_options(project_id, emulator_host);
    let db = if emulator_host.is_some() {
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
    db.map_err(|error| map_firestore_error(error, ProfileOperation::Initialize))
}

fn firestore_db_options(project_id: &str, emulator_host: Option<&str>) -> FirestoreDbOptions {
    let api_url = emulator_host
        .map(|host| format!("http://{host}"))
        .unwrap_or_else(|| FIRESTORE_API_URL.to_owned());
    FirestoreDbOptions::new(project_id.to_owned()).with_firebase_api_url(api_url)
}

pub(crate) fn profile_document_id(user_id: &str) -> String {
    format!("uid_{}", URL_SAFE_NO_PAD.encode(user_id))
}

fn build_profile(
    user_id: &str,
    params: CreateProfileParams,
    now: OffsetDateTime,
) -> Result<Profile, ProfileServiceError> {
    let timestamp = canonical_clock_timestamp(now)
        .ok_or_else(|| internal_profile_error(ProfileOperation::Create))?;
    let profile = Profile {
        id: user_id.to_owned(),
        first_name: params.first_name,
        last_name: params.last_name,
        contact_email: params.contact_email,
        phone_number: params.phone_number,
        marketing_opt_in: params.marketing_opt_in,
        terms_accepted: params.terms_accepted,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    validate_stored_profile(&profile, user_id, ProfileOperation::Create)?;
    Ok(profile)
}

fn updated_profile(
    current: &Profile,
    params: UpdateProfileParams,
    now: OffsetDateTime,
) -> Result<Option<Profile>, ProfileServiceError> {
    let mut updated = current.clone();
    if let Some(value) = params.first_name {
        updated.first_name = value;
    }
    if let Some(value) = params.last_name {
        updated.last_name = value;
    }
    if let Some(value) = params.contact_email {
        updated.contact_email = value;
    }
    if let Some(value) = params.phone_number {
        updated.phone_number = value;
    }
    if let Some(value) = params.marketing_opt_in {
        updated.marketing_opt_in = value;
    }
    if updated == *current {
        return Ok(None);
    }
    updated.updated_at = next_timestamp(&current.updated_at, now)
        .ok_or_else(|| internal_profile_error(ProfileOperation::Update))?;
    validate_stored_profile(&updated, &current.id, ProfileOperation::Update)?;
    Ok(Some(updated))
}

pub(crate) fn validate_stored_profile(
    profile: &Profile,
    expected_id: &str,
    operation: ProfileOperation,
) -> Result<(), ProfileServiceError> {
    let valid = profile.id == expected_id
        && valid_opaque_id(&profile.id)
        && valid_bounded_name(&profile.first_name)
        && valid_bounded_name(&profile.last_name)
        && normalize_contact_email(&profile.contact_email).as_deref()
            == Some(profile.contact_email.as_str())
        && normalize_phone_number(&profile.phone_number).as_deref()
            == Some(profile.phone_number.as_str())
        && profile.terms_accepted
        && normalize_timestamp(&profile.created_at).as_deref() == Some(profile.created_at.as_str())
        && normalize_timestamp(&profile.updated_at).as_deref() == Some(profile.updated_at.as_str())
        && profile.created_at <= profile.updated_at;
    if valid {
        Ok(())
    } else {
        Err(internal_profile_error(operation))
    }
}

fn internal_profile_error(operation: ProfileOperation) -> ProfileServiceError {
    ProfileBackendError::new(operation, std::io::Error::other("invalid profile state")).into()
}

fn map_firestore_error(error: FirestoreError, operation: ProfileOperation) -> ProfileServiceError {
    let unavailable = matches!(
        &error,
        FirestoreError::NetworkError(_)
            | FirestoreError::SystemError(_)
            | FirestoreError::ErrorInTransaction(_)
    ) || matches!(&error, FirestoreError::DatabaseError(value) if value.retry_possible);
    if unavailable {
        return ProfileServiceError::Unavailable(ProfileBackendError::new(operation, error));
    }
    match (operation, error) {
        (ProfileOperation::Create, FirestoreError::DataConflictError(_)) => {
            ProfileServiceError::AlreadyExists
        }
        (
            ProfileOperation::Update | ProfileOperation::Delete,
            FirestoreError::DataConflictError(_),
        )
        | (_, FirestoreError::DataNotFoundError(_)) => ProfileServiceError::NotFound,
        (operation, other) => ProfileBackendError::new(operation, other).into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error as _, sync::Arc};

    use firestore::{FirestoreDb, errors::FirestoreError};
    use gcloud_sdk::tonic::Status;
    use time::macros::datetime;

    use super::{
        CreateProfileParams, MockProfileService, Profile, ProfileBackendError, ProfileOperation,
        ProfileServiceError, StoredProfile, TransactionOutcome, UpdateProfileParams,
        create_transaction_result, delete_transaction_result, map_firestore_error,
        profile_document_id, update_transaction_result, validate_stored_profile,
    };

    fn create_params(first_name: &str) -> CreateProfileParams {
        CreateProfileParams {
            first_name: first_name.to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
        }
    }

    #[test]
    fn firestore_document_id_safely_encodes_opaque_principals() {
        assert_eq!(profile_document_id("tenant/user"), "uid_dGVuYW50L3VzZXI");
        assert_eq!(profile_document_id("."), "uid_Lg");
        assert_eq!(profile_document_id(".."), "uid_Li4");
    }

    #[test]
    fn profile_round_trips_through_the_native_firestore_codec() {
        let profile = Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-07-30T12:00:00.000Z".to_owned(),
            updated_at: "2026-07-30T12:00:00.000Z".to_owned(),
        };
        let document = FirestoreDb::serialize_to_doc(
            "projects/test/databases/(default)/documents/profiles/uid_dXNlci0xMjM",
            &profile,
        )
        .expect("serialize profile");
        let decoded: StoredProfile =
            FirestoreDb::deserialize_doc_to(&document).expect("deserialize profile");
        let decoded = Profile::from(decoded);
        assert_eq!(decoded, profile);
    }

    #[test]
    fn backend_errors_are_safe_and_preserve_the_internal_source() {
        let error = ProfileBackendError::new(
            ProfileOperation::Get,
            std::io::Error::other("private sentinel"),
        );
        assert_eq!(error.operation(), ProfileOperation::Get);
        assert_eq!(error.to_string(), "profile get backend error");
        assert_eq!(
            format!("{error:?}"),
            "ProfileBackendError { operation: Get, .. }"
        );
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("private sentinel")
        );

        for (operation, expected) in [
            (ProfileOperation::Initialize, "initialize"),
            (ProfileOperation::Create, "create"),
            (ProfileOperation::Get, "get"),
            (ProfileOperation::Update, "update"),
            (ProfileOperation::Delete, "delete"),
        ] {
            assert_eq!(operation.to_string(), expected);
        }
    }

    #[test]
    fn stored_profile_validation_checks_every_persisted_invariant() {
        let valid = Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-07-30T12:00:00.000Z".to_owned(),
            updated_at: "2026-07-30T12:00:00.001Z".to_owned(),
        };
        validate_stored_profile(&valid, "user-123", ProfileOperation::Get).expect("valid");

        let invalid = [
            Profile {
                id: "other".to_owned(),
                ..valid.clone()
            },
            Profile {
                id: String::new(),
                ..valid.clone()
            },
            Profile {
                first_name: String::new(),
                ..valid.clone()
            },
            Profile {
                last_name: " ".to_owned(),
                ..valid.clone()
            },
            Profile {
                contact_email: "Ada@EXAMPLE.COM".to_owned(),
                ..valid.clone()
            },
            Profile {
                phone_number: "+358 40 1234567".to_owned(),
                ..valid.clone()
            },
            Profile {
                terms_accepted: false,
                ..valid.clone()
            },
            Profile {
                created_at: "2026-07-30T12:00:00Z".to_owned(),
                ..valid.clone()
            },
            Profile {
                updated_at: "not-a-time".to_owned(),
                ..valid.clone()
            },
            Profile {
                created_at: "2026-07-30T12:00:00.002Z".to_owned(),
                updated_at: "2026-07-30T12:00:00.001Z".to_owned(),
                ..valid
            },
        ];
        for profile in invalid {
            assert!(matches!(
                validate_stored_profile(&profile, "user-123", ProfileOperation::Update),
                Err(ProfileServiceError::Backend(_))
            ));
        }
    }

    #[test]
    fn firestore_error_mapping_preserves_retry_and_operation_semantics() {
        assert!(matches!(
            map_firestore_error(
                FirestoreError::from(Status::unavailable("transient")),
                ProfileOperation::Get,
            ),
            ProfileServiceError::Unavailable(_)
        ));
        assert!(matches!(
            map_firestore_error(
                FirestoreError::from(Status::invalid_argument("permanent")),
                ProfileOperation::Get,
            ),
            ProfileServiceError::Backend(_)
        ));
        assert!(matches!(
            map_firestore_error(
                FirestoreError::from(Status::already_exists("exists")),
                ProfileOperation::Create,
            ),
            ProfileServiceError::AlreadyExists
        ));
        assert!(matches!(
            map_firestore_error(
                FirestoreError::from(Status::already_exists("conflict")),
                ProfileOperation::Update,
            ),
            ProfileServiceError::NotFound
        ));
        assert!(matches!(
            map_firestore_error(
                FirestoreError::from(Status::not_found("missing")),
                ProfileOperation::Create,
            ),
            ProfileServiceError::NotFound
        ));
    }

    #[test]
    fn native_transaction_outcomes_map_to_exact_public_results() {
        let profile = Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-07-30T12:00:00.000Z".to_owned(),
            updated_at: "2026-07-30T12:00:00.000Z".to_owned(),
        };

        assert_eq!(
            create_transaction_result(TransactionOutcome::Profile(profile.clone()))
                .expect("created"),
            profile
        );
        assert!(matches!(
            create_transaction_result(TransactionOutcome::Exists),
            Err(ProfileServiceError::AlreadyExists)
        ));
        for outcome in [
            TransactionOutcome::NotFound,
            TransactionOutcome::NoChange(profile.clone()),
            TransactionOutcome::TimestampExhausted,
            TransactionOutcome::Deleted,
        ] {
            assert!(matches!(
                create_transaction_result(outcome),
                Err(ProfileServiceError::Backend(_))
            ));
        }

        for outcome in [
            TransactionOutcome::Profile(profile.clone()),
            TransactionOutcome::NoChange(profile.clone()),
        ] {
            assert_eq!(
                update_transaction_result(outcome).expect("updated"),
                profile
            );
        }
        assert!(matches!(
            update_transaction_result(TransactionOutcome::NotFound),
            Err(ProfileServiceError::NotFound)
        ));
        for outcome in [
            TransactionOutcome::Exists,
            TransactionOutcome::TimestampExhausted,
            TransactionOutcome::Deleted,
        ] {
            assert!(matches!(
                update_transaction_result(outcome),
                Err(ProfileServiceError::Backend(_))
            ));
        }

        assert!(delete_transaction_result(&TransactionOutcome::Deleted).is_ok());
        assert!(matches!(
            delete_transaction_result(&TransactionOutcome::NotFound),
            Err(ProfileServiceError::NotFound)
        ));
        for outcome in [
            TransactionOutcome::Profile(profile.clone()),
            TransactionOutcome::Exists,
            TransactionOutcome::NoChange(profile),
            TransactionOutcome::TimestampExhausted,
        ] {
            assert!(matches!(
                delete_transaction_result(&outcome),
                Err(ProfileServiceError::Backend(_))
            ));
        }
    }

    #[tokio::test]
    async fn mock_create_is_atomic_under_concurrency() {
        let mock = MockProfileService::default();
        let left = mock.clone();
        let right = mock.clone();
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let spawn =
            |store: MockProfileService, name: &'static str, barrier: Arc<tokio::sync::Barrier>| {
                tokio::spawn(async move {
                    barrier.wait().await;
                    store.create("user-123", create_params(name))
                })
            };
        let first = spawn(left, "Ada", Arc::clone(&barrier));
        let second = spawn(right, "Grace", Arc::clone(&barrier));
        barrier.wait().await;
        let outcomes = [first.await.expect("task"), second.await.expect("task")];
        assert_eq!(outcomes.iter().filter(|value| value.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|value| matches!(value, Err(ProfileServiceError::AlreadyExists)))
                .count(),
            1
        );
        assert_eq!(mock.committed_write_count(), 1);
    }

    #[test]
    fn no_op_update_preserves_timestamp_and_commits_no_write() {
        let mock = MockProfileService::default();
        let created = mock
            .create("user-123", create_params("Ada"))
            .expect("create");
        let updated = mock
            .update(
                "user-123",
                UpdateProfileParams {
                    first_name: Some("Ada".to_owned()),
                    ..UpdateProfileParams::default()
                },
            )
            .expect("update");
        assert_eq!(updated, created);
        assert_eq!(mock.committed_write_count(), 1);
    }

    #[test]
    fn real_update_is_monotonic_and_max_timestamp_fails_without_write() {
        let mock = MockProfileService::default().with_now(datetime!(2025-01-01 0:00 UTC));
        let profile = Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "Ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-01-01T00:00:00.000Z".to_owned(),
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let mock = mock.with_profile(profile);
        let updated = mock
            .update(
                "user-123",
                UpdateProfileParams {
                    marketing_opt_in: Some(true),
                    ..UpdateProfileParams::default()
                },
            )
            .expect("update");
        assert_eq!(updated.updated_at, "2026-01-01T00:00:00.001Z");

        let max = Profile {
            updated_at: crate::validation::MAX_TIMESTAMP.to_owned(),
            ..updated
        };
        let max_store = MockProfileService::default().with_profile(max.clone());
        let writes = max_store.committed_write_count();
        assert!(matches!(
            max_store.update(
                "user-123",
                UpdateProfileParams {
                    marketing_opt_in: Some(false),
                    ..UpdateProfileParams::default()
                }
            ),
            Err(ProfileServiceError::Backend(_))
        ));
        assert_eq!(max_store.committed_write_count(), writes);
        assert_eq!(max_store.stored_profile("user-123"), Some(max));
    }
}
