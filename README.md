# axum-playground

[![Build and tests](https://img.shields.io/github/actions/workflow/status/janisto/axum-playground/app-ci.yml?branch=main&label=build%20%26%20tests&logo=github)](https://github.com/janisto/axum-playground/actions/workflows/app-ci.yml)
[![Code quality](https://img.shields.io/github/actions/workflow/status/janisto/axum-playground/app-lint.yml?branch=main&label=code%20quality&logo=github)](https://github.com/janisto/axum-playground/actions/workflows/app-lint.yml)
[![Rust 1.97.1](https://img.shields.io/badge/Rust-1.97.1-000000?logo=rust&logoColor=white)](rust-toolchain.toml)
[![MIT license](https://img.shields.io/github/license/janisto/axum-playground)](LICENSE)

A public REST API example built with [Axum](https://github.com/tokio-rs/axum) and Tokio, demonstrating Firebase Authentication, Firestore CRUD operations, GitHub proxy endpoints, and a modern Rust development workflow using [Just](https://github.com/casey/just). It is intentionally not deployed yet; the repository is the example and validation target.

It showcases `axum-observability`-based structured request logging, RFC 9457 Problem Details for errors, JSON/CBOR content negotiation, and a modular route layout that is ready to grow into a larger service.

<img src="assets/ferris.svg" alt="Rust Ferris mascot illustration" width="400">

<sub>Ferris illustration from [free-ferris-pack](https://github.com/MariaLetta/free-ferris-pack/) by Maria Letta</sub>

### Features

- Layered middleware architecture with security headers, panic recovery, explicit HEAD handling, operation-scoped body limits, and [`axum-observability` v2.0.0](https://crates.io/crates/axum-observability/2.0.0) request correlation and terminal access logging
- Request-scoped W3C Trace Context Level 1 correlation via `traceparent`, plus the portable `X-Request-ID` grammar and a 32-character lowercase hexadecimal fallback
- GCP-shaped NDJSON logs on stdout with low-cardinality Axum route templates; concrete paths, query strings, peer IPs, and user agents are omitted
- RFC 9457 Problem Details for JSON errors and the same data model encoded as generic CBOR
- Strict JSON/CBOR negotiation on versioned responses, including `406 Not Acceptable` and exact media-range precedence
- Strict JSON/CBOR request decoding with negotiated Problem Details for malformed, unsupported, and oversized bodies
- Cursor-based pagination with RFC 8288 `Link` headers on items, GitHub repositories, activity, and tags
- OpenAPI 3.1 documentation at `/openapi.json`, including exact JSON/CBOR media types, controlled responses, headers, and Firebase bearer auth, with Swagger UI at `/api-docs`
- Firebase Authentication with production JWKS verification, disabled and revoked user checks, a 30-second authentication-operation deadline, and emulator-mode support
- Firestore-backed profile persistence with safe opaque-UID document keys, atomic lifecycle operations, a 30-second persistence-operation deadline, and an audit-first one-time migration for the retired profile shape
- Anonymous, credential-free GitHub transport with fixed API-version headers, manual same-origin redirects, strict projections, bounded bodies, and a single ten-second operation deadline
- Health check endpoint at `/health`

### API Design Principles

#### URI Design

- Use plural nouns for collections and resource groupings
- Avoid verbs in URIs when the HTTP method already expresses the action
- Keep the versioned API under `/v1` and reserve root-level routes for shared platform endpoints such as `/health`, `/openapi.json`, and `/api-docs`

#### HTTP Methods & Status Codes

| Method | Purpose | Success Status |
| --- | --- | --- |
| GET | Retrieve resource(s) | 200 OK |
| POST | Create a resource | 201 Created |
| PATCH | Partially update a resource | 200 OK |
| DELETE | Remove a resource | 204 No Content |

#### Error Responses

Errors use the RFC 9457 Problem Details data model and honor content negotiation:

- `application/problem+json` when JSON is requested or selected by default
- `application/cbor` when CBOR is selected by quality and specificity, including wildcard fallback after a more-specific JSON exclusion

`application/problem+cbor` is not a registered media type. The registered `application/concise-problem-details+cbor` type defines a different compact model and is not implemented here.

| Status | Use Case |
| --- | --- |
| 400 Bad Request | Malformed syntax, invalid cursor, cursor type mismatch |
| 401 Unauthorized | Missing or invalid authentication |
| 403 Forbidden | A controlled authorization policy rejects the request |
| 404 Not Found | Resource does not exist |
| 406 Not Acceptable | No supported success representation is acceptable |
| 409 Conflict | Profile already exists |
| 413 Content Too Large | A modeled request body exceeds exactly 1,000,000 bytes |
| 415 Unsupported Media Type | Request body is not owned JSON or CBOR |
| 422 Unprocessable Entity | Validation failures on well-formed input or invalid upstream cursor parameters |
| 429 Too Many Requests | GitHub reports quota exhaustion |
| 502 Bad Gateway | GitHub transport or response validation fails |
| 503 Service Unavailable | Authentication or persistence is temporarily unavailable |
| 504 Gateway Timeout | The complete GitHub operation exceeds ten seconds |

#### Content Negotiation

- JSON is the default and wins equal-quality ties.
- CBOR is selected when it outranks JSON. A wildcard alone ties the representations and JSON wins, but a wildcard can select CBOR when a more-specific range excludes JSON, such as `application/json;q=0, application/*;q=1`.
- Exact exclusions and media-range specificity follow RFC 9110. Unsupported success representations return 406 before endpoint work begins.
- Request bodies must declare exactly one `Content-Type` value of `application/json` or exact `application/cbor`. Vendor `+cbor` types are not treated as interchangeable, a CBOR body must contain exactly one data item, and `Content-Encoding` is limited to absent or a single `identity` value.
- Problems use `application/problem+json` by default and `application/cbor` under the same quality and specificity rules. Error negotiation is best effort so an existing error is not replaced by a second 406.
- Bodyless 204 responses ignore `Accept`.
- `/health` follows the same JSON-default representation negotiation contract

See [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110), [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949), [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457), and the [IANA media type registry](https://www.iana.org/assignments/media-types/media-types.xhtml) for the underlying contracts. Deterministic CBOR is intentionally not required because these payloads are transport representations, not signature or hash inputs.

#### Pagination

- Cursor-based tokens for stable pagination
- Links are emitted through the HTTP `Link` header per RFC 8288
- Items and all paginated GitHub collections use opaque cursor values rather than exposing storage or upstream page details

## Configuration

Copy `.env.example` to `.env` and customize as needed:

```bash
cp .env.example .env
```

`just` commands auto-load `.env` through the repo Justfile. If you run `cargo` directly instead of `just`, export the environment variables yourself.

### Environment Variables

| Variable | Description | Default |
| --- | --- | --- |
| `PORT` | Server listen port | `8080` |
| `FIREBASE_PROJECT_ID` | Firebase project ID and fallback Google project anchor | `demo-test-project` |
| `APP_ENVIRONMENT` | Runtime environment: `development`, `test`, or `production` | `development` |
| `GOOGLE_APPLICATION_CREDENTIALS` | Local ADC override path; leave unset on Cloud Run | - |
| `FIREBASE_AUTH_EMULATOR_HOST` | Firebase Auth emulator host without scheme | - |
| `FIRESTORE_EMULATOR_HOST` | Firestore emulator host without scheme | - |
| `GOOGLE_CLOUD_PROJECT` | Optional Google project fallback for Firestore | - |
| `GCP_PROJECT` | Optional Google project fallback for Firestore | - |
| `GCLOUD_PROJECT` | Optional Google project fallback for Firestore | - |
| `PROJECT_ID` | Optional Google project fallback for Firestore | - |

Notes:

- Emulator hosts must omit the protocol prefix and use loopback, for example `127.0.0.1:9099` and `127.0.0.1:8080`. Emulator configuration is rejected outside `development` and `test`.
- Unknown `APP_ENVIRONMENT` values fail startup instead of silently selecting development behavior.
- When `K_SERVICE` indicates Cloud Run, startup requires `APP_ENVIRONMENT=production` and an explicit `FIREBASE_PROJECT_ID`.
- On Cloud Run, use the attached service identity and leave `GOOGLE_APPLICATION_CREDENTIALS` unset.
- If the Google project fallback variables are unset, the app falls back to `FIREBASE_PROJECT_ID`.
- Runtime state always constructs real HTTP, authentication, and persistence services. Tests compose explicit doubles; setting `APP_ENVIRONMENT=test` does not activate mock services.
- The application routes never read or attach `GITHUB_TOKEN` or another ambient GitHub credential. Their caller-selected resources are fetched anonymously and are restricted to public projections.
- The credentials path is redacted from `AppConfig` debug output.
- Outbound GitHub requests pin [`X-GitHub-Api-Version: 2026-03-10`](https://docs.github.com/en/rest/about-the-rest-api/api-versions). This is an application contract rather than an environment setting; upgrading it requires reviewing GitHub payload schemas and the deterministic service tests together.

## Local Development

### Requirements

- Rust 1.97.1 via `rust-toolchain.toml`
- [Just](https://github.com/casey/just) 1.57.0
- [actionlint](https://github.com/rhysd/actionlint) 1.7.12 and [zizmor](https://github.com/zizmorcore/zizmor) 1.28.0 for local workflow checks
- [Firebase CLI](https://firebase.google.com/docs/cli) and Java 21 when running emulator-backed tests
- Podman or Docker for local image builds

Install the workflow linters through Homebrew, as in `axum-observability`:

```bash
brew install actionlint zizmor
```

The actionlint formula installs ShellCheck, which actionlint uses for embedded
shell scripts. This repository does not require pyflakes because its workflows
do not contain embedded Python scripts.

Run `just install` to fetch the locked dependency graph and install the current
Cargo maintenance and QA tools: cargo-edit, cargo-nextest, cargo-llvm-cov,
cargo-deny, cargo-audit, cargo-mutants, cargo-sort, and cargo-machete. Cargo
selects each tool's latest release and uses that release's packaged lockfile.

`just update` is intentionally comprehensive for this application. It upgrades
all direct dependency requirements to their latest releases, including
SemVer-incompatible releases, refreshes all transitive dependencies in
`Cargo.lock`, and fetches the resulting locked graph. Review the resulting
manifest and source changes, then run `just qa` before committing them.

### Quick Start

Choose either Application Default Credentials or the local Firebase emulators before starting. For local exploration of the public unauthenticated routes, the Auth emulator configuration keeps startup local:

```bash
FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:9099 just run
```

Setting `FIREBASE_AUTH_EMULATOR_HOST` selects the loopback-only emulator verifier; the server validates those tokens locally and does not connect to the Auth emulator. Run the Auth emulator when you need it to issue test tokens. To exercise profile persistence, start the Firestore emulator as described below.

Then visit:

- `http://localhost:8080/health` for the health probe
- `http://localhost:8080/api-docs` for Swagger UI
- `http://localhost:8080/openapi.json` for the OpenAPI document

Sample request:

```bash
curl -s localhost:8080/health
```

### Project Layout

```text
src/
	app.rs            # Root router and middleware composition
	auth/             # Firebase auth extraction and verification
	config.rs         # Environment-backed application configuration
	error.rs          # Startup and application error types
	http/
		health.rs       # Root health endpoint
		codec.rs        # Shared body decoding and response encoding
		extract.rs      # Problem Details-aware path and query extractors
		negotiation.rs  # RFC 9110 JSON/CBOR selection
		v1/             # Versioned API routes and docs wiring
	middleware/       # Application-owned recovery and security middleware
	pagination/       # Cursor and RFC 8288 link helpers
	profile_migration.rs # Audit-first retired-profile migration
	problem/          # Problem Details model and response construction
	services/         # GitHub and profile service implementations
	shutdown.rs       # Graceful shutdown coordination
	state.rs          # Shared application state
	telemetry.rs      # Tracing subscriber initialization
	lib.rs            # Reusable app construction
	main.rs           # Thin startup entrypoint
	bin/migrate_profiles.rs # Explicit one-time migration command
tests/              # In-process integration tests
.agents/skills/     # Five portable coding-agent workflows with Codex metadata
.github/agents/     # GitHub Copilot custom-agent profiles
.github/workflows/  # GitHub Actions automation
functions/          # Placeholder directory for future Firebase functions
```

Portable repository skills follow the [Agent Skills specification](https://agentskills.io/specification) under
`.agents/skills/`. See [AGENTS.md](AGENTS.md) for coding-agent execution rules and the current skill catalog.

### Routes

| Method | Path | Description |
| --- | --- | --- |
| GET | `/health` | Health check route |
| GET | `/api-docs` | Swagger UI |
| GET | `/openapi.json` | OpenAPI document |
| GET | `/v1/hello` | Default greeting |
| POST | `/v1/hello` | Create a personalized greeting |
| GET | `/v1/items` | List items with cursor-based pagination |
| GET | `/v1/profile` | Get current user profile, requires auth |
| POST | `/v1/profile` | Create user profile, requires auth |
| PATCH | `/v1/profile` | Update user profile, requires auth |
| DELETE | `/v1/profile` | Delete user profile, requires auth |
| GET | `/v1/github/owners/{owner}` | Get GitHub owner details |
| GET | `/v1/github/owners/{owner}/repos` | List repositories for an owner with cursor pagination |
| GET | `/v1/github/repos/{owner}/{repo}` | Get GitHub repository details |
| GET | `/v1/github/repos/{owner}/{repo}/activity` | List repository activity with cursor pagination |
| GET | `/v1/github/repos/{owner}/{repo}/languages` | Get repository language totals |
| GET | `/v1/github/repos/{owner}/{repo}/tags` | List repository tags with cursor pagination |

### Development

#### Justfile Commands

| Command | Description |
| --- | --- |
| `just build` | Build the application |
| `just run` | Run the server |
| `just install` | Fetch locked dependencies and install current Cargo maintenance and QA tools |
| `just install-tools` | Install current Cargo maintenance and QA tools |
| `just download` | Fetch locked Cargo dependencies |
| `just update` | Upgrade all direct requirements and refresh all locked transitive dependencies |
| `just fmt` | Apply formatting |
| `just fmt-check` | Verify formatting |
| `just lint` | Run clippy with warnings denied |
| `just doc` | Build documentation with all rustdoc lints denied |
| `just sort-check` | Verify grouped manifest ordering |
| `just unused-dependencies` | Detect unused direct dependencies |
| `just workflow-check` | Validate workflows with actionlint and zizmor |
| `just qa` | Run the non-mutating local quality gate |
| `just test` | Run the main test suite with `cargo nextest` |
| `just test-doc` | Run doctests |
| `just test-emulators` | Run the Firestore emulator test when configured |
| `just test-emulators-ci` | Start the Firestore emulator, run its required integration test, and stop it |
| `just mutations` | Run the explicit mutation-testing campaign |
| `just check` | Run `just qa` plus optional emulator coverage |
| `just ci` | Run `just qa` plus a container build |
| `just coverage-lcov` | Generate `coverage.lcov` |
| `just coverage-html` | Generate HTML coverage output |
| `just deny` | Run dependency policy checks |
| `just audit` | Run dependency vulnerability checks |
| `just docker-build` | Build the development image with Podman first, then Docker |
| `just lock` | Regenerate `Cargo.lock` |

Run `just --list` to see all available recipes.

#### Mutation Testing

Mutation testing is an explicit contributor campaign outside `just qa`. Run the
full local campaign with four workers:

```bash
just mutations --jobs 4
```

While strengthening tests, reuse previously caught and unviable results:

```bash
just mutations --jobs 4 --iterate
```

`--iterate` is a development optimization, not a final gate. After the tests
stabilize, rerun the full command without `--iterate`. Treat every unexplained
miss and timeout as unresolved; add a behavioral test for a real contract gap
rather than an artificial assertion for an equivalent transformation.

Mutation campaigns can take a long time. Do not start a competing run while
`cargo-mutants` holds the output lock. Results under `mutants.out` are ignored
and rotated by cargo-mutants. Firestore persistence remains an emulator-backed
boundary, so run `just test-emulators` with `FIRESTORE_EMULATOR_HOST` configured
when profile storage behavior changes. Precise default-campaign exclusions are
documented in `.cargo/mutants.toml`.

#### Firebase Emulators

The Firebase CLI can manage the Firestore emulator lifecycle and run the
integration test in one command:

```bash
just test-emulators-ci
```

To reuse an emulator that is already running, export its host instead:

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:8080
just test-emulators
```

The emulator test is explicitly ignored by the normal suite, and
`just test-emulators` skips cleanly when `FIRESTORE_EMULATOR_HOST` is unset.
GitHub Actions runs `just test-emulators-ci` as a required, isolated job using
the demo project ID, so it cannot contact a live Firestore project.

#### One-time Profile Migration

The accepted profile contract retires the persisted `firstname`, `lastname`,
`email`, `marketing`, and `terms` fields in favor of `firstName`, `lastName`,
`contactEmail`, `marketingOptIn`, and `termsAccepted`. The runtime does not
dual-read or dual-write the retired shape.

The repository-owned command is read-only unless `--apply` and an exact project
confirmation are both present. Audit is the default:

```bash
APP_ENVIRONMENT=production \
FIREBASE_PROJECT_ID=my-project \
cargo run --locked --bin migrate_profiles -- --audit
```

It classifies only the exact retired shape, the exact current shape, or a
blocked record. It validates document ownership and every canonical value,
normalizes legacy contact fields, converts legacy clock timestamps to UTC
milliseconds, and reports record identifiers only as SHA-256 fingerprints.
Any mixed, unknown, wrongly typed, invalid, or document-ID-mismatched record
blocks the complete apply preflight before the first write.

Applying requires the same configured project ID to be repeated literally:

```bash
APP_ENVIRONMENT=production \
FIREBASE_PROJECT_ID=my-project \
cargo run --locked --bin migrate_profiles -- \
  --apply --confirm-project my-project
```

Each target is re-read in a Firestore transaction and fully replaced with the
canonical object. A changed or newly invalid target stops the run; records
already migrated are recognized as current, so the command is safe to rerun.
A final audit must find only current records.

This migration and the new runtime form one atomic operational cutover. Build
the new revision without serving it, stop old profile writers and traffic,
take the required Firestore backup or export, run and resolve the audit, apply
the migration, activate the new revision, and verify profile reads before
reopening traffic. Rolling back the application alone is unsafe after apply;
restore the pre-migration data together with the retired runtime. None of these
production steps is performed by repository QA.

## Future Deployment

No environment has been deployed or validated from this repository. The following files document the intended Cloud Run path for future use; they are not evidence of production readiness.

### Container

```bash
just docker-build
```

`just docker-build` prefers Podman when it is installed and falls back to Docker otherwise.

Example local run with Podman:

```bash
podman run --rm -p 8080:8080 --env-file .env axum-playground:dev
```

Use `docker run` with the same flags if Docker is your local runtime.

### Google Cloud Run

```bash
gcloud builds submit --config cloudbuild.yaml \
	--substitutions _REGION=europe-west4,_AR_REPOSITORY=app-images,_IMAGE_NAME=axum-playground,_SERVICE=axum-playground,_DEPLOY=false
```

```bash
gcloud builds submit --config cloudbuild.yaml \
	--substitutions _REGION=europe-west4,_AR_REPOSITORY=app-images,_IMAGE_NAME=axum-playground,_SERVICE=axum-playground,_DEPLOY=true
```

The committed `cloudbuild.yaml` builds and explicitly pushes both immutable `${BUILD_ID}` and `latest` tags. When `_DEPLOY=true`, it deploys the `${BUILD_ID}` image after the push and configures `APP_ENVIRONMENT=production` plus `FIREBASE_PROJECT_ID=${PROJECT_ID}`.

Production runtime expectations:

- The service listens on `0.0.0.0:$PORT` and defaults to `8080` locally
- Cloud Run terminates TLS before forwarding HTTP traffic to the container
- Production credentials should come from the attached service identity rather than a local key file
- Configure any required CORS policy at the environment boundary; the application does not enable wildcard CORS
- Put Cloud Run or another front proxy in front of the Axum server for connection and platform deadlines; the application owns 30-second authentication- and persistence-operation deadlines and the contract's ten-second GitHub operation deadline rather than a global request timeout

## QA Surface

### Automated Checks

- In-process route behavior and JSON/CBOR negotiation across the public API
- Problem details, request ID behavior, and 404 or 405 fallback behavior
- Firebase auth parsing, revocation semantics, and local-only emulator guardrails
- Firestore-backed profile CRUD and retired-shape migration through required hosted emulator coverage and an equivalent local command
- Exact generated OpenAPI inventory, local-reference resolution, schemas, media types, statuses, security, and headers
- GitHub transport redirects, content metadata, body bounds, deadlines, quota hints, projection guards, and provider-link translation through deterministic doubles
- Dependency policy and vulnerability checks through `just deny` and `just audit`
- Axum-aligned Clippy policy, canonical Rust formatting, strict rustdoc, manifest ordering, and unused-dependency checks
- Local GitHub Actions workflow validation through the documented actionlint and zizmor versions
- Coverage export through `just coverage-lcov` and `just coverage-html`
- Container image buildability through `just docker-build` and the hosted `app-ci.yml` container job

### CI/CD

GitHub Actions workflows in `.github/workflows/`:

| Workflow | Description |
| --- | --- |
| `app-ci.yml` | Build, tests, doctests, required Firestore emulator coverage, production container build, and coverage artifact generation |
| `app-lint.yml` | Formatting, manifest ordering, clippy, rustdoc, unused dependencies, dependency policy, and security audit |
| `workflow-security.yml` | Hosted zizmor workflow security analysis |
| `labeler.yml` | Automatic pull request labeling |
| `labeler-manual.yml` | Manual backfill labeling for historical pull requests |
| `dependabot-auto-merge.yml` | Auto-merge Dependabot minor and patch updates |

Dependabot configuration lives in `.github/dependabot.yml`, and label rules live in `.github/labeler.yml`.

## License

MIT. See `LICENSE`.
