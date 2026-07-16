---
name: axum-endpoint
description: Create or change axum-playground Axum endpoints, including route registration, shared state and services, JSON or CBOR requests and responses, Problem Details, Utoipa OpenAPI metadata, authentication, and tests.
---

# Axum endpoints

Read `AGENTS.md`, the neighboring module under `src/http/v1/`, `src/http/v1/mod.rs`, `src/http/v1/docs.rs`, the relevant service or validation code, and nearby integration tests before editing a route.

## Design boundary

- Put versioned routes in `src/http/v1/<resource>.rs` and expose `pub fn router() -> Router<Arc<AppState>>`.
- Merge a new module in `src/http/v1/mod.rs`. Use `src/app.rs` only for intentionally root-level routes or application-wide middleware.
- Keep transport behavior in handlers and external API or persistence behavior behind focused services in `src/services/`.
- Extract `State<Arc<AppState>>` only when the handler needs shared dependencies. Use `AuthenticatedUser` for protected routes rather than reading authorization headers in the handler.

## Contract implementation

- Define request and response DTOs beside the route and use Serde plus `utoipa::ToSchema`. Keep public JSON names camelCase.
- Decode request bodies with `decode_request_body(...)`; do not add handler-local JSON or CBOR parsing.
- Return modeled bodies through `success_response(...)` or `success_response_with_headers(...)`. Use `no_content_response(...)` for 204 and shared helpers for `Location`, `Link`, and `Vary` behavior.
- Return controlled failures through `problem_response(...)` or the existing service-error mapping. Keep internal and upstream details out of client responses.
- Reuse validators in `src/validation.rs` before adding route-local validation. Treat malformed bodies as 400 and well-formed validation failures as 422.
- Document every public operation with `#[utoipa::path(...)]`, including its external `/v1/...` path, tag, request body, implemented JSON and CBOR response media, headers, and reachable error statuses.
- Add each documented handler import and path entry to `ApiDoc` in `src/http/v1/docs.rs`; route registration alone does not add it to the generated document.

## Tests and validation

Apply `$adversarial-testing` to rank failure modes, then `$rust-testing` for fixtures and commands. Add the smallest integration-test set that proves success, validation, authentication or authorization, service failures, negotiation, OpenAPI presence, and relevant headers. Assert forbidden service calls when negotiation, validation, or authentication should stop execution.

Run the focused test target first, then:

```bash
just fmt-check
just lint
just test
```

Run `just check` when shared routing, middleware, state, or contract behavior changes.
