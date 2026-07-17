---
name: rust-testing
description: Write or review axum-playground Rust tests using Tokio, in-process Axum routers, shared test state and body helpers, service doubles, Firebase emulators, cargo-nextest, and repository validation commands.
---

# Rust testing

Read `AGENTS.md`, the implementation and callers under test, `tests/common/mod.rs`, and neighboring tests. Apply `$adversarial-testing` before selecting cases; this skill supplies Rust and repository mechanics, not the risk policy.

## Choose the narrowest useful boundary

- Keep pure helper and data-structure tests beside the module under `src/`.
- Put HTTP contract tests under `tests/` and exercise `build_app(...)` or `build_app_with_routes(...)` with `tower::ServiceExt::oneshot(...)`.
- Use `test_state()`, `test_state_with_github_service(...)`, or `test_state_with_auth_and_profile(...)` instead of rebuilding application state.
- Decode bodies with `read_json_body(...)`, `read_cbor_body(...)`, or `read_text_body(...)` from `tests/common/mod.rs`.
- Use service mocks for deterministic GitHub, authentication, and profile behavior. Reserve real Firestore SDK semantics for the conditional emulator test.

Do not open network ports for ordinary tests, contact live services, add production test switches, use real credentials, or introduce sleeps for coordination.

## Assertions that matter

- Assert exact status, `Content-Type`, and contract headers such as `Link`, `Location`, `WWW-Authenticate`, `Vary`, and `X-Request-Id`.
- Decode and inspect the response model or Problem Details fields that prove the changed behavior.
- Exercise JSON and CBOR only where the route implements both, including request and response selection when relevant.
- Assert validation and authentication stop service execution. Assert cleanup, state mutation, and forbidden side effects at service boundaries.
- For pagination, distinguish first, middle, and terminal pages and cover malformed, wrong-kind, stale, and boundary cursors.
- For OpenAPI changes, inspect `/v1/openapi` for the affected path, status, headers, and media types instead of snapshotting the whole document.
- Ask which plausible removal, inversion, off-by-one error, or stale-state bug each test catches; strengthen or remove cases that would survive the mutation.

## Commands

Run the narrowest target first while preserving the lockfile:

```bash
cargo test --locked --test api_health
cargo test --locked --test api_hello
cargo test --locked --test api_items
cargo test --locked --test api_profile
cargo test --locked --test api_github
```

Then use the applicable repository gates:

```bash
just fmt-check
just lint
just test
just test-doc
just check
```

Run `just test-emulators` when Firestore emulator behavior changes. Ordinary local runs may skip that test when `FIRESTORE_EMULATOR_HOST` is unset; report the skipped external validation explicitly.
