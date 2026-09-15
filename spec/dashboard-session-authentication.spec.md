# Dashboard Session Authentication Specification

## 0A. Public-route exception

DSA-P1. `/`, `/login`, `/apidocs`, `/status`, and `/marketplace` require no
dashboard session. The browser MUST NOT redirect an unauthenticated visitor from any of
these paths to `/login`.

DSA-P2. The five `/api/public/**` endpoints in `public-site.spec.md` PS-A1 require no
dashboard session. They MUST ignore dashboard authentication state when deciding access.

DSA-P3. `/dashboard/marketplace` MUST inherit the protected `/dashboard` route guard.
It MUST apply DSA6 through DSA9. `/marketplace` MUST remain outside the guard.

## 0. Scope

This specification defines browser storage and transport of dashboard sessions.

## 1. Session cookie

DSA1. Successful login and registration MUST set `monoize_session` with attributes `HttpOnly`, `Secure`, `SameSite=Strict`, and `Path=/`.

DSA2. Dashboard browser requests MUST send cookies with `credentials: "include"`.

DSA3. The dashboard browser MUST NOT store the dashboard session token in `localStorage`, `sessionStorage`, or IndexedDB.

DSA4. The dashboard browser MUST NOT read the dashboard session token from browser storage.

DSA5. The dashboard browser MUST NOT add an `Authorization` header for dashboard session authentication. This rule applies to REST and SSE requests.

DSA6. The dashboard browser MUST determine its authenticated state by calling `GET /api/dashboard/auth/me` with the session cookie.

DSA7. Logout MUST invalidate the server session identified by the cookie and MUST expire the `monoize_session` cookie.

DSA8. If a dashboard endpoint requires authentication and the request contains neither a `monoize_session` cookie nor a Bearer session token, the backend MUST return HTTP `401` with code `unauthorized` and message `missing dashboard session`.

DSA9. A dashboard API response with HTTP `401` and error code `unauthorized` MUST invalidate the browser's authenticated state. The dashboard MUST clear cached authenticated data and navigate to `/login` instead of rendering the response error in the current page.

## 2. Non-browser clients

DSA10. The backend MAY accept `Authorization: Bearer <session-token>` for non-browser dashboard clients. This compatibility MUST NOT cause the dashboard browser to expose or persist the token.

## 3. Token storage

DSA11. `sessions.token` MUST hold the SHA-256 hex digest of the session token, and MUST NOT hold the token itself. The token is 128 bits from `Uuid::new_v4`, so a fast digest is sufficient; a password KDF MUST NOT be used, because it would add latency to every authenticated request without adding resistance to guessing.

DSA12. Session lookup and session deletion MUST hash the presented token and compare digests. A digest and a plaintext token MUST NOT be interchangeable: a row already holding a 64-character hex digest MUST NOT be re-hashed.

DSA13. `POST /api/dashboard/auth/login` MUST be throttled. A super-admin, admin, or operator MUST NOT be able to disable this throttle from the dashboard, because it is the control that limits password guessing.

DSA14. The throttle MUST count failed password verifications only, and MUST NOT count a successful login, a failed CAPTCHA, a disabled account, or a malformed request.

DSA15. The throttle MUST apply two independent limits over a 15-minute window:
- 5 failures for one `(username, source address)` pair. The username comparison MUST be case-insensitive, matching the case-insensitive account lookup.
- 25 failures for one source address across all usernames, so a single password sprayed over many accounts is bounded.

DSA16. A request that exceeds either limit MUST be refused with HTTP `429` and code `login_throttled` before CAPTCHA verification and before password verification. A request under both limits MUST be answered exactly as it is today.

DSA17. The throttle MUST be checked before CAPTCHA verification, so a throttled caller cannot spend CAPTCHA verifications.

DSA18. A successful login MUST clear both counters for that `(username, source address)` pair. When the source address is unavailable, the pair key MUST use a fixed placeholder so that the per-pair limit still applies.

DSA19. A resolved password change or any other session-invalidating mutation MUST NOT clear the throttle; only a successful login does.
