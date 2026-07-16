# AGENTS.md

Instructions for coding agents working in this repository.

`README.md` is for human users and contributors: setup, capabilities, architecture, operations, and contribution entry points. `AGENTS.md` is for coding agents: execution rules, implementation constraints, and validation policy. Do not duplicate agent instructions into the README or turn this file into human onboarding documentation.

## Engineering priorities

- Correctness first, then readability and maintainability, then performance.
- Inspect the relevant implementation, callers, and existing tests before changing behavior.
- Prefer the smallest safe change that solves the problem.
- Reuse existing local patterns and utilities, refactoring them when needed, instead of creating parallel abstractions or adding dependencies.
- State the failure mode before architectural, security, persistence, or production-impacting changes.
- Do not declare completion until implementation, validation, and remaining risks are reported.
- Keep source comments and documentation concise. Do not add progress narration, generated banners, emojis, or speculative TODOs.

## Pull requests

- Format titles as `type[optional scope]: description`. Prefer no scope; include one only when it materially improves clarity.
- Use `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`, `chore`, or `revert` as the type. Example: `feat: add response size field`.
- Keep each pull request focused. In the body, explain why the change is needed, what changed, how it was validated, and any remaining risk.
- Keep the title suitable for the final squash or merge commit.
- This repository does not maintain a `CHANGELOG.md`; do not create one or require changelog entries in pull requests.

## Commits

- Follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).
- Prefer no scope; include one only when it materially improves clarity. Write a short, imperative description. Example: `fix: preserve request ID`.
- Mark breaking changes with `!` and explain them in a `BREAKING CHANGE:` footer.
- Before committing, run `just qa` and `git diff --check`.

## Mandatory skills

- Use `.agents/skills/adversarial-testing/SKILL.md` for every task that plans, creates, modifies, reviews, debugs, or evaluates tests. Apply it alongside any more specific framework or infrastructure testing skill.
- Use `.agents/skills/readme-maintenance/SKILL.md` for every README audit or change. Also use it to assess README impact whenever public behavior, configuration, setup, commands, architecture, deployment, CI, or supported versions change. A README edit is required only when the audit finds a stale or missing reader-facing claim.

## Repository and toolchain

- Rust 1.96.1, edition 2024, Axum 0.8, and Tokio are the current baseline. Keep `Cargo.toml`, `rust-toolchain.toml`, the container builder, workflows, and documentation aligned when the supported Rust version changes.
- Prefer current official Rust, Axum, Tokio, Tower, and crate documentation over older examples, especially pre-Axum-0.8 or pre-edition-2024 patterns.
- Use the root `Justfile` for normal workflows. `just qa` applies formatting before lint, build, and tests; use `just check` for the non-mutating repository gate and `just ci` for the broader CI-equivalent checks.
- Use a focused `cargo test --locked --test <target>` only when narrowing a test failure and the dependency graph must remain unchanged.
- Keep `Cargo.lock` checked in and never edit it manually. Use `cargo update` or `just lock` only for intentional dependency-resolution changes. If unrelated lockfile churn appears, stop instead of hand-editing it away.
- Avoid new dependencies when the standard library or an existing crate fits. Prefer actively maintained, well-documented crates and justify additions against a concrete feature need.

## Application architecture

- Keep `src/main.rs` limited to configuration, telemetry, state creation, listener binding, serving, and shutdown.
- Keep reusable application construction in `src/lib.rs` and `src/app.rs`. Tests and alternate composition use `build_app(...)` or `build_app_with_routes(...)` rather than rebuilding the middleware stack.
- App-wide dependencies live in `AppState` behind `Arc<AppState>` and are passed through Axum state. Keep handlers stateless when they do not need services or configuration.
- Put versioned route modules under `src/http/v1/`, export `pub fn router() -> Router<Arc<AppState>>`, merge them in `src/http/v1/mod.rs`, and add their documented handlers to `ApiDoc` in `src/http/v1/docs.rs`.
- Keep `/health` and `/api-docs` at the root. Keep business routes and `/v1/openapi` under `/v1`.
- Keep cross-cutting middleware centralized in `src/app.rs`. Preserve the request ID, panic recovery, request logging, security, CORS, timeout, and 1 MiB body-limit boundaries unless the task explicitly changes their contract.
- Preserve graceful shutdown through `tokio::select!` over Ctrl-C and Unix SIGTERM. The service must remain compatible with Cloud Run: bind `0.0.0.0:$PORT`, rely on platform TLS termination, and assume no persistent local filesystem.

## HTTP and API contracts

