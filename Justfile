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

[group('build')]
container-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    image=localhost/axum-playground:ci
    container=axum-playground-smoke
    cleanup() {
        smoke_status=$?
        if [[ "$smoke_status" != 0 ]]; then
            {{ CONTAINER_RUNTIME }} logs "$container" 2>/dev/null || true
        fi
        {{ CONTAINER_RUNTIME }} rm --force "$container" >/dev/null 2>&1 || true
        exit "$smoke_status"
    }
    trap cleanup EXIT
    {{ CONTAINER_RUNTIME }} build --tag "$image" .
    {{ CONTAINER_RUNTIME }} inspect "$image" | jq --exit-status '.[0].Config.User == "65532:65532"'
    {{ CONTAINER_RUNTIME }} run --detach --pull=never --read-only --name "$container" \
        --publish 127.0.0.1:18087:8080 \
        --env APP_ENVIRONMENT=test \
        --env FIREBASE_PROJECT_ID=demo-test-project \
        --env FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:7110 \
        --env FIRESTORE_EMULATOR_HOST=127.0.0.1:7130 \
        --env GOOGLE_APPLICATION_CREDENTIALS=/no-google-credentials.json \
        "$image"
    curl --fail --silent --show-error --retry 15 --retry-connrefused --retry-delay 1 \
        --max-time 2 http://127.0.0.1:18087/health | jq --exit-status '.status == "healthy"'
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/v1/hello \
        | jq --exit-status '.message == "Hello, World!"'
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/openapi.json \
        | jq --exit-status '.openapi == "3.1.0" and (.paths["/v1/profile"].get.security == [{"bearerAuth":[]}])'
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/.well-known/api-catalog \
        | jq --exit-status '.linkset[0]["service-desc"][0].href == "/openapi.json" and (.linkset[0]["service-doc"] | length) == 2'
    curl --head --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/.well-known/api-catalog >/dev/null
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/llms.txt | cmp - llms.txt
    curl --fail --silent --show-error --max-time 2 http://127.0.0.1:18087/llms-full.txt | cmp - llms-full.txt
    {{ CONTAINER_RUNTIME }} stop --time 10 "$container"
    {{ CONTAINER_RUNTIME }} inspect "$container" | jq --exit-status '.[0].State.ExitCode == 0'

[group('run')]
run:
    cargo run --locked --bin axum-playground

[group('lifecycle')]
install: download install-tools

[group('lifecycle')]
install-tools:
    cargo install --locked cargo-edit --version 0.13.14
    cargo install --locked cargo-nextest --version 0.9.151
    cargo install --locked cargo-llvm-cov --version 0.9.1
    cargo install --locked cargo-deny --version 0.20.2
    cargo install --locked cargo-audit --version 0.22.2
    cargo install --locked cargo-mutants --version 27.1.0
    cargo install --locked cargo-sort --version 2.1.4
    cargo install --locked cargo-machete --version 0.9.2
    cargo install --locked zizmor --version 1.32.0

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
    zizmor --offline --persona pedantic .

[group('test')]
test:
    cargo nextest run --locked

[group('test')]
test-doc:
    cargo test --doc --locked

[group('run')]
emulators:
    npx --yes firebase-tools@15.34.2 emulators:start --only auth,firestore --project demo-test-project

[group('test')]
test-emulators:
    if [[ -z "${FIRESTORE_EMULATOR_HOST:-}" ]]; then echo "Skipping Firestore emulator tests because FIRESTORE_EMULATOR_HOST is unset."; else cargo test --locked --test firestore_emulator -- --ignored; fi

[group('test')]
test-emulators-ci:
    npx --yes firebase-tools@15.34.2 emulators:exec --only firestore --project demo-test-project \
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
