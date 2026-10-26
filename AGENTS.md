# AGENTS.md

Instructions for coding agents. Keep execution rules here; `README.md` serves
human users and contributors. Do not duplicate agent instructions into it.

## Engineering priorities

- Prioritize correctness, readability, maintainability, then performance.
- Inspect the implementation, callers, and tests before changing behavior.
- Prefer the smallest safe change; reuse or refactor local patterns and utilities
  before adding abstractions or dependencies.
- Validate review findings against requirements and reproducible behavior.
  Simplify unnecessary behavior instead of adding machinery to defend it.
- State the failure mode before architectural, security, persistence, or
  production-impacting changes.
- Report implementation, validation, and remaining risks before completion.
- Keep instructions, comments, and documentation concise. Explain and obtain
  explicit approval before removing, merging, or relocating guidance, including
  obsolete rules, completed-task narration, and duplicates.
- Do not add progress narration, generated banners, emojis, or speculative TODOs.
- Keep public code, instructions, comments, documentation, commits, and PR text
  self-contained. Do not disclose private coordination repositories, paths,
  revisions, or discussions.

## Epistemic honesty and scope discipline

- Report defects, corrupted data, unmet requirements, and unfinished work plainly.
  Never conceal them in justifications or relabel them as simplifications,
  provisional scope, or expected limitations. If a problem is deeper than
  expected, stop at a safe boundary and report the exact failure, affected
  behavior or records, and remaining work.
- Counts, checksums, and passing tests show consistency, not semantic correctness.
  Before claiming Serde parsing, validation, or Firestore transformations work,
  inspect accepted inputs and resulting responses or profiles using synthetic
  fixtures or authorized data. Hunt for false positives; report the worst counterexamples,
  or describe the sample when none were found.
- Fix the bug class behind cited failures: correct the grammar, validation, query,
  or logic and audit all affected paths or records within the authorized scope.
  Do not hardcode cited values into exceptions or blocklists, or patch only named
  IDs.
- Do not weaken specifications, `AGENTS.md`, or active plans to excuse defects or
  shortcuts. Follow current task scope and stop gates. For review-only or
  report-only work, edit only an explicitly requested report.
- After the final code, schema, or generator change, rebuild affected generated
  OpenAPI, fixture artifacts, and test data, then rerun relevant checks. Earlier passing results do not validate later edits.

## Commits and pull requests

