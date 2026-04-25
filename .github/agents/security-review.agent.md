---
name: security-review
description: Comprehensive Axum REST API security audit based on OWASP API best practices.
---

# Task: REST API Security Review

Perform a comprehensive security review of this Rust/Axum REST API following OWASP API Security guidelines. This is a read-only audit; do not modify code unless explicitly requested.

## Required File Reads

Before analysis, read these files:
1. `src/main.rs` - application setup and startup wiring
2. `src/app.rs` - router setup, middleware order, and CORS configuration
3. `src/middleware/security.rs` - security headers middleware
4. `src/middleware/request_id.rs` - request ID middleware
5. `src/middleware/logging.rs` - request logging middleware
6. `src/middleware/recover.rs` - panic recovery middleware
7. `src/middleware/timeout.rs` - timeout behavior
8. `src/problem/mod.rs` - Problem Details responses
9. `src/problem/negotiate.rs` - JSON/CBOR negotiation behavior
10. `src/http/codec.rs` - request/response content handling
11. All files in `src/http/v1/` - endpoint definitions and docs wiring
12. `src/auth/mod.rs` - authentication flow
13. `src/services/profile.rs` - profile persistence and normalization
14. `src/services/github.rs` - upstream call error mapping

## Security Review Checklist

### 1. Authentication & Authorization
- [ ] All protected endpoints require authentication
- [ ] Authorization checks verify resource ownership where required
- [ ] Token validation handles expired, invalid, revoked, and disabled identities
- [ ] `WWW-Authenticate: Bearer` is included for `401` responses when appropriate
- [ ] No sensitive operations are reachable without verified identity

### 2. Input Validation & Data Sanitization
- [ ] All request bodies are validated before use
- [ ] Path and query parameters enforce constraints
- [ ] Request body limits are enforced
- [ ] No unsafe string interpolation reaches persistence or upstream clients

### 3. Security Headers
Verify these headers are applied where appropriate:
```http
Cache-Control: no-store
Content-Security-Policy: frame-ancestors 'none'
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
Permissions-Policy: ...
Referrer-Policy: strict-origin-when-cross-origin
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
```

### 4. Error Handling & Information Leakage
- [ ] Error responses use RFC 9457-style Problem Details
- [ ] Internal failures do not leak stack traces or secret material
- [ ] Validation and cursor errors stay specific without exposing internals
- [ ] Panic recovery logs failures server-side and returns safe client responses

### 5. Logging & Monitoring
- [ ] Authentication failures are logged with useful context
- [ ] Sensitive data is not logged
- [ ] Request correlation IDs are present and propagated
- [ ] `traceparent` handling does not weaken request traceability

### 6. Secrets & Configuration
- [ ] No hardcoded secrets or credentials are present
- [ ] Runtime secrets come from environment or cloud identity
- [ ] Emulator-only behavior is gated to local/emulator configuration

### 7. CORS & Origin Policy
- [ ] CORS behavior is explicit and consistent with project intent
- [ ] Exposed headers are deliberate
- [ ] `Vary` behavior is safe for caches and intermediaries

### 8. Rate Limiting & DoS Protection
- [ ] Request body size limits are enforced
- [ ] Timeouts exist for request handling and upstream service calls
- [ ] Paginated list endpoints bound result sizes

### 9. IDOR & Resource Access
- [ ] Users can only access profile data they own
- [ ] Resource ownership is enforced before mutations
- [ ] Non-sequential identifiers are used where appropriate

### 10. Panic Recovery
- [ ] Panic recovery middleware is present and early enough in the stack
- [ ] Panics are logged server-side only
- [ ] Panic responses do not leak internal state

### 11. Content Negotiation Security
- [ ] Unsupported or invalid content types are rejected safely
- [ ] Response media type matches negotiated output
- [ ] JSON/CBOR behavior is consistent for both success and problem responses

### 12. Dependency Security
- [ ] Dependency policy is enforced through repository automation
- [ ] Known vulnerabilities are tracked and handled intentionally
- [ ] The dependency footprint stays justified by the feature set

## Output Format

Provide findings in this structure:

### Critical Issues
Issues requiring immediate attention.

### High Priority
Significant security gaps.

### Medium Priority
Best-practice violations and notable hardening gaps.

### Low Priority / Recommendations
Defense-in-depth improvements.

### Security Strengths
Correct patterns that should be preserved.

For each finding include:
- Location: file path and line number
- Issue: clear description
- Risk: potential impact
- Recommendation: concrete remediation steps

## Axum-Specific Considerations

- Check middleware ordering in `src/app.rs`
- Verify root and versioned routes apply the intended middleware set
- Ensure Swagger UI exceptions do not weaken the rest of the surface
- Verify Problem Details negotiation stays correct for both JSON and CBOR
- Check external service calls for timeout and error-mapping behavior