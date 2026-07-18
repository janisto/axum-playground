use std::process::Command;

use axum_playground::telemetry::init_tracing;
use serde_json::Value;

const TRACING_CHILD_ENV: &str = "AXUM_PLAYGROUND_TRACING_TEST_CHILD";

#[test]
fn binary_rejects_an_invalid_port() {
    let output = Command::new(env!("CARGO_BIN_EXE_axum-playground"))
        .env("PORT", "not-a-port")
        .env_remove("FIREBASE_AUTH_EMULATOR_HOST")
        .env_remove("FIRESTORE_EMULATOR_HOST")
        .output()
        .expect("application binary should run");

    assert!(!output.status.success(), "invalid PORT should fail startup");

    let stderr = String::from_utf8(output.stderr).expect("startup error should be UTF-8");
    assert!(
        stderr.contains("InvalidPort(\"not-a-port\")"),
        "startup should report the rejected PORT value, got: {stderr}"
    );
}

#[test]
fn binary_rejects_an_unknown_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_axum-playground"))
        .env("APP_ENVIRONMENT", "staging")
        .env_remove("FIREBASE_AUTH_EMULATOR_HOST")
        .env_remove("FIRESTORE_EMULATOR_HOST")
        .output()
        .expect("application binary should run");

    assert!(
        !output.status.success(),
        "unknown APP_ENVIRONMENT should fail startup"
    );

    let stderr = String::from_utf8(output.stderr).expect("startup error should be UTF-8");
    assert!(
        stderr.contains("InvalidEnvironment(\"staging\")"),
        "startup should report the rejected environment, got: {stderr}"
    );
}

#[test]
fn tracing_initialization_emits_structured_json() {
    if std::env::var_os(TRACING_CHILD_ENV).is_some() {
        init_tracing(axum_playground::AppEnvironment::Test)
            .expect("isolated tracing initialization should succeed");
        tracing::info!(mutation_test_marker = true, "tracing initialized");
        return;
    }

    let output = Command::new(std::env::current_exe().expect("test executable should be known"))
        .args([
            "--exact",
            "tracing_initialization_emits_structured_json",
            "--nocapture",
        ])
        .env(TRACING_CHILD_ENV, "1")
        .env("RUST_LOG", "info")
        .output()
        .expect("isolated tracing test should run");

    assert!(
        output.status.success(),
        "isolated tracing test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("tracing output should be UTF-8");
    let event = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["message"] == "tracing initialized")
        .unwrap_or_else(|| panic!("structured tracing event was not emitted: {stdout}"));

    assert_eq!(event["severity"], "INFO");
    assert_eq!(event["mutation_test_marker"], true);
}