- Use [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):
  `type: imperative description`. Add a scope only when it materially improves
  clarity. Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`,
  `ci`, `chore`, and `revert`.
- Use the same format for PR titles, suitable for the final merge commit. Keep PRs
  focused; describe the problem, resulting behavior, validation, and remaining
  risks.
- Mark breaking changes with `!` and a `BREAKING CHANGE:` footer explaining the
  impact.
- Work on task branches; never commit directly to the default branch.
- Before committing, always run `just qa` and `git diff --check`.
- Use one atomic PR when layers would require manual settings changes between
  merges. A stack must be mergeable as a whole.
- This repository has no `CHANGELOG.md`; do not create one or require entries.

## Skills and validation

### Required guidance

- Use `.agents/skills/edit-agent-instructions/SKILL.md` when reviewing or editing
  root or nested agent instructions.
- Use `.agents/skills/adversarial-testing/SKILL.md` whenever planning, creating,
  modifying, reviewing, debugging, or evaluating tests. Apply relevant language,
  framework, and infrastructure testing guidance alongside it.
- Use `.agents/skills/readme-maintenance/SKILL.md` for README audits and edits.
  Assess README impact when API behavior/contracts, persistence, configuration,
  setup, commands, architecture, tooling, deployment, CI, or supported versions
  change. Edit the README only for stale or missing reader-facing claims.

### Validation

- Apply `adversarial-testing` for risk analysis and `rust-testing` for repository
  mechanics whenever tests are in scope. Keep pure helper tests beside `src/`
  modules. Before HTTP integration work or behavior changes requiring HTTP
  coverage, read [tests/AGENTS.md](tests/AGENTS.md).
- Unit and ordinary integration tests must not contact live Firebase, Firestore,
  GitHub, or other network services. Emulator coverage remains conditional and
  isolated in `tests/firestore_emulator.rs`.
- Run the narrowest relevant target first, then `just fmt-check`, `just lint`,
  `just test`, and `just check` as appropriate. Run `just test-emulators` for
  Firestore emulator changes and `just docker-build` for container/runtime changes.

### Task guides

Portable repository skills live under `.agents/skills/` and each contains `SKILL.md` plus `agents/openai.yaml`. GitHub Copilot custom-agent profiles remain separately under `.github/agents/`.

| Skill | Use when |
|---|---|
| `adversarial-testing` | Planning, writing, reviewing, or debugging tests at any layer |
| `edit-agent-instructions` | Reviewing or editing root or nested agent instructions |
| `axum-endpoint` | Creating or changing Axum routes and their HTTP contracts |
| `pagination-endpoint` | Creating or changing cursor-paginated list endpoints |
| `readme-maintenance` | Auditing human-facing README claims against the repository |
| `rust-testing` | Applying Rust, Axum, fixture, emulator, and command conventions for tests |

Keep skill frontmatter, directory names, UI metadata, paths, commands, and repository behavior aligned. Do not turn skills into generic persona prompts or duplicate large sections of this file.

## GitHub automation

This example is not deployed; GCP is a possible future target. Keep manual
deployment examples project-agnostic and do not deploy on pushes, PRs, or merges.

- Keep required `ci` and independent pure `lint` checks. Preserve early lint
  feedback and fail CI when a mandatory validation result is not successful.
- Before workflow changes, use Chrome to inspect GitHub Settings > Rules >
  Rulesets, including required check names and GitHub App sources. Preserve
  existing enforcement until replacements reach trunk. Verify final rulesets
  and actual PR checks in Chrome; CLI/API evidence supplements this. If access
  is unavailable, report the blocker. Never weaken protection to make a PR pass.
- Match local and CI workflow scanners. Use one enforcing read-only offline
  pedantic zizmor scan in plain mode; do not add SARIF uploads or security-event
  write permissions. Keep scanners independent of language quality checks.
- Keep Dependabot security alerts and automated security updates disabled;
  retain scheduled version updates and native audits. Do not add replacement
  alert bots or repeat unchanged upstream triage. Keep auto-merge limited to
  verified Dependabot minor/patch updates behind required checks; do not add
  duplicate package filters, state queries, enablement triggers, or revocation
  machinery. Handle old PR cleanup once.
- Reference GitHub Actions by explicit full release tags such as
  `owner/action@v1.2.3`, not full commit SHAs or floating major-version tags.
  Dependabot updates those release tags.

## Repository and toolchain

- Use the Rust pin in `rust-toolchain.toml`, edition 2024, Axum 0.8, and Tokio. Keep `Cargo.toml`, `rust-toolchain.toml`, the container builder, workflows, and documentation aligned when the supported Rust version changes.
- Prefer current official Rust, Axum, Tokio, Tower, and crate documentation over older examples, especially pre-Axum-0.8 or pre-edition-2024 patterns.
- Use the root `Justfile` for normal workflows. `just fmt` applies formatting, `just qa` is the non-mutating repository gate, `just check` adds optional emulator coverage, and `just ci` adds a container build.
- Use a focused `cargo test --locked --test <target>` only when narrowing a test failure and the dependency graph must remain unchanged.
- Keep `Cargo.lock` checked in and never edit it manually. Use `cargo update` or `just lock` only for intentional dependency-resolution changes. If unrelated lockfile churn appears, stop instead of hand-editing it away.
- Keep compatible Cargo requirements in `Cargo.toml`, exact application resolution in `Cargo.lock`, and Dependabot in `lockfile-only` mode. Keep Cargo CLI tool installs aligned between the `Justfile` and workflows; use each tool release's packaged lockfile without hard-coding tool versions that Dependabot cannot maintain.
- Avoid new dependencies when the standard library or an existing crate fits. Prefer actively maintained, well-documented crates and justify additions against a concrete feature need.

## Application architecture

- Keep `src/main.rs` limited to configuration, telemetry, state creation, listener binding, serving, and shutdown.
- Keep reusable application construction in `src/lib.rs` and `src/app.rs`. Tests and alternate composition use `build_app(...)` or `build_app_with_routes(...)` rather than rebuilding the middleware stack.
- App-wide dependencies live in `AppState` behind `Arc<AppState>` and are passed through Axum state. Keep handlers stateless when they do not need services or configuration.
- Put versioned route modules under `src/http/v1/`, export `pub fn router() -> Router<Arc<AppState>>`, merge them in `src/http/v1/mod.rs`, and add their documented handlers to `ApiDoc` in `src/http/v1/docs.rs`.
- Keep `/health`, `/openapi.json`, and the optional `/api-docs` UI at the root. Keep business routes under `/v1`.
- Keep cross-cutting middleware centralized in `src/app.rs`. Preserve the `axum-observability` request correlation and terminal access logging layer, panic recovery, security headers, explicit HEAD handling, and operation-scoped body-limit boundaries unless the task explicitly changes their contract. CORS and a global application timeout are not enabled by default.
- Preserve graceful shutdown through `tokio::select!` over Ctrl-C and Unix SIGTERM. The service must remain compatible with Cloud Run: bind `0.0.0.0:$PORT`, rely on platform TLS termination, and assume no persistent local filesystem.

## HTTP and API contracts

- Keep handlers focused on transport behavior. Put external API and persistence behavior behind the services in `src/services/`.
- Decode JSON or CBOR request bodies with `decode_request_body(...)`. Use `success_response(...)`, `success_response_with_headers(...)`, and `no_content_response(...)` for responses instead of hand-rolling serialization and shared headers.
- Use `problem_response(...)` for controlled failures. Keep client details stable and safe; never expose internal errors, stack traces, credentials, or raw upstream bodies.
- Treat malformed syntax and cursor encoding as 400, validation failures as 422, missing or invalid authentication as 401, conflicts as 409, and controlled upstream failures according to the existing service mapping.
- Keep public JSON fields camelCase, using Serde renames when Rust naming differs. Keep Firestore storage naming separate from the HTTP contract.
- Document every public handler with `#[utoipa::path(...)]`. Keep external paths, tags, request bodies, statuses, headers, and implemented JSON/CBOR media types aligned with runtime behavior and the OpenAPI document.
- Preserve JSON as the default success representation and explicit `application/cbor` as the negotiated alternative on represented endpoints, including `/health`. JSON wins ties and wildcards; exact exclusions and RFC 9110 specificity remain authoritative. Unsupported modeled success representations return 406 before endpoint work. Bodyless 204 responses ignore `Accept`.
- Accept request bodies only with exactly one `Content-Type` value of owned `application/json` or exact `application/cbor`; do not claim arbitrary `+cbor` media types. Accept `Content-Encoding` only when absent or a single `identity` value. Return Problem Details with 415 for unsupported or ambiguous media metadata, 400 for malformed JSON/CBOR, and 413 for the exact 1,000,000-byte limit. A CBOR request must contain exactly one data item.
- Keep JSON problems as `application/problem+json`. Encode the same Problem Details data model as generic `application/cbor` only when CBOR is explicitly preferred. Do not restore the unregistered `application/problem+cbor` type or claim `application/concise-problem-details+cbor` without implementing its different model.
- Preserve `Vary: Accept` on represented application responses, add `Origin` only when the origin policy varies, and include contract headers such as `Location` and `Link` through shared helpers.
- Respect exactly one incoming `X-Request-ID` only when it matches the portable 1–128-byte grammar; generate an exactly 32-character lowercase hexadecimal fallback when it is absent or invalid, and keep request correlation behavior centralized in middleware.
- Keep outbound GitHub REST requests pinned to API version `2026-03-10` through `X-GitHub-Api-Version`. Do not rely on GitHub's default version. Treat any version change as an explicit upstream contract migration: verify the official schemas and live response shape, then update the production constant, deterministic request and payload tests, README, and this instruction together.

