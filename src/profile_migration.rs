//! One-time migration from the retired profile persistence shape.

use std::{collections::BTreeSet, error::Error, fmt, sync::Arc};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use firestore::{
    FirestoreDb, FirestoreDocument, FirestoreWritePrecondition,
    errors::{BackoffError, FirestoreError},
    timestamp_utils::from_timestamp,
};
use futures_util::StreamExt;
use gcloud_sdk::{google::firestore::v1::value::ValueType, prost_types::Timestamp};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    AppConfig,
    services::profile::{
        CANONICAL_PROFILE_FIELDS, PROFILES_COLLECTION, Profile, new_firestore_db,
        profile_document_id, transaction_firestore_error, validate_stored_profile,
    },
    validation::{canonical_clock_timestamp, normalize_contact_email, normalize_phone_number},
};

const LEGACY_FIELDS: [&str; 9] = [
    "createdAt",
    "email",
    "firstname",
    "id",
    "lastname",
    "marketing",
    "phoneNumber",
    "terms",
    "updatedAt",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileMigrationMode {
    Audit,
    Apply { confirmed_project: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMigrationStatus {
    Current,
    MigrationRequired,
    Blocked,
}

impl fmt::Display for ProfileMigrationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Current => "current",
            Self::MigrationRequired => "migration_required",
            Self::Blocked => "blocked",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMigrationReason {
    CanonicalProfile,
    LegacyProfile,
    UnexpectedShape,
    InvalidFieldType,
    InvalidValue,
    DocumentIdMismatch,
}

impl fmt::Display for ProfileMigrationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalProfile => "canonical_profile",
            Self::LegacyProfile => "legacy_profile",
            Self::UnexpectedShape => "unexpected_shape",
            Self::InvalidFieldType => "invalid_field_type",
            Self::InvalidValue => "invalid_value",
            Self::DocumentIdMismatch => "document_id_mismatch",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileMigrationRecord {
    pub fingerprint: String,
    pub status: ProfileMigrationStatus,
    pub reason: ProfileMigrationReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileMigrationReport {
    pub project_id: String,
    pub records: Vec<ProfileMigrationRecord>,
    pub migrated: usize,
}

impl ProfileMigrationReport {
    #[must_use]
    pub fn current(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.status == ProfileMigrationStatus::Current)
            .count()
    }

    #[must_use]
    pub fn migration_required(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.status == ProfileMigrationStatus::MigrationRequired)
            .count()
    }

    #[must_use]
    pub fn blocked(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.status == ProfileMigrationStatus::Blocked)
            .count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMigrationErrorKind {
    ProjectConfirmation,
    Initialize,
    Read,
    BlockedPreflight,
    ConcurrentChange,
    Write,
    Verification,
}

pub struct ProfileMigrationError {
    kind: ProfileMigrationErrorKind,
    report: Option<ProfileMigrationReport>,
    source: Option<Arc<dyn Error + Send + Sync>>,
}

impl ProfileMigrationError {
    #[must_use]
    pub const fn kind(&self) -> ProfileMigrationErrorKind {
        self.kind
    }

    #[must_use]
    pub fn report(&self) -> Option<&ProfileMigrationReport> {
        self.report.as_ref()
    }

    fn with_report(mut self, report: Option<ProfileMigrationReport>) -> Self {
        self.report = report;
        self
    }

    fn reclassify(
        mut self,
        kind: ProfileMigrationErrorKind,
        report: Option<ProfileMigrationReport>,
    ) -> Self {
        self.kind = kind;
        self.report = report;
        self
    }
}

impl fmt::Debug for ProfileMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileMigrationError")
            .field("kind", &self.kind)
            .field("has_report", &self.report.is_some())
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl fmt::Display for ProfileMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ProfileMigrationErrorKind::ProjectConfirmation => {
                "the explicit project confirmation does not match the configured project"
            }
            ProfileMigrationErrorKind::Initialize => {
                "the profile migration could not initialize Firestore"
            }
            ProfileMigrationErrorKind::Read => "the profile migration audit could not complete",
            ProfileMigrationErrorKind::BlockedPreflight => {
                "the profile migration is blocked by noncanonical persisted data"
            }
            ProfileMigrationErrorKind::ConcurrentChange => {
                "a profile changed after audit; rerun the audit before applying"
            }
            ProfileMigrationErrorKind::Write => "a profile migration transaction did not complete",
            ProfileMigrationErrorKind::Verification => {
                "post-migration verification did not find only canonical profiles"
            }
        })
    }
}

impl Error for ProfileMigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

