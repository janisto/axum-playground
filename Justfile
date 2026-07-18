set dotenv-load
set shell := ["bash", "-ceu"]

# Container runtime: prefer podman, fallback to docker
CONTAINER_RUNTIME := if `command -v podman 2>/dev/null || true` != "" { "podman" } else { "docker" }

@_:
    just --list

[group('build')]
build:
    cargo build --locked

[group('build')]
docker-build:
    {{ CONTAINER_RUNTIME }} build -t axum-playground:dev .

[group('run')]
run:
    cargo run --locked

[group('lifecycle')]
install: download install-tools

[group('lifecycle')]
install-tools:
    cargo install --locked cargo-edit
    cargo install --locked cargo-nextest
    cargo install --locked cargo-llvm-cov
    cargo install --locked cargo-deny
    cargo install --locked cargo-audit
    cargo install --locked cargo-mutants
    cargo install --locked cargo-sort
    cargo install --locked cargo-machete

[group('lifecycle')]
download:
    cargo fetch --locked

[group('lifecycle')]
update:
    cargo upgrade --incompatible allow
    cargo update
    cargo fetch --locked

[group('qa')]
fmt:
    cargo fmt --all

[group('qa')]
fmt-check:
    cargo fmt --all -- --check

[group('qa')]
lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

[group('qa')]
doc:
    RUSTDOCFLAGS="-D rustdoc::all" cargo doc --locked --all-features --no-deps

[group('qa')]
sort-check:
    cargo sort --check --grouped

[group('qa')]
unused-dependencies:
    cargo machete

[group('qa')]
workflow-check:
    actionlint
    zizmor --offline .

[group('test')]
test:
    cargo nextest run --locked

[group('test')]
test-doc:
    cargo test --doc --locked

[group('test')]
test-emulators:
    if [[ -z "${FIRESTORE_EMULATOR_HOST:-}" ]]; then echo "Skipping Firestore emulator tests because FIRESTORE_EMULATOR_HOST is unset."; else cargo test --locked --test firestore_emulator -- --ignored; fi

[group('test')]
test-emulators-ci:
    firebase emulators:exec --only firestore --project demo-test-project \
        "cargo test --locked --test firestore_emulator -- --ignored"

[group('adversarial')]
mutations *args:
    cargo mutants {{ args }}

[group('qa')]
coverage-lcov:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v brew >/dev/null && [[ "$(rustc --print sysroot)" == "$(brew --cellar rust)"/* ]]; then
        export LLVM_COV="$(brew --prefix llvm)/bin/llvm-cov"
        export LLVM_PROFDATA="$(brew --prefix llvm)/bin/llvm-profdata"
    fi
    cargo llvm-cov nextest --locked --lcov --output-path coverage.lcov

[group('qa')]
coverage-html:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v brew >/dev/null && [[ "$(rustc --print sysroot)" == "$(brew --cellar rust)"/* ]]; then
        export LLVM_COV="$(brew --prefix llvm)/bin/llvm-cov"
        export LLVM_PROFDATA="$(brew --prefix llvm)/bin/llvm-profdata"
    fi
    cargo llvm-cov nextest --locked --html

[group('qa')]
deny:
    cargo deny check

[group('qa')]
audit:
    cargo audit

[group('qa')]
qa: workflow-check fmt-check sort-check unused-dependencies lint build test test-doc doc deny audit

[group('qa')]
check: qa
    just test-emulators

[group('qa')]
ci: qa
    just docker-build

[group('lifecycle')]
lock:
    cargo generate-lockfile