## Pagination

- Reuse `src/pagination/cursor.rs`, `src/pagination/link.rs`, and `paginate(...)`; do not create endpoint-local cursor formats or link builders.
- Cursors are unpadded Base64URL encodings of the endpoint kind and value. Validate malformed, wrong-kind, and stale cursors before slicing data or calling an upstream service.
- Limits must remain positive and within the endpoint maximum. Preserve endpoint filters and the effective limit in generated links.
- Emit RFC 8288 `Link` relations only when a next or previous page exists. Test first, middle, terminal, invalid, wrong-kind, and filter-preservation behavior relevant to the changed endpoint.

## Authentication, persistence, security, and logging

- Protected profile routes use the verified `AuthenticatedUser` extractor. Do not parse authorization headers in handlers or accept a client-selected profile owner.
- Preserve production Firebase verification: RS256 Google keys, issuer and audience checks, Identity Platform lookup, and disabled or revoked-user handling. This application is not tenant-aware, so reject tokens carrying `firebase.tenant` rather than looking the UID up in the root project. Missing or malformed revocation metadata is an authentication dependency failure, never evidence that a token is valid. Preserve the explicit emulator path when `FIREBASE_AUTH_EMULATOR_HOST` is configured.
- Use mock services for deterministic tests and the Firestore-backed profile service only when persistence semantics are under test. Preserve create-if-absent, ownership, audit, and timestamp behavior in the service layer. Derive Firestore profile document IDs with the shared prefixed Base64URL helper; Firebase UIDs are opaque and may contain Firestore path delimiters.
- Keep runtime configuration in environment variables and `AppConfig`. Never commit or log credentials, tokens, service-account paths, authorization values, profile data, or other PII. Prefer Application Default Credentials and workload identity in deployed environments.
- Runtime state construction must always use real services. Tests compose doubles through `AppState::with_services(...)`; environment labels must not activate mocks. Firebase emulator hosts are local-only, loopback-only, and must remain rejected in production environments. When `K_SERVICE` identifies Cloud Run, require `APP_ENVIRONMENT=production` and an explicit `FIREBASE_PROJECT_ID`.
- Keep request logs and trace correlation in `axum-observability`. Add domain logs only when they provide information beyond the access record, and keep diagnostic fields non-sensitive.
- Treat upstream transport errors and payloads as untrusted. Preserve useful internal error chains while returning controlled public details.