#[derive(Clone, Debug)]
struct ClassifiedRecord {
    document_id: String,
    update_time: Option<Timestamp>,
    classification: Classification,
}

#[derive(Clone, Debug)]
enum Classification {
    Current,
    Legacy(Profile),
    Blocked(ProfileMigrationReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransactionOutcome {
    Migrated,
    AlreadyCurrent,
    ConcurrentChange,
}

#[derive(Clone, Debug, PartialEq)]
struct MigrationTarget {
    document_id: String,
    update_time: Timestamp,
}

#[derive(Debug)]
enum MigrationDecision {
    Migrate(Profile),
    AlreadyCurrent,
    ConcurrentChange,
}

/// Audits by default and applies only after an exact project-ID confirmation.
///
/// Apply performs a complete read-only preflight before its first write, then
/// re-reads and replaces each legacy record transactionally. It is idempotent;
/// a post-apply audit must find only canonical records.
pub async fn run_profile_migration(
    config: &AppConfig,
    mode: ProfileMigrationMode,
) -> Result<ProfileMigrationReport, ProfileMigrationError> {
    if let Some(host) = config.firestore_emulator_host.as_deref()
        && !config.emulator_host_is_loopback(host)
    {
        return Err(migration_error(ProfileMigrationErrorKind::Initialize, None));
    }
    let project_id = config.resolved_google_project_id();
    if let ProfileMigrationMode::Apply { confirmed_project } = &mode
        && confirmed_project != project_id
    {
        return Err(migration_error(
            ProfileMigrationErrorKind::ProjectConfirmation,
            None,
        ));
    }

    let db = new_firestore_db(project_id, config.firestore_emulator_host.as_deref())
        .await
        .map_err(|error| {
            migration_error_with_source(ProfileMigrationErrorKind::Initialize, None, error)
        })?;
    run_profile_migration_with_store(project_id, &mode, &FirestoreMigrationStore { db: &db }).await
}

trait MigrationStore {
    async fn load(&self) -> Result<Vec<ClassifiedRecord>, ProfileMigrationError>;
    async fn migrate(
        &self,
        target: &MigrationTarget,
    ) -> Result<TransactionOutcome, ProfileMigrationError>;
}

struct FirestoreMigrationStore<'a> {
    db: &'a FirestoreDb,
}

impl MigrationStore for FirestoreMigrationStore<'_> {
    async fn load(&self) -> Result<Vec<ClassifiedRecord>, ProfileMigrationError> {
        load_classified(self.db).await
    }

    async fn migrate(
        &self,
        target: &MigrationTarget,
    ) -> Result<TransactionOutcome, ProfileMigrationError> {
        migrate_one(self.db, target).await
    }
}

async fn run_profile_migration_with_store<S: MigrationStore>(
    project_id: &str,
    mode: &ProfileMigrationMode,
    store: &S,
) -> Result<ProfileMigrationReport, ProfileMigrationError> {
    let classified = store
        .load()
        .await
        .map_err(|error| error.with_report(None))?;
    let mut report = report_for(project_id, &classified, 0);
    if *mode == ProfileMigrationMode::Audit {
        return Ok(report);
    }

    let Ok(targets) = preflight_targets(&classified) else {
        return Err(migration_error(
            ProfileMigrationErrorKind::BlockedPreflight,
            Some(report),
        ));
    };

    for target in targets {
        let outcome = store
            .migrate(&target)
            .await
            .map_err(|error| error.with_report(Some(report.clone())))?;
        match outcome {
            TransactionOutcome::Migrated => report.migrated += 1,
            TransactionOutcome::AlreadyCurrent => {}
            TransactionOutcome::ConcurrentChange => {
                return Err(migration_error(
                    ProfileMigrationErrorKind::ConcurrentChange,
                    Some(report),
                ));
            }
        }
    }

    let verified = store.load().await.map_err(|error| {
        error.reclassify(
            ProfileMigrationErrorKind::Verification,
            Some(report.clone()),
        )
    })?;
    let mut verified_report = report_for(project_id, &verified, report.migrated);
    if verified
        .iter()
        .any(|record| !matches!(record.classification, Classification::Current))
    {
        verified_report.migrated = report.migrated;
        return Err(migration_error(
            ProfileMigrationErrorKind::Verification,
            Some(verified_report),
        ));
    }
    Ok(verified_report)
}

