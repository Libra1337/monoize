# Standby Deployment Forwarding

SD-1. A deployment forwarding configuration consists of
`MONOIZE_DEPLOYMENT_PREVIOUS_URL` and `MONOIZE_DEPLOYMENT_CONTROL_TOKEN`. Both
variables MUST be absent, or both MUST be present. The URL MUST use HTTP, a
literal loopback IP address, and a port different from the candidate listener.
It MUST contain no credentials, query, fragment, or path other than `/`.
The token MUST contain at least 32 characters. Forwarding MUST require a Primary
with `MONOIZE_BOOT_STANDBY_LEASE=1`. Invalid configuration MUST fail startup.

SD-2. A configured candidate starts in `forwarding` mode. Every method mounted
by `build_store_mutation_router`, both payment callback methods, and every
internal Replica route MUST pass through the deployment gate before local
authentication, mutation validation, or body parsing. Other application routes
MUST retain their existing behavior. Deployment control routes MUST bypass this gate.

SD-3. In `forwarding` mode, a gated request MUST make exactly one attempt to the
configured previous origin. Preserve the original method, path, query, body
bytes, Authorization, Cookie, Origin, and end-to-end headers. Remove hop-by-hop
headers and headers named by Connection. Remove untrusted forwarding headers
and rebuild X-Forwarded-For from the canonical client IP established by the
candidate's trusted-proxy middleware. The previous origin MUST trust this
loopback hop. Reject a URI whose normalization changes its path or query.

SD-4. The forwarding client MUST disable redirects, retries, inherited proxies,
and idle connection pooling. It MUST impose no total response timeout. Forward
the previous response status, end-to-end headers, and body bytes. A transport
failure MUST return HTTP 502 without retrying. A request cancelled before
response headers MUST cancel its forwarding future. A returned body MUST retain
its forwarding admission until EOF, an upstream body error, or downstream drop.

SD-5. The deployment gate has `forwarding`, `paused`, and `local` modes. An
admitted forwarded operation owns a read lock until SD-4 completion. Pausing
MUST acquire the write lock, wait for all admitted operations, and set `paused`.
New gated requests MUST wait while paused. Acquisition of the local Store lease
MUST acquire the same write lock, set `local`, and wake waiting requests. Local
mode MUST never transition back to forwarding.

SD-6. Configured candidates mount `POST /internal/deployment/pause`,
`POST /internal/deployment/resume`, and `GET /internal/deployment/status`.
Each route MUST require `Authorization: Bearer <control token>` before mutation.
Invalid credentials MUST return HTTP 401. Status MUST return JSON with `mode`
and `lease_owned`, where ownership requires successful local lease validation.
Responses MUST have `Cache-Control: no-store` and MUST never expose the token.
Pause in local mode MUST return HTTP 409. Resume MUST return HTTP 409 after any
local lease has been acquired. Otherwise resume sets forwarding and wakes waiters.
An unconfigured application MUST not mount these routes.

SD-7. Candidate per-process request-log and last-used flushers and cache eviction
MUST continue while standby. UserStore session cleanup, request-log retention,
billing-plan grants and revenue daily settlement MUST start only after the local
Store lease is acquired. Each singleton tick and revenue retry MUST check the
local lease, shutdown flag, and handover flag before work; it MUST stop after any
check fails. Any Primary with `MONOIZE_BOOT_STANDBY_LEASE=1` MUST skip global
pending-log cleanup at startup, acquisition, and shutdown because either process
can own live pending rows. An ordinary Primary MUST perform startup pending-log
cleanup only after acquiring the lease. The existing Store
retention, reconciliation, and admission recovery tasks retain the same gate.
Each process MUST start these singleton duties at most once. A process without
a valid local lease MUST not perform global pending-log cleanup at shutdown.

SD-8. Every overlapping process MUST use a distinct durable request-log spool
directory. Starting a candidate MUST not recover, promote, remove, or flush
files from the previous process's live spool. The deployment procedure MUST
retain each directory for recovery until its owner has drained and its durable
records have been persisted.

SD-9. Before signaling the previous process, the supervisor MUST pause the
candidate gate and confirm zero previous accepted connections. If connections
remain, it MUST resume forwarding without signaling the previous process.
After a zero observation it MAY request lease handover. It MUST wait for
`mode=local` and `lease_owned=true` before declaring handover complete. This
protocol MUST NOT cancel existing inference responses on either process.