## Mutation testing

- Run mutation testing when changing production logic or strengthening focused tests. Add tests for meaningful surviving mutants, not equivalent transformations, and keep generated mutation artifacts untracked.
- Before starting a mutation run, check for active `cargo-mutants` and `just mutations` processes and inspect `mutants.out/lock.json`. The lock file persists after a completed run, so its presence alone does not mean the lock is held; never delete it to bypass a live OS lock or start a competing run. Mutation campaigns can legitimately take a long time, so allow an active healthy run to finish and report progress instead of interrupting it because of elapsed time alone.
- Preserve any mutation results still needed before starting another run: cargo-mutants rotates `mutants.out` to `mutants.out.old` and deletes the previous old output. Before using `--iterate`, verify that the current `mutants.out` is the intended source run and contains the expected `caught.txt`, `previously_caught.txt`, and `unviable.txt` state.
- Use four workers for local mutation testing. While iterating on tests, run `just mutations --jobs 4 --iterate` so cargo-mutants skips mutants already caught or found unviable. Treat `--iterate` as a development optimization only: after the implementation and tests stabilize, run `just mutations --jobs 4` without `--iterate` as the authoritative full pass.
- Treat every missed mutant and timeout as unresolved. Rerun timeouts with `--iterate`, inspect their logs, and adjust timeout or skip configuration only after establishing that the outcome is a tooling limit, intentional hang, or equivalent mutation. Do not report a mutation campaign as clean while unexplained misses or timeouts remain.
- Do not use `--baseline=skip` for local mutation testing. It is acceptable in CI only after the same checkout has passed the complete test suite; when skipping the baseline, set an explicit test timeout because cargo-mutants cannot derive one from baseline timing.
- Reserve `--in-place` for disposable single-worker CI checkouts or targeted troubleshooting where reusing the existing target directory is more important than local parallelism; cargo-mutants does not support combining `--in-place` with `--jobs`. Before an `--in-place` run, record the working-tree state and ensure no editor, formatter, test, or second mutation process can modify the checkout concurrently. After the run, inspect `git diff` and search for `/* ~ changed by cargo-mutants ~ */`; if the run was interrupted or failed to restore a file, preserve the intended edits and remove only the mutation before continuing.