async fn load_classified(db: &FirestoreDb) -> Result<Vec<ClassifiedRecord>, ProfileMigrationError> {
    let mut stream = db
        .fluent()
        .select()
        .from(PROFILES_COLLECTION)
        .stream_query_with_errors()
        .await
        .map_err(|error| {
            migration_error_with_source(ProfileMigrationErrorKind::Read, None, error)
        })?;
    let mut records = Vec::new();
    while let Some(document) = stream.next().await {
        let document = document.map_err(|error| {
            migration_error_with_source(ProfileMigrationErrorKind::Read, None, error)
        })?;
        records.push(classify_document(&document));
    }
    records.sort_by(|left, right| left.document_id.cmp(&right.document_id));
    Ok(records)
}

async fn migrate_one(
    db: &FirestoreDb,
    target: &MigrationTarget,
) -> Result<TransactionOutcome, ProfileMigrationError> {
    let target = target.clone();
    db.run_transaction(|db, transaction| {
        let target = target.clone();
        Box::pin(async move {
            let current = db
                .fluent()
                .select()
                .by_id_in(PROFILES_COLLECTION)
                .one(&target.document_id)
                .await
                .map_err(transaction_firestore_error)?;
            let Some(current) = current else {
                return Ok(TransactionOutcome::ConcurrentChange);
            };
            match migration_decision(&current, &target) {
                MigrationDecision::AlreadyCurrent => {
                    Ok::<_, BackoffError<FirestoreError>>(TransactionOutcome::AlreadyCurrent)
                }
                MigrationDecision::Migrate(profile) => {
                    let update_time =
                        from_timestamp(target.update_time).map_err(transaction_firestore_error)?;
                    db.fluent()
                        .update()
                        .in_col(PROFILES_COLLECTION)
                        .precondition(FirestoreWritePrecondition::UpdateTime(update_time))
                        .document_id(&target.document_id)
                        .object(&profile)
                        .add_to_transaction(transaction)
                        .map_err(transaction_firestore_error)?;
                    Ok(TransactionOutcome::Migrated)
                }
                MigrationDecision::ConcurrentChange => Ok(TransactionOutcome::ConcurrentChange),
            }
        })
    })
    .await
    .map_err(|error| migration_error_with_source(ProfileMigrationErrorKind::Write, None, error))
}

fn migration_decision(document: &FirestoreDocument, target: &MigrationTarget) -> MigrationDecision {
    if document.update_time.as_ref() != Some(&target.update_time) {
        return MigrationDecision::ConcurrentChange;
    }
    match classify_document(document).classification {
        Classification::Current => MigrationDecision::AlreadyCurrent,
        Classification::Legacy(profile) => MigrationDecision::Migrate(profile),
        Classification::Blocked(_) => MigrationDecision::ConcurrentChange,
    }
}

fn classify_document(document: &FirestoreDocument) -> ClassifiedRecord {
    let Some(document_id) = document_id(document) else {
        return ClassifiedRecord {
            document_id: document.name.clone(),
            update_time: document.update_time,
            classification: Classification::Blocked(ProfileMigrationReason::UnexpectedShape),
        };
    };
    let fields = document
        .fields
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let canonical = CANONICAL_PROFILE_FIELDS
        .into_iter()
        .collect::<BTreeSet<_>>();
    let legacy = LEGACY_FIELDS.into_iter().collect::<BTreeSet<_>>();
    let classification = if document.update_time.is_none() {
        Classification::Blocked(ProfileMigrationReason::UnexpectedShape)
    } else if fields == canonical {
        classify_canonical(document, &document_id)
    } else if fields == legacy {
        classify_legacy(document, &document_id)
    } else {
        Classification::Blocked(ProfileMigrationReason::UnexpectedShape)
    };
    ClassifiedRecord {
        document_id,
        update_time: document.update_time,
        classification,
    }
}

fn classify_canonical(document: &FirestoreDocument, document_id: &str) -> Classification {
    let Some(profile) = profile_from_fields(document, false) else {
        return Classification::Blocked(ProfileMigrationReason::InvalidFieldType);
    };
    validate_profile(profile, document_id, false)
}

fn classify_legacy(document: &FirestoreDocument, document_id: &str) -> Classification {
    let Some(profile) = profile_from_fields(document, true) else {
        return Classification::Blocked(ProfileMigrationReason::InvalidFieldType);
    };
    validate_profile(profile, document_id, true)
}

