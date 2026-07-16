---
name: readme-maintenance
description: Audit or update axum-playground README.md when Rust or Cargo configuration, routes, API contracts, commands, containers, environment and Firebase behavior, OpenAPI or Swagger UI, deployment, CI, or supported versions change.
---

# README maintenance

Read `AGENTS.md`, then verify every affected `README.md` claim against the current repository. Keep the README focused on software engineers and contributors; keep coding-agent execution rules and detailed implementation constraints in `AGENTS.md` or repository skills.

## Sources of truth

- runtime and versions: `Cargo.toml`, `Cargo.lock`, and `rust-toolchain.toml`;
- application and routes: `src/main.rs`, `src/app.rs`, `src/http/health.rs`, and `src/http/v1/`;
- API behavior: `src/http/codec.rs`, `src/problem/`, `src/pagination/`, `src/auth/`, `src/services/`, and integration tests;
- configuration and Firebase: `src/config.rs`, `.env.example`, and emulator tests;
- commands and containers: `Justfile`, `Dockerfile`, and `cloudbuild.yaml`;
- automation and supported versions: `.github/workflows/`, `.github/dependabot.yml`, and pinned action or image references.

## Accuracy rules

- Require every named route, path, recipe, environment variable, version, and default to exist.
- Keep the Rust pin aligned across the toolchain, manifest, workflows, container, and README.
- Describe `/health` as the root liveness endpoint, `/v1/openapi` as the OpenAPI document, and `/api-docs` as Swagger UI.
- State only the JSON and CBOR request or response behavior that the affected route implements. Reflect the current Problem Details media types without implying a standard the code does not implement.
- Describe Firebase production verification, emulator configuration, Firestore-backed behavior, and Application Default Credentials according to the current code; do not imply all runtime configuration fields are already wired to every client.
- Keep local development, emulator, container, Cloud Build, Cloud Run, CI, and coverage guidance aligned with committed commands and assets.
- Keep the project layout concise. Distinguish portable workflows under `.agents/skills/` from GitHub Copilot profiles under `.github/agents/` and point readers to `AGENTS.md` for execution rules.
- Do not claim deployment, production readiness, strict standards compliance, or security properties that have not been verified.
- Remove stale material instead of preserving an obsolete README structure. Do not add agent instructions, source tutorials, speculative features, or duplicated `AGENTS.md` sections.

## Verification

Dry-run named recipes where practical and inspect route registration, configuration, workflows, and container files directly. For a documentation-only change, validate paths, links, YAML where touched, and `git diff --check`; do not run mutating `just qa` solely for documentation.

Reread the complete README before finishing and check for contradictory routes, versions, commands, environment behavior, skill counts, and discovery paths.
