# HTTP integration test instructions

The root `AGENTS.md` also applies. Use `adversarial-testing` and `rust-testing`.
Pure helper tests remain beside their modules in `src/`.

- Exercise the composed router with `tower::ServiceExt::oneshot(...)`; do not
  bind network ports for ordinary tests or recreate the middleware stack.
- Reuse `tests/common/mod.rs` for application state and JSON, CBOR, or text body
  decoding. Use deterministic service doubles for authentication, profiles, and
  GitHub; real Firestore behavior belongs in the isolated conditional emulator
  test.
- Assert exact status, media type, relevant headers, decoded body fields, service
  side effects, and forbidden calls or leaks. Registration-only checks or a mock
  returning its configured value do not establish application behavior.
- Follow the root validation and mutation-testing rules. Run the narrowest
  relevant target first; report skipped emulator validation explicitly.