fn profile_from_fields(document: &FirestoreDocument, legacy: bool) -> Option<Profile> {
    let id = string_field(document, "id")?.to_owned();
    let first_name = string_field(document, if legacy { "firstname" } else { "firstName" })?;
    let last_name = string_field(document, if legacy { "lastname" } else { "lastName" })?;
    let contact_email = string_field(document, if legacy { "email" } else { "contactEmail" })?;
    let phone_number = string_field(document, "phoneNumber")?;
    let marketing_opt_in = bool_field(
        document,
        if legacy {
            "marketing"
        } else {
            "marketingOptIn"
        },
    )?;
    let terms_accepted = bool_field(document, if legacy { "terms" } else { "termsAccepted" })?;
    let created_at = string_field(document, "createdAt")?;
    let updated_at = string_field(document, "updatedAt")?;

    Some(Profile {
        id,
        first_name: first_name.to_owned(),
        last_name: last_name.to_owned(),
        contact_email: if legacy {
            normalize_contact_email(contact_email)?
        } else {
            contact_email.to_owned()
        },
        phone_number: if legacy {
            normalize_phone_number(phone_number)?
        } else {
            phone_number.to_owned()
        },
        marketing_opt_in,
        terms_accepted,
        created_at: if legacy {
            normalize_legacy_timestamp(created_at)?
        } else {
            created_at.to_owned()
        },
        updated_at: if legacy {
            normalize_legacy_timestamp(updated_at)?
        } else {
            updated_at.to_owned()
        },
    })
}

fn validate_profile(profile: Profile, document_id: &str, legacy: bool) -> Classification {
    if profile_document_id(&profile.id) != document_id {
        return Classification::Blocked(ProfileMigrationReason::DocumentIdMismatch);
    }
    if validate_stored_profile(
        &profile,
        &profile.id,
        crate::services::profile::ProfileOperation::Get,
    )
    .is_err()
    {
        return Classification::Blocked(ProfileMigrationReason::InvalidValue);
    }
    if legacy {
        Classification::Legacy(profile)
    } else {
        Classification::Current
    }
}

fn string_field<'a>(document: &'a FirestoreDocument, name: &str) -> Option<&'a str> {
    match document.fields.get(name)?.value_type.as_ref()? {
        ValueType::StringValue(value) => Some(value),
        _ => None,
    }
}

fn bool_field(document: &FirestoreDocument, name: &str) -> Option<bool> {
    match document.fields.get(name)?.value_type.as_ref()? {
        ValueType::BooleanValue(value) => Some(*value),
        _ => None,
    }
}

fn normalize_legacy_timestamp(value: &str) -> Option<String> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(canonical_clock_timestamp)
}

fn document_id(document: &FirestoreDocument) -> Option<String> {
    let marker = format!("/documents/{PROFILES_COLLECTION}/");
    let value = document.name.split_once(&marker)?.1;
    (!value.is_empty() && !value.contains('/')).then(|| value.to_owned())
}

fn preflight_targets(records: &[ClassifiedRecord]) -> Result<Vec<MigrationTarget>, ()> {
    if records
        .iter()
        .any(|record| matches!(record.classification, Classification::Blocked(_)))
    {
        return Err(());
    }
    records
        .iter()
        .filter(|record| matches!(record.classification, Classification::Legacy(_)))
        .map(|record| {
            Ok(MigrationTarget {
                document_id: record.document_id.clone(),
                update_time: record.update_time.ok_or(())?,
            })
        })
        .collect()
}

fn report_for(
    project_id: &str,
    records: &[ClassifiedRecord],
    migrated: usize,
) -> ProfileMigrationReport {
    ProfileMigrationReport {
        project_id: project_id.to_owned(),
        records: records
            .iter()
            .map(|record| {
                let (status, reason) = match &record.classification {
                    Classification::Current => (
                        ProfileMigrationStatus::Current,
                        ProfileMigrationReason::CanonicalProfile,
                    ),
                    Classification::Legacy(_) => (
                        ProfileMigrationStatus::MigrationRequired,
                        ProfileMigrationReason::LegacyProfile,
                    ),
                    Classification::Blocked(reason) => (ProfileMigrationStatus::Blocked, *reason),
                };
                ProfileMigrationRecord {
                    fingerprint: fingerprint(&record.document_id),
                    status,
                    reason,
                }
            })
            .collect(),
        migrated,
    }
}

fn fingerprint(document_id: &str) -> String {
    let digest = Sha256::digest(format!("{PROFILES_COLLECTION}/{document_id}"));
    format!("sha256:{}", URL_SAFE_NO_PAD.encode(&digest[..18]))
}

fn migration_error(
    kind: ProfileMigrationErrorKind,
    report: Option<ProfileMigrationReport>,
) -> ProfileMigrationError {
    ProfileMigrationError {
        kind,
        report,
        source: None,
    }
}