- Keep handlers focused on transport behavior. Put external API and persistence behavior behind the services in `src/services/`.
- Decode JSON or CBOR request bodies with `decode_request_body(...)`. Use `success_response(...)`, `success_response_with_headers(...)`, and `no_content_response(...)` for responses instead of hand-rolling serialization and shared headers.
- Use `problem_response(...)` for controlled failures. Keep client details stable and safe; never expose internal errors, stack traces, credentials, or raw upstream bodies.
- Treat malformed syntax and cursor encoding as 400, validation failures as 422, missing or invalid authentication as 401, conflicts as 409, and controlled upstream failures according to the existing service mapping.
- Keep public JSON fields camelCase, using Serde renames when Rust naming differs. Keep Firestore storage naming separate from the HTTP contract.
- Document every public handler with `#[utoipa::path(...)]`. Keep external paths, tags, request bodies, statuses, headers, and implemented JSON/CBOR media types aligned with runtime behavior and the OpenAPI document.
- Preserve JSON as the default success representation and explicit `application/cbor` as the negotiated alternative on versioned endpoints. JSON wins ties and wildcards; exact exclusions and RFC 9110 specificity remain authoritative. Unsupported modeled success representations return 406 before endpoint work. `/health` remains JSON-only, and bodyless 204 responses ignore `Accept`.
- Accept request bodies only as owned `application/json` or exact `application/cbor`; do not claim arbitrary `+cbor` media types. Return Problem Details with 415 for unsupported media types, 400 for malformed JSON/CBOR, and 413 for the shared 1 MiB limit. A CBOR request must contain exactly one data item.
- Keep JSON problems as `application/problem+json`. Encode the same Problem Details data model as generic `application/cbor` only when CBOR is explicitly preferred. Do not restore the unregistered `application/problem+cbor` type or claim `application/concise-problem-details+cbor` without implementing its different model.
- Preserve `Vary: Origin, Accept` on application-owned responses and include contract headers such as `Location` and `Link` through shared helpers.
- Respect a valid incoming `X-Request-Id`, generate a UUIDv4 fallback when it is absent or invalid, and keep request correlation behavior centralized in middleware.

## Pagination

- Reuse `src/pagination/cursor.rs`, `src/pagination/link.rs`, and `paginate(...)`; do not create endpoint-local cursor formats or link builders.
- Cursors are unpadded Base64URL encodings of the endpoint kind and value. Validate malformed, wrong-kind, and stale cursors before slicing data or calling an upstream service.
- Limits must remain positive and within the endpoint maximum. Preserve endpoint filters and the effective limit in generated links.
- Emit RFC 8288 `Link` relations only when a next or previous page exists. Test first, middle, terminal, invalid, wrong-kind, and filter-preservation behavior relevant to the changed endpoint.

## Authentication, persistence, security, and logging

- Protected profile routes use the verified `AuthenticatedUser` extractor. Do not parse authorization headers in handlers or accept a client-selected profile owner.
- Preserve production Firebase verification: RS256 Google keys, issuer and audience checks, Identity Platform lookup, and disabled or revoked-user handling. Missing or malformed revocation metadata is an authentication dependency failure, never evidence that a token is valid. Preserve the explicit emulator path when `FIREBASE_AUTH_EMULATOR_HOST` is configured.
- Use mock services for deterministic tests and the Firestore-backed profile service only when persistence semantics are under test. Preserve create-if-absent, ownership, audit, and timestamp behavior in the service layer. Derive Firestore profile document IDs with the shared prefixed Base64URL helper; Firebase UIDs are opaque and may contain Firestore path delimiters.
- Keep runtime configuration in environment variables and `AppConfig`. Never commit or log credentials, tokens, service-account paths, authorization values, profile data, or other PII. Prefer Application Default Credentials and workload identity in deployed environments.
- Runtime state construction must always use real services. Tests compose doubles through `AppState::with_services(...)`; environment labels must not activate mocks. Firebase emulator hosts are local-only, loopback-only, and must remain rejected in production environments.
- Keep request logs and trace correlation in the existing tracing middleware. Add domain logs only when they provide information beyond the access record, and keep diagnostic fields non-sensitive.
- Treat upstream transport errors and payloads as untrusted. Preserve useful internal error chains while returning controlled public details.

## Tests and validation

- Apply `adversarial-testing` for risk analysis and `rust-testing` for repository mechanics whenever tests are in scope.
- Use in-process router tests with `tower::ServiceExt::oneshot(...)`; do not bind network ports for ordinary tests.
- Reuse `tests/common/mod.rs` for application state and JSON, CBOR, or text body decoding. Keep unit tests next to pure helpers and HTTP behavior in `tests/`.
- Assert observable contracts: exact status, media type, relevant headers, decoded body fields, service side effects, and forbidden calls or leaks. Do not test only that Axum registration succeeds or that a mock returned its configured value.
- Unit and ordinary integration tests must not contact live Firebase, Firestore, GitHub, or other network services. Emulator coverage stays conditional and isolated in `tests/firestore_emulator.rs`.
- Run the narrowest relevant target first. For code changes, broaden through `just fmt-check`, `just lint`, `just test`, and `just check` as appropriate. Run `just test-emulators` when Firestore emulator semantics change and `just docker-build` when the container or runtime assembly changes.

## Agent skills and custom agents

Portable repository skills live under `.agents/skills/` and each contains `SKILL.md` plus `agents/openai.yaml`. GitHub Copilot custom-agent profiles remain separately under `.github/agents/`.

| Skill | Use when |
|---|---|
| `adversarial-testing` | Planning, writing, reviewing, or debugging tests at any layer |
| `axum-endpoint` | Creating or changing Axum routes and their HTTP contracts |
| `pagination-endpoint` | Creating or changing cursor-paginated list endpoints |
| `readme-maintenance` | Auditing human-facing README claims against the repository |
| `rust-testing` | Applying Rust, Axum, fixture, emulator, and command conventions for tests |

Keep skill frontmatter, directory names, UI metadata, paths, commands, and repository behavior aligned. Do not turn skills into generic persona prompts or duplicate large sections of this file.
