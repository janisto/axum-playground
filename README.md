# axum-playground

[![Build and tests](https://img.shields.io/github/actions/workflow/status/janisto/axum-playground/app-ci.yml?branch=main&label=build%20%26%20tests&logo=github)](https://github.com/janisto/axum-playground/actions/workflows/app-ci.yml)
[![Code quality](https://img.shields.io/github/actions/workflow/status/janisto/axum-playground/app-lint.yml?branch=main&label=code%20quality&logo=github)](https://github.com/janisto/axum-playground/actions/workflows/app-lint.yml)
[![Rust 1.96.1](https://img.shields.io/badge/Rust-1.96.1-000000?logo=rust&logoColor=white)](rust-toolchain.toml)
[![MIT license](https://img.shields.io/github/license/janisto/axum-playground)](LICENSE)

A public REST API example built with [Axum](https://github.com/tokio-rs/axum) and Tokio, demonstrating Firebase Authentication, Firestore CRUD operations, GitHub proxy endpoints, and a modern Rust development workflow using [Just](https://github.com/casey/just). It is intentionally not deployed yet; the repository is the example and validation target.

It showcases tracing-based request logging, RFC 9457 Problem Details for errors, JSON/CBOR content negotiation, and a modular route layout that is ready to grow into a larger service.

<img src="assets/ferris.svg" alt="Rust Ferris mascot illustration" width="400">

<sub>Ferris illustration from [free-ferris-pack](https://github.com/MariaLetta/free-ferris-pack/) by Maria Letta</sub>

### Features

- Layered middleware architecture with security headers, CORS, request IDs, panic recovery, timeouts, and tracing-based access logging
- Request-scoped trace correlation via `traceparent`, falling back to the request ID when no Google trace context is present
- RFC 9457 Problem Details for JSON errors and the same data model encoded as generic CBOR
- Strict JSON/CBOR negotiation on versioned responses, including `406 Not Acceptable` and exact media-range precedence
- Strict JSON/CBOR request decoding with negotiated Problem Details for malformed, unsupported, and oversized bodies
- Cursor-based pagination with RFC 8288 `Link` headers on items and GitHub activity endpoints
- OpenAPI 3.1 documentation at `/v1/openapi`, including JSON/CBOR request and response media types plus bearer auth, with Swagger UI at `/api-docs`
- A resolvable standalone Problem Details JSON Schema at `/schemas/ErrorModel.json`, advertised through RFC 8288 `describedBy` links
- Firebase Authentication with production JWKS verification, disabled and revoked user checks, and emulator-mode support
- Firestore-backed profile persistence with safe opaque-UID document keys, normalization, and audit logging
- Health check endpoint at `/health`

### API Design Principles

#### URI Design

- Use plural nouns for collections and resource groupings
- Avoid verbs in URIs when the HTTP method already expresses the action
- Keep the versioned API under `/v1` and reserve root-level routes for shared platform endpoints such as `/health` and `/api-docs`

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
- `application/cbor` when CBOR is explicitly preferred

`application/problem+cbor` is not a registered media type. The registered `application/concise-problem-details+cbor` type defines a different compact model and is not implemented here.

| Status | Use Case |
| --- | --- |
| 400 Bad Request | Malformed syntax, invalid cursor, cursor type mismatch |
| 401 Unauthorized | Missing or invalid authentication |
| 403 Forbidden | Authenticated but not authorized, or upstream access denied |
| 404 Not Found | Resource does not exist |
| 406 Not Acceptable | No supported success representation is acceptable |
| 409 Conflict | Profile already exists |
| 413 Content Too Large | Request body exceeds the 1 MiB limit |
| 415 Unsupported Media Type | Request body is not owned JSON or CBOR |
| 422 Unprocessable Entity | Validation failures on well-formed input |
| 502 Bad Gateway | Upstream dependency failure |

#### Content Negotiation

- JSON is the default and wins equal-quality ties.
- CBOR is selected only by an explicit positive-quality `application/cbor` media range; wildcards do not silently opt clients into binary responses.
- Exact exclusions and media-range specificity follow RFC 9110. Unsupported success representations return 406 before endpoint work begins.
- Request bodies must declare `application/json` or exact `application/cbor`. Vendor `+cbor` types are not treated as interchangeable, and a CBOR body must contain exactly one data item.
- Problems use `application/problem+json` by default and `application/cbor` when CBOR is explicitly preferred. Error negotiation is best effort so an existing error is not replaced by a second 406.
- Bodyless 204 responses ignore `Accept`.
- `/health` remains JSON-only

See [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110), [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949), [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457), and the [IANA media type registry](https://www.iana.org/assignments/media-types/media-types.xhtml) for the underlying contracts. Deterministic CBOR is intentionally not required because these payloads are transport representations, not signature or hash inputs.

#### Pagination

- Cursor-based tokens for stable pagination
- Links are emitted through the HTTP `Link` header per RFC 8288
- Items and GitHub activity both use opaque cursor values rather than exposing storage details

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
| `APP_ENVIRONMENT` | Environment label used by tracing and local-emulator guardrails | `development` |
| `GITHUB_TOKEN` | Optional token for higher-rate GitHub API access | - |
| `GOOGLE_APPLICATION_CREDENTIALS` | Local ADC override path; leave unset on Cloud Run | - |
| `FIREBASE_AUTH_EMULATOR_HOST` | Firebase Auth emulator host without scheme | - |
| `FIRESTORE_EMULATOR_HOST` | Firestore emulator host without scheme | - |
| `GOOGLE_CLOUD_PROJECT` | Optional Google project fallback for trace correlation | - |
| `GCP_PROJECT` | Optional Google project fallback for trace correlation | - |
| `GCLOUD_PROJECT` | Optional Google project fallback for trace correlation | - |
| `PROJECT_ID` | Optional Google project fallback for trace correlation | - |

Notes:

- Emulator hosts must omit the protocol prefix and use loopback, for example `127.0.0.1:9099` and `127.0.0.1:8080`. Emulator configuration is rejected outside `development` and `test`.
- On Cloud Run, use the attached service identity and leave `GOOGLE_APPLICATION_CREDENTIALS` unset.
- If the Google project fallback variables are unset, the app falls back to `FIREBASE_PROJECT_ID`.
- Runtime state always constructs real HTTP, authentication, and persistence services. Tests compose explicit doubles; setting `APP_ENVIRONMENT=test` does not activate mock services.
- `GITHUB_TOKEN` and the credentials path are redacted from `AppConfig` debug output.

## Local Development

### Requirements

- Rust 1.96.1 via `rust-toolchain.toml`
- [Just](https://github.com/casey/just)
- `cargo-nextest`
- `cargo-llvm-cov`
- `cargo-deny`
- `cargo-audit`
- [Firebase CLI](https://firebase.google.com/docs/cli) when running emulator-backed tests
- Podman or Docker for local image builds

### Quick Start

Choose either Application Default Credentials or the local Firebase emulators before starting. For local exploration of the public unauthenticated routes, the Auth emulator configuration keeps startup local:

```bash
FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:9099 just run
```

Setting `FIREBASE_AUTH_EMULATOR_HOST` selects the loopback-only emulator verifier; the server validates those tokens locally and does not connect to the Auth emulator. Run the Auth emulator when you need it to issue test tokens. To exercise profile persistence, start the Firestore emulator as described below.

Then visit:

- `http://localhost:8080/health` for the health probe
- `http://localhost:8080/api-docs` for Swagger UI
- `http://localhost:8080/v1/openapi` for the OpenAPI document

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
		schema.rs       # Standalone Problem Details JSON Schema
		v1/             # Versioned API routes and docs wiring
	middleware/       # Cross-cutting HTTP middleware
	pagination/       # Cursor and RFC 8288 link helpers
	problem/          # Problem Details model and response construction
	services/         # GitHub and profile service implementations
	shutdown.rs       # Graceful shutdown coordination
	state.rs          # Shared application state
	telemetry.rs      # Tracing subscriber initialization
	lib.rs            # Reusable app construction
	main.rs           # Thin startup entrypoint
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
| GET | `/v1/openapi` | OpenAPI document |
| GET | `/schemas/ErrorModel.json` | Problem Details JSON Schema |
| GET | `/v1/hello` | Default greeting |
| POST | `/v1/hello` | Create a personalized greeting |
| GET | `/v1/items` | List items with cursor-based pagination |
| GET | `/v1/profile` | Get current user profile, requires auth |
| POST | `/v1/profile` | Create user profile, requires auth |
| PATCH | `/v1/profile` | Update user profile, requires auth |
| DELETE | `/v1/profile` | Delete user profile, requires auth |
| GET | `/v1/github/owners/{owner}` | Get GitHub owner details |
| GET | `/v1/github/owners/{owner}/repos` | List repositories for an owner |
| GET | `/v1/github/repos/{owner}/{repo}` | Get GitHub repository details |
| GET | `/v1/github/repos/{owner}/{repo}/activity` | List repository activity with cursor pagination |
| GET | `/v1/github/repos/{owner}/{repo}/languages` | Get repository language totals |
| GET | `/v1/github/repos/{owner}/{repo}/tags` | List repository tags |

### Development

#### Justfile Commands

| Command | Description |
| --- | --- |
| `just build` | Build the application |
| `just run` | Run the server |
| `just install` | Alias for `just download` |
| `just download` | Fetch locked Cargo dependencies |
| `just update` | Update dependencies recorded in `Cargo.lock` within `Cargo.toml` constraints |
| `just fmt` | Apply formatting |
| `just fmt-check` | Verify formatting |
| `just lint` | Run clippy with warnings denied |
| `just qa` | Run format, lint, build, and tests |
| `just test` | Run the main test suite with `cargo nextest` |
| `just test-doc` | Run doctests |
| `just test-emulators` | Run the Firestore emulator test when configured |
| `just check` | Run format, lint, tests, and optional emulator coverage |
| `just ci` | Run the CI-oriented local verification bundle |
| `just coverage-lcov` | Generate `coverage.lcov` |
| `just coverage-html` | Generate HTML coverage output |
| `just deny` | Run dependency policy checks |
| `just audit` | Run dependency vulnerability checks |
| `just docker-build` | Build the development image with Podman first, then Docker |
| `just lock` | Regenerate `Cargo.lock` |

Run `just --list` to see all available recipes.

#### Firebase Emulators

Firestore integration tests require the Firestore emulator. Start it before running emulator-backed tests and export its host:

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:8080
just test-emulators
```

The emulator test skips cleanly when `FIRESTORE_EMULATOR_HOST` is unset. GitHub Actions does not currently start Firebase emulators, so emulator-backed Firestore coverage is an explicit local gate rather than part of the hosted CI claim.

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

The committed `cloudbuild.yaml` is configured to build and push both `${SHORT_SHA}` and `latest` tags. When `_DEPLOY=true`, it is intended to deploy the `${SHORT_SHA}` image to Cloud Run in the configured region.

Production runtime expectations:

- The service listens on `0.0.0.0:$PORT` and defaults to `8080` locally
- Cloud Run terminates TLS before forwarding HTTP traffic to the container
- Production credentials should come from the attached service identity rather than a local key file
- Configure a restrictive CORS policy before public deployment; the current wildcard, non-credentialed policy is for local example access
- Put Cloud Run or another front proxy in front of the Axum server to enforce connection-level limits in addition to the application request timeout

## QA Surface

### Automated Checks

- In-process route behavior and JSON/CBOR negotiation across the public API
- Problem details, request ID behavior, and 404 or 405 fallback behavior
- Firebase auth parsing, revocation semantics, and local-only emulator guardrails
- Firestore-backed profile CRUD via an optional local emulator integration test
- Dependency policy and vulnerability checks through `just deny` and `just audit`
- Coverage export through `just coverage-lcov` and `just coverage-html`
- Container image buildability through `just docker-build`

### CI/CD

GitHub Actions workflows in `.github/workflows/`:

| Workflow | Description |
| --- | --- |
| `app-ci.yml` | Build, tests, doctests, and coverage artifact generation |
| `app-lint.yml` | Formatting, clippy, dependency policy, and security audit |
| `labeler.yml` | Automatic pull request labeling |
| `labeler-manual.yml` | Manual backfill labeling for historical pull requests |
| `dependabot-auto-merge.yml` | Auto-merge Dependabot minor and patch updates |

Dependabot configuration lives in `.github/dependabot.yml`, and label rules live in `.github/labeler.yml`.

## License

MIT. See `LICENSE`.