fn migration_error_with_source(
    kind: ProfileMigrationErrorKind,
    report: Option<ProfileMigrationReport>,
    source: impl Error + Send + Sync + 'static,
) -> ProfileMigrationError {
    ProfileMigrationError {
        kind,
        report,
        source: Some(Arc::new(source)),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use firestore::FirestoreDb;
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct LegacyProfile<'a> {
        id: &'a str,
        firstname: &'a str,
        lastname: &'a str,
        email: &'a str,
        phone_number: &'a str,
        marketing: bool,
        terms: bool,
        created_at: &'a str,
        updated_at: &'a str,
    }

    fn path(document_id: &str) -> String {
        format!("projects/test/databases/(default)/documents/{PROFILES_COLLECTION}/{document_id}")
    }

    fn test_update_time(seconds: i64) -> Timestamp {
        Timestamp { seconds, nanos: 0 }
    }

    fn legacy_document() -> FirestoreDocument {
        let id = "user-123";
        let mut document = FirestoreDb::serialize_to_doc(
            path(&profile_document_id(id)),
            &LegacyProfile {
                id,
                firstname: "Ada",
                lastname: "Lovelace",
                email: "ada@EXAMPLE.COM",
                phone_number: "+358401234567",
                marketing: false,
                terms: true,
                created_at: "2026-07-30T14:00:00.123456+02:00",
                updated_at: "2026-07-30T12:05:00.999999Z",
            },
        )
        .expect("legacy document");
        document.update_time = Some(test_update_time(1));
        document
    }

    fn canonical_profile() -> Profile {
        Profile {
            id: "user-123".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            contact_email: "ada@example.com".to_owned(),
            phone_number: "+358401234567".to_owned(),
            marketing_opt_in: false,
            terms_accepted: true,
            created_at: "2026-07-30T12:00:00.123Z".to_owned(),
            updated_at: "2026-07-30T12:05:00.999Z".to_owned(),
        }
    }

    fn canonical_document() -> FirestoreDocument {
        let profile = canonical_profile();
        let mut document =
            FirestoreDb::serialize_to_doc(path(&profile_document_id(&profile.id)), &profile)
                .expect("canonical document");
        document.update_time = Some(test_update_time(2));
        document
    }

    struct StubMigrationStore {
        loads: Mutex<VecDeque<Result<Vec<ClassifiedRecord>, ProfileMigrationError>>>,
        outcomes: Mutex<VecDeque<Result<TransactionOutcome, ProfileMigrationError>>>,
        migrated_targets: Mutex<Vec<MigrationTarget>>,
    }

    impl StubMigrationStore {
        fn new(
            loads: impl IntoIterator<Item = Result<Vec<ClassifiedRecord>, ProfileMigrationErrorKind>>,
            outcomes: impl IntoIterator<Item = Result<TransactionOutcome, ProfileMigrationErrorKind>>,
        ) -> Self {
            Self {
                loads: Mutex::new(
                    loads
                        .into_iter()
                        .map(|result| result.map_err(|kind| migration_error(kind, None)))
                        .collect(),
                ),
                outcomes: Mutex::new(
                    outcomes
                        .into_iter()
                        .map(|result| result.map_err(|kind| migration_error(kind, None)))
                        .collect(),
                ),
                migrated_targets: Mutex::new(Vec::new()),
            }
        }

        fn push_load_error(&self, kind: ProfileMigrationErrorKind, message: &'static str) {
            self.loads.lock().expect("load queue lock").push_back(Err(
                migration_error_with_source(kind, None, std::io::Error::other(message)),
            ));
        }

        fn push_outcome_error(&self, kind: ProfileMigrationErrorKind, message: &'static str) {
            self.outcomes
                .lock()
                .expect("outcome queue lock")
                .push_back(Err(migration_error_with_source(
                    kind,
                    None,
                    std::io::Error::other(message),
                )));
        }
    }

    impl MigrationStore for StubMigrationStore {
        async fn load(&self) -> Result<Vec<ClassifiedRecord>, ProfileMigrationError> {
            self.loads
                .lock()
                .expect("load queue lock")
                .pop_front()
                .unwrap_or_else(|| Err(migration_error(ProfileMigrationErrorKind::Read, None)))
        }

        async fn migrate(
            &self,
            target: &MigrationTarget,
        ) -> Result<TransactionOutcome, ProfileMigrationError> {
            self.migrated_targets
                .lock()
                .expect("migration target lock")
                .push(target.clone());
            self.outcomes
                .lock()
                .expect("outcome queue lock")
                .pop_front()
                .unwrap_or_else(|| Err(migration_error(ProfileMigrationErrorKind::Write, None)))
        }
    }

    #[test]
    fn exact_legacy_shape_maps_to_the_canonical_profile() {
        let classified = classify_document(&legacy_document());
        let Classification::Legacy(profile) = classified.classification else {
            panic!("expected legacy profile");
        };
        assert_eq!(profile, canonical_profile());

        let replacement = FirestoreDb::serialize_to_doc(path(&classified.document_id), &profile)
            .expect("canonical replacement");
        assert_eq!(
            replacement
                .fields
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            CANONICAL_PROFILE_FIELDS
                .into_iter()
                .collect::<BTreeSet<_>>()
        );
        assert!(!replacement.fields.contains_key("firstname"));
        assert!(!replacement.fields.contains_key("email"));
    }

    #[test]
    fn canonical_shape_is_idempotent() {
        let document = canonical_document();
        assert!(matches!(
            classify_document(&document).classification,
            Classification::Current
        ));
    }

    #[test]
    fn mixed_unknown_invalid_and_mismatched_records_block_the_whole_plan() {
        let mut mixed = legacy_document();
        mixed
            .fields
            .insert("contactEmail".to_owned(), mixed.fields["email"].clone());
        let mut wrong_type = legacy_document();
        wrong_type.fields.insert(
            "terms".to_owned(),
            gcloud_sdk::google::firestore::v1::Value {
                value_type: Some(ValueType::StringValue("true".to_owned())),
            },
        );
        let mut mismatch = legacy_document();
        mismatch.name = path("uid_b3RoZXI");
        let mut missing_version = legacy_document();
        missing_version.update_time = None;
        let records = [mixed, wrong_type, mismatch, missing_version]
            .iter()
            .map(classify_document)
            .collect::<Vec<_>>();
        assert!(
            records
                .iter()
                .all(|record| matches!(record.classification, Classification::Blocked(_)))
        );
        assert_eq!(preflight_targets(&records), Err(()));
    }

    #[test]
    fn migration_decision_requires_the_exact_preflight_version() {
        let audited = classify_document(&legacy_document());
        let target = preflight_targets(&[audited])
            .expect("preflight")
            .pop()
            .expect("migration target");

        let MigrationDecision::Migrate(profile) = migration_decision(&legacy_document(), &target)
        else {
            panic!("unchanged legacy document should migrate");
        };
        assert_eq!(profile, canonical_profile());

        let mut changed = legacy_document();
        changed.update_time = Some(test_update_time(2));
        changed.fields.insert(
            "firstname".to_owned(),
            gcloud_sdk::google::firestore::v1::Value {
                value_type: Some(ValueType::StringValue("Grace".to_owned())),
            },
        );
        assert!(matches!(
            migration_decision(&changed, &target),
            MigrationDecision::ConcurrentChange
        ));
    }

    #[test]
    fn preflight_selects_only_legacy_records_and_reports_only_hashes() {
        let legacy = classify_document(&legacy_document());
        let current = classify_document(&canonical_document());
        let targets = preflight_targets(&[legacy, current]).expect("preflight");
        assert_eq!(
            targets,
            [MigrationTarget {
                document_id: profile_document_id("user-123"),
                update_time: test_update_time(1),
            }]
        );

        let report = report_for("test", &[classify_document(&legacy_document())], 0);
        assert!(!report.records[0].fingerprint.contains("user-123"));
        assert!(!report.records[0].fingerprint.contains("uid_"));
        assert_eq!(report.migration_required(), 1);
        assert_eq!(report.blocked(), 0);
        assert_eq!(report.current(), 0);
        assert!(report.records[0].fingerprint.starts_with("sha256:"));
        assert_eq!(report.records[0].fingerprint.len(), 31);
        assert_eq!(fingerprint("same"), fingerprint("same"));
        assert_ne!(fingerprint("same"), fingerprint("different"));
    }

    #[test]
    fn migration_report_labels_counts_and_errors_are_exact() {
        let statuses = [
            (ProfileMigrationStatus::Current, "current"),
            (
                ProfileMigrationStatus::MigrationRequired,
                "migration_required",
            ),
            (ProfileMigrationStatus::Blocked, "blocked"),
        ];
        for (status, expected) in statuses {
            assert_eq!(status.to_string(), expected);
        }
        let reasons = [
            (
                ProfileMigrationReason::CanonicalProfile,
                "canonical_profile",
            ),
            (ProfileMigrationReason::LegacyProfile, "legacy_profile"),
            (ProfileMigrationReason::UnexpectedShape, "unexpected_shape"),
            (
                ProfileMigrationReason::InvalidFieldType,
                "invalid_field_type",
            ),
            (ProfileMigrationReason::InvalidValue, "invalid_value"),
            (
                ProfileMigrationReason::DocumentIdMismatch,
                "document_id_mismatch",
            ),
        ];
        for (reason, expected) in reasons {
            assert_eq!(reason.to_string(), expected);
        }

        let report = ProfileMigrationReport {
            project_id: "test".to_owned(),
            records: statuses
                .into_iter()
                .enumerate()
                .map(|(index, (status, _))| ProfileMigrationRecord {
                    fingerprint: format!("fingerprint-{index}"),
                    status,
                    reason: ProfileMigrationReason::CanonicalProfile,
                })
                .collect(),
            migrated: 2,
        };
        assert_eq!(report.current(), 1);
        assert_eq!(report.migration_required(), 1);
        assert_eq!(report.blocked(), 1);

        let messages = [
            (
                ProfileMigrationErrorKind::ProjectConfirmation,
                "the explicit project confirmation does not match the configured project",
            ),
            (
                ProfileMigrationErrorKind::Initialize,
                "the profile migration could not initialize Firestore",
            ),
            (
                ProfileMigrationErrorKind::Read,
                "the profile migration audit could not complete",
            ),
            (
                ProfileMigrationErrorKind::BlockedPreflight,
                "the profile migration is blocked by noncanonical persisted data",
            ),
            (
                ProfileMigrationErrorKind::ConcurrentChange,
                "a profile changed after audit; rerun the audit before applying",
            ),
            (
                ProfileMigrationErrorKind::Write,
                "a profile migration transaction did not complete",
            ),
            (
                ProfileMigrationErrorKind::Verification,
                "post-migration verification did not find only canonical profiles",
            ),
        ];
        for (kind, message) in messages {
            let error = migration_error(kind, Some(report.clone()));
            assert_eq!(error.to_string(), message);
            assert_eq!(error.report(), Some(&report));
            let debug = format!("{error:?}");
            assert!(debug.contains(&format!("kind: {kind:?}")));
            assert!(debug.contains("has_report: true"));
            assert!(debug.contains("has_source: false"));
        }
    }

    #[tokio::test]
    async fn migration_engine_preserves_dependency_sources_without_exposing_them() {
        let read_store = StubMigrationStore::new([], []);
        read_store.push_load_error(ProfileMigrationErrorKind::Read, "private read sentinel");
        let read_error =
            run_profile_migration_with_store("test", &ProfileMigrationMode::Audit, &read_store)
                .await
                .expect_err("read failure");
        assert_migration_source(
            &read_error,
            ProfileMigrationErrorKind::Read,
            "private read sentinel",
            false,
        );

        let legacy = classify_document(&legacy_document());
        let write_store = StubMigrationStore::new([Ok(vec![legacy.clone()])], []);
        write_store.push_outcome_error(ProfileMigrationErrorKind::Write, "private write sentinel");
        let write_error = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &write_store,
        )
        .await
        .expect_err("write failure");
        assert_migration_source(
            &write_error,
            ProfileMigrationErrorKind::Write,
            "private write sentinel",
            true,
        );

        let verification_store =
            StubMigrationStore::new([Ok(vec![legacy])], [Ok(TransactionOutcome::AlreadyCurrent)]);
        verification_store.push_load_error(
            ProfileMigrationErrorKind::Read,
            "private verification sentinel",
        );
        let verification_error = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &verification_store,
        )
        .await
        .expect_err("verification read failure");
        assert_migration_source(
            &verification_error,
            ProfileMigrationErrorKind::Verification,
            "private verification sentinel",
            true,
        );
    }

    fn assert_migration_source(
        error: &ProfileMigrationError,
        kind: ProfileMigrationErrorKind,
        source: &str,
        has_report: bool,
    ) {
        assert_eq!(error.kind(), kind);
        assert_eq!(error.report().is_some(), has_report);
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some(source)
        );
        assert!(!error.to_string().contains(source));
        let debug = format!("{error:?}");
        assert!(!debug.contains(source));
        assert!(debug.contains("has_source: true"));
    }

    #[tokio::test]
    async fn migration_engine_audits_blocks_applies_and_verifies_deterministically() {
        let legacy = classify_document(&legacy_document());
        let current = classify_document(&canonical_document());

        let audit_store = StubMigrationStore::new([Ok(vec![legacy.clone(), current.clone()])], []);
        let audit =
            run_profile_migration_with_store("test", &ProfileMigrationMode::Audit, &audit_store)
                .await
                .expect("audit");
        assert_eq!(audit.current(), 1);
        assert_eq!(audit.migration_required(), 1);
        assert_eq!(audit.migrated, 0);
        assert!(audit_store.migrated_targets.lock().unwrap().is_empty());

        let blocked = ClassifiedRecord {
            document_id: "blocked".to_owned(),
            update_time: None,
            classification: Classification::Blocked(ProfileMigrationReason::UnexpectedShape),
        };
        let blocked_store = StubMigrationStore::new([Ok(vec![blocked])], []);
        let blocked_error = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &blocked_store,
        )
        .await
        .expect_err("blocked preflight");
        assert_eq!(
            blocked_error.kind(),
            ProfileMigrationErrorKind::BlockedPreflight
        );
        assert_eq!(
            blocked_error.report().map(ProfileMigrationReport::blocked),
            Some(1)
        );
        assert!(blocked_store.migrated_targets.lock().unwrap().is_empty());

        let apply_store = StubMigrationStore::new(
            [Ok(vec![legacy.clone()]), Ok(vec![current.clone()])],
            [Ok(TransactionOutcome::Migrated)],
        );
        let applied = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &apply_store,
        )
        .await
        .expect("apply");
        assert_eq!(applied.current(), 1);
        assert_eq!(applied.migration_required(), 0);
        assert_eq!(applied.migrated, 1);
        assert_eq!(
            *apply_store.migrated_targets.lock().unwrap(),
            vec![MigrationTarget {
                document_id: profile_document_id("user-123"),
                update_time: test_update_time(1),
            }]
        );

        let concurrent_store = StubMigrationStore::new(
            [Ok(vec![legacy.clone()])],
            [Ok(TransactionOutcome::ConcurrentChange)],
        );
        let concurrent = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &concurrent_store,
        )
        .await
        .expect_err("concurrent change");
        assert_eq!(
            concurrent.kind(),
            ProfileMigrationErrorKind::ConcurrentChange
        );

        let verification_store = StubMigrationStore::new(
            [Ok(vec![legacy.clone()]), Ok(vec![legacy])],
            [Ok(TransactionOutcome::AlreadyCurrent)],
        );
        let verification = run_profile_migration_with_store(
            "test",
            &ProfileMigrationMode::Apply {
                confirmed_project: "test".to_owned(),
            },
            &verification_store,
        )
        .await
        .expect_err("verification must require canonical data");
        assert_eq!(verification.kind(), ProfileMigrationErrorKind::Verification);
        assert_eq!(verification.report().map(|report| report.migrated), Some(0));
    }

    #[test]
    fn document_ids_reject_empty_nested_and_wrong_collection_paths() {
        let mut document = legacy_document();
        assert_eq!(
            document_id(&document),
            Some(profile_document_id("user-123"))
        );
        for name in [
            "projects/test/databases/(default)/documents/profiles/",
            "projects/test/databases/(default)/documents/profiles/one/two",
            "projects/test/databases/(default)/documents/other/value",
        ] {
            document.name = name.to_owned();
            assert_eq!(document_id(&document), None);
        }
    }

    #[tokio::test]
    async fn unsafe_emulator_configuration_fails_before_initializing_a_client() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "test".to_owned(),
            app_environment: crate::AppEnvironment::Production,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: Some("firestore.example:8080".to_owned()),
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };
        let error = run_profile_migration(&config, ProfileMigrationMode::Audit)
            .await
            .expect_err("unsafe emulator host must fail");
        assert_eq!(error.kind(), ProfileMigrationErrorKind::Initialize);
        assert!(error.report().is_none());
    }

    #[tokio::test]
    async fn safe_emulator_host_reaches_confirmation_before_initializing_a_client() {
        let config = AppConfig {
            port: 8080,
            firebase_project_id: "configured-project".to_owned(),
            app_environment: crate::AppEnvironment::Development,
            google_application_credentials: None,
            firebase_auth_emulator_host: None,
            firestore_emulator_host: Some("127.0.0.1:8080".to_owned()),
            google_cloud_project: None,
            gcp_project: None,
            gcloud_project: None,
            project_id: None,
        };
        let error = run_profile_migration(
            &config,
            ProfileMigrationMode::Apply {
                confirmed_project: "other-project".to_owned(),
            },
        )
        .await
        .expect_err("mismatched project confirmation must fail");
        assert_eq!(error.kind(), ProfileMigrationErrorKind::ProjectConfirmation);
        assert!(error.report().is_none());
    }
}
