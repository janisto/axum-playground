---
name: pagination-endpoint
description: Create or change axum-playground cursor-paginated endpoints using the shared Base64URL cursor, pagination, and RFC 8288 Link helpers, including validation, filters, upstream cursors, OpenAPI, and tests.
---

# Pagination endpoints

Read `AGENTS.md`, `src/pagination/`, the affected handler, its service boundary, and nearby pagination tests before editing.

## Choose the existing pagination path

- For an in-memory ordered collection, use `pagination::paginate(...)` as `src/http/v1/items.rs` does.
- For an upstream-owned page, keep the upstream cursor inside the repository cursor kind and build links with `pagination::link::build_link_header(...)` as the GitHub activity route does.
- Do not invent a second cursor encoding, query serializer, or link builder.

## Cursor and query contract

- Use a stable endpoint-specific cursor kind. `Cursor::encode()` produces unpadded Base64URL for `kind:value`; `decode_cursor("")` represents the first page.
- Reject malformed encodings and wrong cursor kinds with 400 before slicing data or calling the upstream service.
- Reject stale local cursors when the referenced item is not present in the filtered collection.
- Validate `limit` before conversion. Enforce a positive value and the endpoint maximum; use 422 for limit or filter validation failures.
- Preserve endpoint filters and the effective limit in generated links. Keep cursor values opaque to clients.

## Link behavior

- Emit RFC 8288 `rel="next"` and `rel="prev"` only when those pages exist.
- Use `success_response_with_headers(...)` to attach `Link` without bypassing content negotiation or `Vary` handling.
- Preserve the current upstream limitation when only a next cursor is available; do not synthesize a previous link without enough source information.
- Keep Utoipa query parameters, bounds, Link response metadata, and reachable 400 or 422 responses aligned with runtime behavior.

## Tests and validation

Apply `$adversarial-testing` and `$rust-testing`. Cover the mutations most likely to break paging: zero or negative limits, maximum boundaries, malformed and wrong-kind cursors, stale cursors, off-by-one page starts, first/middle/terminal links, preserved filters, and no service call after invalid input.

Run the focused test target first, then `just fmt-check`, `just lint`, and `just test`. Use `just check` when changing shared pagination helpers.
