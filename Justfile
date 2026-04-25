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
    {{CONTAINER_RUNTIME}} build -t axum-playground:dev .

[group('run')]
run:
    cargo run --locked

[group('qa')]
fmt:
    cargo fmt --all

[group('qa')]
fmt-check:
    cargo fmt --all --check

[group('qa')]
lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

[group('test')]
test:
    cargo nextest run --locked

[group('test')]
test-doc:
    cargo test --doc --locked

[group('test')]
test-emulators:
    if [[ -z "${FIRESTORE_EMULATOR_HOST:-}" ]]; then echo "Skipping Firestore emulator tests because FIRESTORE_EMULATOR_HOST is unset."; else cargo test --locked --test firestore_emulator; fi

[group('qa')]
coverage-lcov:
    toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml); \
    rustup run "$toolchain" cargo llvm-cov nextest --lcov --output-path coverage.lcov

[group('qa')]
coverage-html:
    toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml); \
    rustup run "$toolchain" cargo llvm-cov nextest --html

[group('qa')]
deny:
    cargo deny check

[group('qa')]
audit:
    cargo audit

[group('qa')]
check:
    just fmt-check
    just lint
    just test
    just test-emulators

[group('qa')]
ci:
    just fmt-check
    just lint
    just test
    just test-doc
    just deny
    just audit
    just docker-build

[group('lifecycle')]
lock:
    cargo generate-lockfile
