# axum-playground

A REST API skeleton built with [Axum](https://github.com/tokio-rs/axum) and Tokio, demonstrating Firebase Authentication, Firestore CRUD operations, GitHub proxy endpoints, and a modern Rust development workflow using [Just](https://github.com/casey/just).

It showcases tracing-based request logging, RFC 9457 Problem Details for errors, JSON/CBOR content negotiation, and a modular route layout that is ready to grow into a larger service.

<img src="assets/ferris.svg" alt="Rust Ferris mascot illustration" width="400">

<sub>Ferris illustration from [free-ferris-pack](https://github.com/MariaLetta/free-ferris-pack/) by Maria Letta</sub>

### Features

- Layered middleware architecture with security headers, CORS, request IDs, panic recovery, timeouts, and tracing-based access logging
- Request-scoped trace correlation via `traceparent`, falling back to the request ID when no Google trace context is present
- RFC 9457-style Problem Details for error responses with JSON and CBOR variants
- Content negotiation supporting JSON and CBOR on versioned success responses via the `Accept` header
- Cursor-based pagination with RFC 8288 `Link` headers on items and GitHub activity endpoints
- OpenAPI 3.1 documentation at `/v1/openapi` with Swagger UI at `/api-docs`
- Firebase Authentication with production JWKS verification, disabled and revoked user checks, and emulator-mode support
- Firestore-backed profile persistence with normalization and audit logging
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

Errors follow RFC 9457-style Problem Details and honor content negotiation:

- `application/problem+json` when JSON is requested or selected by default
- `application/problem+cbor` when CBOR is requested

| Status | Use Case |
| --- | --- |
| 400 Bad Request | Malformed syntax, invalid cursor, cursor type mismatch |
| 401 Unauthorized | Missing or invalid authentication |
| 403 Forbidden | Authenticated but not authorized, or upstream access denied |
| 404 Not Found | Resource does not exist |
| 409 Conflict | Profile already exists |
| 422 Unprocessable Entity | Validation failures on well-formed input |
| 502 Bad Gateway | Upstream dependency failure |

#### Content Negotiation

- Default: `application/json`
- Alternate: `application/cbor`
- Problem responses switch to `application/problem+json` or `application/problem+cbor`
- `/health` remains JSON-only

#### Pagination

- Cursor-based tokens for stable pagination
- Links are emitted through the HTTP `Link` header per RFC 8288
- Items and GitHub activity both use opaque cursor values rather than exposing storage details

## Configuration

Copy `.env.example` to `.env` and customize as needed:

```bash
cp .env.example .env
```

### Environment Variables

| Variable | Description | Default |
| --- | --- | --- |
| `PORT` | Server listen port | `8080` |
| `FIREBASE_PROJECT_ID` | Firebase project ID and fallback Google project anchor | `demo-test-project` |
| `APP_ENVIRONMENT` | Environment label used by tracing setup | `development` |
| `GITHUB_TOKEN` | Optional token for higher-rate GitHub API access | - |
| `GOOGLE_APPLICATION_CREDENTIALS` | Local ADC override path; leave unset on Cloud Run | - |
| `FIREBASE_AUTH_EMULATOR_HOST` | Firebase Auth emulator host without scheme | - |
| `FIRESTORE_EMULATOR_HOST` | Firestore emulator host without scheme | - |
| `GOOGLE_CLOUD_PROJECT` | Optional Google project fallback for trace correlation | - |
| `GCP_PROJECT` | Optional Google project fallback for trace correlation | - |
| `GCLOUD_PROJECT` | Optional Google project fallback for trace correlation | - |
| `PROJECT_ID` | Optional Google project fallback for trace correlation | - |

Notes:

- Emulator hosts must omit the protocol prefix, for example `127.0.0.1:9099` and `127.0.0.1:8080`.
- On Cloud Run, use the attached service identity and leave `GOOGLE_APPLICATION_CREDENTIALS` unset.
- If the Google project fallback variables are unset, the app falls back to `FIREBASE_PROJECT_ID`.

## Local Development

### Requirements

- Rust 1.95.x via `rust-toolchain.toml`
- [Just](https://github.com/casey/just)
- `cargo-nextest`
- `cargo-llvm-cov`
- `cargo-deny`
- `cargo-audit`
- [Firebase CLI](https://firebase.google.com/docs/cli) when running emulator-backed tests
- Podman or Docker for local image builds

### Quick Start

```bash
just run
```

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
	http/
		health.rs       # Root health endpoint
		v1/             # Versioned API routes and docs wiring
	middleware/       # Cross-cutting HTTP middleware
	pagination/       # Cursor and RFC 8288 link helpers
	problem/          # Problem Details responses and negotiation
	services/         # GitHub and profile service implementations
	state.rs          # Shared application state
	telemetry.rs      # Tracing subscriber initialization
	lib.rs            # Reusable app construction
	main.rs           # Thin startup entrypoint
tests/              # In-process integration tests
.github/            # Workflows, agents, and skills
functions/          # Placeholder directory for future Firebase functions
```

### Routes

| Method | Path | Description |
| --- | --- | --- |
| GET | `/health` | Health check route |
| GET | `/api-docs` | Swagger UI |
| GET | `/v1/openapi` | OpenAPI document |
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
| `just fmt` | Apply formatting |
| `just fmt-check` | Verify formatting |
| `just lint` | Run clippy with warnings denied |
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

Run `just --list` to see all available recipes.

#### Firebase Emulators

Firestore integration tests require the Firebase emulators. Start them before running emulator-backed tests and export both host variables:

```bash
export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:9099
export FIRESTORE_EMULATOR_HOST=127.0.0.1:8080
just test-emulators
```

The emulator test skips cleanly when `FIRESTORE_EMULATOR_HOST` is unset.

## Deployment

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

The committed `cloudbuild.yaml` builds and pushes both `${SHORT_SHA}` and `latest` tags. When `_DEPLOY=true`, it deploys the `${SHORT_SHA}` image to Cloud Run in the configured region.

Production runtime expectations:

- The service listens on `0.0.0.0:$PORT` and defaults to `8080` locally
- Cloud Run terminates TLS before forwarding HTTP traffic to the container
- Production credentials should come from the attached service identity rather than a local key file

## QA Surface

### Automated Checks

- In-process route behavior and JSON/CBOR negotiation across the public API
- Problem details, request ID behavior, and 404 or 405 fallback behavior
- Firebase auth parsing and emulator-mode flows
- Firestore-backed profile CRUD via the conditional emulator integration test
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
