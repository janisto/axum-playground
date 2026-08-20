//! Audits and, with explicit confirmation, migrates persisted profile documents.

use std::{env, process::ExitCode};

use axum_playground::{
    AppConfig,
    profile_migration::{ProfileMigrationMode, ProfileMigrationReport, run_profile_migration},
};

const USAGE: &str = "usage: cargo run --locked --bin migrate_profiles -- [--audit | --apply --confirm-project <project-id>]";

#[tokio::main]
async fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mode = match parse_mode(&args) {
        Ok(Some(mode)) => mode,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(()) => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("profile migration configuration error: {error}");
            return ExitCode::FAILURE;
        }
    };

    match run_profile_migration(&config, mode).await {
        Ok(report) => {
            println!("{}", render_report(&report));
            ExitCode::SUCCESS
        }
        Err(error) => {
            if let Some(report) = error.report() {
                println!("{}", render_report(report));
            }
            eprintln!("profile migration failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_mode(args: &[String]) -> Result<Option<ProfileMigrationMode>, ()> {
    match args {
        [] => Ok(Some(ProfileMigrationMode::Audit)),
        [arg] if arg == "--audit" => Ok(Some(ProfileMigrationMode::Audit)),
        [arg] if matches!(arg.as_str(), "--help" | "-h") => Ok(None),
        [apply, confirm, project]
            if apply == "--apply" && confirm == "--confirm-project" && !project.is_empty() =>
        {
            Ok(Some(ProfileMigrationMode::Apply {
                confirmed_project: project.clone(),
            }))
        }
        _ => Err(()),
    }
}

fn render_report(report: &ProfileMigrationReport) -> String {
    let mut lines = vec![format!(
        "profile_migration project={} total={} current={} migration_required={} blocked={} migrated={}",
        report.project_id,
        report.records.len(),
        report.current(),
        report.migration_required(),
        report.blocked(),
        report.migrated,
    )];
    lines.extend(report.records.iter().map(|record| {
        format!(
            "profile_record fingerprint={} status={} reason={}",
            record.fingerprint, record.status, record.reason
        )
    }));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_is_the_default_and_apply_requires_exact_confirmation_syntax() {
        assert_eq!(parse_mode(&[]), Ok(Some(ProfileMigrationMode::Audit)));
        assert_eq!(
            parse_mode(&["--audit".to_owned()]),
            Ok(Some(ProfileMigrationMode::Audit))
        );
        for help in ["--help", "-h"] {
            assert_eq!(parse_mode(&[help.to_owned()]), Ok(None));
        }
        assert_eq!(
            parse_mode(&[
                "--apply".to_owned(),
                "--confirm-project".to_owned(),
                "project-123".to_owned(),
            ]),
            Ok(Some(ProfileMigrationMode::Apply {
                confirmed_project: "project-123".to_owned()
            }))
        );
        for args in [
            vec!["--apply".to_owned()],
            vec!["--apply".to_owned(), "project-123".to_owned()],
            vec![
                "--apply".to_owned(),
                "--confirm-project".to_owned(),
                String::new(),
            ],
        ] {
            assert_eq!(parse_mode(&args), Err(()));
        }
    }

    #[test]
    fn migration_report_rendering_is_exact_and_non_sensitive() {
        use axum_playground::profile_migration::{
            ProfileMigrationReason, ProfileMigrationRecord, ProfileMigrationStatus,
        };

        let report = ProfileMigrationReport {
            project_id: "demo-project".to_owned(),
            records: vec![
                ProfileMigrationRecord {
                    fingerprint: "sha256:current".to_owned(),
                    status: ProfileMigrationStatus::Current,
                    reason: ProfileMigrationReason::CanonicalProfile,
                },
                ProfileMigrationRecord {
                    fingerprint: "sha256:legacy".to_owned(),
                    status: ProfileMigrationStatus::MigrationRequired,
                    reason: ProfileMigrationReason::LegacyProfile,
                },
                ProfileMigrationRecord {
                    fingerprint: "sha256:blocked".to_owned(),
                    status: ProfileMigrationStatus::Blocked,
                    reason: ProfileMigrationReason::UnexpectedShape,
                },
            ],
            migrated: 1,
        };
        assert_eq!(
            render_report(&report),
            "profile_migration project=demo-project total=3 current=1 migration_required=1 blocked=1 migrated=1\
\nprofile_record fingerprint=sha256:current status=current reason=canonical_profile\
\nprofile_record fingerprint=sha256:legacy status=migration_required reason=legacy_profile\
\nprofile_record fingerprint=sha256:blocked status=blocked reason=unexpected_shape"
        );
    }
}
