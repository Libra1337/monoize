# Public Provider Status Specification

## 0. Scope

PST-0.1. This specification defines physical-upstream status events, durable persistence,
Replica shipment, completeness tracking, public aggregation, and `GET /api/public/status`.

PST-0.2. `provider-pricing.spec.md` defines the Provider generation and embedded Channel
identity used by this specification. `public-site.spec.md` defines shared public API limits,
cache validators, and security headers.

## 1. Event and source-state storage

PST-D1. `upstream_call_events` MUST contain:

```text
id TEXT PRIMARY KEY
group_id TEXT NOT NULL
provider_id TEXT NOT NULL
channel_id TEXT NOT NULL
outcome TEXT NOT NULL
failure_class TEXT NULL
upstream_status INTEGER NULL
occurred_at_unix_ms BIGINT NOT NULL
source_node_id TEXT NOT NULL
provider_generation BIGINT NOT NULL
```

PST-D2. `outcome` MUST be `success` or `failure`. A failure row MUST use only
`rate_limited`, `transient`, or `persistent` as `failure_class`.

PST-D3. An event MUST NOT store a user ID, API-key ID, request or response body, prompt,
model content, URL, error text, or another customer value.

PST-D4. The table MUST NOT have a foreign key to mutable Provider, Channel, or Group rows.
It MUST have indexes over `(provider_id, occurred_at_unix_ms)` and
`(occurred_at_unix_ms)`.

PST-D5. Cleanup MUST delete rows older than 48 hours in batches of at most 1,000. It MUST
run after primary migrations and every 15 minutes. A cleanup failure MUST be logged and
retried at the next interval without stopping forwarding.

PST-D6. `status_source_state` MUST contain one row per `source_node_id` with
`last_seen_at_unix_ms`, `ship_interval_ms`, `pending_event_count`,
`oldest_pending_event_unix_ms`, `retired_at_unix_ms`, `clock_synchronized`,
`clock_good_heartbeat_streak`, `incomplete_since_unix_ms`, and
`incomplete_until_unix_ms`.

## 2. Event identity and counting boundary

PST-E1. One request lifecycle MUST receive an internal UUID v4 before its first physical
upstream dispatch. It MUST NOT derive from a client request ID.

PST-E2. The lifecycle MUST assign a zero-based monotonically increasing dispatch index
immediately before each physical upstream HTTP request or WebSocket connection, including
same-Channel retry and Provider fail-forward attempts.

PST-E3. Event ID MUST be the UTF-8 string
`{source_node_id}.{lowercase-hyphenated-lifecycle-uuid}.{base-10-dispatch-index}`.
Primary uses `primary`. Replica uses its stable metering UUID.

PST-E4. Persistence retry and Replica replay MUST reuse the same event ID. Database insert
MUST ignore an ID conflict. Replay MUST NOT increase counts.

PST-E5. Each completed physical dispatch MUST be evaluated exactly once. Event time is the
source node's Unix milliseconds when the dispatch reaches its terminal outcome.

PST-E6. A usable terminal upstream success creates a success event. An in-stream terminal
upstream error creates a failure event when the shared passive-health classifier assigns
one of PST-D2's failure classes.

PST-E7. A client disconnect followed by a billable upstream success creates a success
event. Every same-Channel retry is a separate event.

PST-E8. Credential, quota, model availability, network, timeout, rate-limit, and upstream
server failures create events when the shared classifier assigns a PST-D2 class.

PST-E9. An ordinary upstream HTTP `400`, `409`, or `422` without a listed structured
failure signal MUST NOT create an event.

PST-E10. A local authentication, authorization, balance, validation, transform, encoding,
pricing, or internal failure before dispatch MUST NOT create an event. Active probes and
dashboard connectivity tests MUST NOT create events.

PST-E11. Event-persistence failure MUST NOT change the forwarding response or billing
result.

## 3. Global dispatch bound

PST-C1. `MONOIZE_STATUS_EVENT_MAX_IN_FLIGHT_DISPATCHES` MUST be a positive integer and
default to `1024`.

PST-C2. One process-wide fair semaphore with PST-C1 permits MUST cover every physical
upstream HTTP and WebSocket dispatch. A path MUST acquire one permit before capacity
reservation and dispatch, and hold it until terminal outcome and reservation publication
or release.

PST-C3. Initial attempts, retries, and fail-forward attempts MUST use the same semaphore.
No dispatch path may bypass it.

PST-C4. Waiting MUST preserve the existing request deadline and cancellation behavior and
MUST NOT create a status-specific error.

PST-C5. Deployment preflight MUST prove PST-C1 is at least the node's approved maximum
simultaneous physical dispatch count. Its default is not an approval.

## 4. Durable event spool configuration

PST-S1. Configuration is process-local:

| Variable | Default | Constraint |
| --- | --- | --- |
| `MONOIZE_STATUS_EVENT_SPOOL_DIR` | `./data/status-event-spool` | resolved directory |
| `MONOIZE_STATUS_EVENT_SPOOL_MAX_BYTES` | `536870912` | positive allocated-byte quota |
| `MONOIZE_STATUS_EVENT_SPOOL_ENTRY_MAX_BYTES` | `4096` | at least `1024` |
| `MONOIZE_STATUS_EVENT_MAX_OUTAGE_SECONDS` | `900` | at least `900` |
| `MONOIZE_STATUS_EVENT_SPOOL_SAFETY_FACTOR_MILLI` | `1200` | at least `1200` |
| `MONOIZE_STATUS_EVENT_PEAK_EVENTS_PER_SECOND` | none | required positive integer |

PST-S2. The spool MUST use one JSON file per event. Publication MUST perform a same-directory
temporary write, file sync, atomic rename, and directory sync. Final filenames use event IDs.

PST-S3. An encoded event longer than the entry maximum MUST be rejected without truncation
or splitting and MUST set the incomplete-data latch.

PST-S4. The process MUST resolve the spool filesystem allocation unit.
`entry_reservation_bytes` is the logical entry maximum rounded up to that unit. A synced,
non-sparse probe file of the logical maximum MUST report an allocated size no greater than
the reservation. A logical file length MUST NOT substitute for allocated size.

PST-S5. For one node, compute with checked unsigned arithmetic:

```text
outage_event_slots = ceil(
    peak_events_per_second
    * max_outage_seconds
    * safety_factor_milli
    / 1000
)
in_flight_event_slots = max_in_flight_dispatches
minimum_spool_event_slots = outage_event_slots + in_flight_event_slots
minimum_spool_bytes = minimum_spool_event_slots * entry_reservation_bytes
```

PST-S6. `ceil(x / 1000)` MUST use checked `(x + 999) / 1000`. Parse failure, overflow,
unknown allocation unit, failed probe, or quota below `minimum_spool_bytes` MUST stop startup.

## 5. Spool accounting and startup admission

PST-Q1. The quota MUST charge final and temporary event files by OS-reported allocated
size. It MUST charge each outstanding dispatch reservation by `entry_reservation_bytes`.
An active event is charged once: while a temporary file exists, charge the greater of its
reservation and allocation; after rename, replace the reservation with final allocation.

PST-Q2. Bounded final and temporary node-state files are outside the event quota but their
allocated blocks MUST be covered by the filesystem free-byte check.

PST-Q3. Symlinks and non-regular entries in the spool directory MUST cause startup failure.

PST-Q4. Before a physical dispatch, after semaphore acquisition, the process MUST reserve
one allocated entry and one file slot. Reservation failure does not block the dispatch. A
later counted outcome without a reservation MUST set the incomplete-data latch.

PST-Q5. An excluded outcome releases its reservation. Publication failure for a counted
event sets the latch. A reservation is released only after temporary cleanup succeeds or
remaining allocation transfers into settled accounting.

PST-Q6. Define `accounted_spool_bytes` as event-file allocations plus outstanding
reservations. At startup, `remaining_spool_bytes = quota - accounted_spool_bytes` MUST be
at least `minimum_spool_bytes` before readiness becomes healthy.

PST-Q7. Filesystem bytes available to the service account MUST be at least
`remaining_spool_bytes + 67108864` before startup admission.

PST-Q8. Define `remaining_quota_event_slots = floor(remaining_spool_bytes /
entry_reservation_bytes)`. Each applicable finite inode, file-record, or per-directory
entry limit MUST report at least `remaining_quota_event_slots + 1024` available slots.
An API that explicitly reports no finite limit makes only that limit inapplicable. Unknown,
unsupported, permission-denied, and ambiguous results are failures.

PST-Q9. An allocated-size, free-byte, or applicable file-slot query failure, or accounted
bytes above quota, MUST stop startup. A valid configuration with insufficient startup
admission capacity MUST start batch and Replica recovery only, keep readiness unhealthy,
and reject forwarding admission until every condition passes.

PST-Q10. Recovery-only mode MUST re-evaluate after every committed drain batch and at least
every two seconds. It MUST NOT delete or ignore a final event.

PST-Q11. After forwarding begins, outage reserve is working capacity. Reserve consumption
alone MUST NOT return the process to recovery-only mode. Every new reservation still
enforces byte and file-slot limits.

PST-Q12. At startup, a final event file is replayable. A temporary event file proves an
incomplete publication: set the latch, charge it during cleanup, delete it, and sync the
directory. Cleanup failure enters recovery-only mode.

PST-Q13. A final node-state file is authoritative over its temporary file. Delete and sync
the temporary file. If only a temporary state exists, set the latch before cleanup.

PST-Q14. Startup and deployment preflight MUST pass create, write, file-sync,
allocated-size read, rename, directory-sync, and delete probes and record every capacity
input and result for every Primary and Replica.

## 6. Batching and incomplete-data latch

PST-B1. The batcher MUST insert at most 100 events per transaction and flush at least every
two seconds. Publishing a final file MUST wake it.

PST-B2. A final file may be deleted only after the insert transaction commits. A failed or
ambiguous transaction retains the file unchanged for idempotent replay.

PST-B3. The node-local incomplete-data latch is set only when a required counted event
cannot be retained in a final spool file or committed table. A pending intact file is not loss.

PST-B4. The latch MUST persist first-loss and most-recent-loss times and remain incomplete
until at least 24 hours after the most recent loss and every spool drains.

PST-B5. A bounded JSON node-state file, at most 1,024 logical bytes, MUST store the latch
and `clean_shutdown`. It MUST use the PST-S2 atomic publication sequence.

PST-B6. Before accepting traffic, the process MUST atomically set `clean_shutdown = false`.
An existing false value at startup extends incomplete time to 24 hours after that startup.

PST-B7. Graceful shutdown MUST stop admission, drain physical dispatches and the status
spool, then atomically set `clean_shutdown = true`.

PST-B8. A newly created empty spool without state is a first start and is not loss. An
unwritable latch file keeps readiness unhealthy. Simultaneous event and latch persistence
failure MUST retain an in-memory latch and continuously visible error metric.

## 7. Replica shipment and source completeness

PST-R1. Status events are the fourth data class in the existing Replica metering pipeline.
Request logs, last-used updates, and balance deltas remain unchanged.

PST-R2. Replica shipment MUST retain event files until Primary HTTP `200`. Primary apply is
idempotent by event ID.

PST-R3. Every heartbeat MUST include source clock, ship interval, pending event count,
oldest pending event time, incomplete interval, and retirement flag. Primary persists the
state in the same transaction that applies a batch or heartbeat.

PST-R4. Primary local state MUST update at least once per configured ship interval.

PST-R5. Graceful Replica retirement may be sent only after its status spool drains and
pending count is zero. A later heartbeat clears retirement.

PST-R6. Primary MUST set `clock_synchronized = false` when source time differs from receipt
time by more than 30 seconds. Three consecutive in-bound heartbeats restore true. A false
transition extends source incompleteness for at least 24 hours after receipt.

PST-R7. An active source is local or a non-retired source seen in the preceding 24 hours.
`data_through_unix_ms` MUST equal
`now - max(30000, 3 * maximum_active_ship_interval_ms)`. With no active source, use 30 seconds.

PST-R8. Data is incomplete through `data_through` if any active source has pending data
whose oldest time is not later than `data_through`, misses heartbeats beyond PST-R7, has an
unsynchronized clock, or has `incomplete_until_unix_ms > data_through`.

PST-R9. A non-retired source with no heartbeat for 24 hours is outside the public 24-hour
window and MUST no longer affect completeness.

## 8. Public status response

PST-P1. `GET /api/public/status` MUST require no dashboard session and return exactly:

```text
generated_at: RFC3339 UTC
data_through: RFC3339 UTC
data_complete: boolean
groups: Array<{
  public_name: string,
  state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
  insufficient_provider_count: nonnegative integer,
  providers: Array<{
    public_name: string,
    state: operational | minor_degradation | major_degradation | unavailable | insufficient_data,
    success_rate_24h_basis_points: integer 0..10000 | null
  }>
}>
```

PST-P2. The response MUST NOT contain attempt counts, internal IDs or names, Channel fields,
failure classes, upstream statuses, source-node fields, or secrets.

PST-P3. Include only enabled Providers whose embedded Channel is enabled. Include only a
Group with at least one included Provider. Order Groups by current Group order and Providers
by Group-local priority, creation time, and ID.

PST-P4. Count an event only when Provider and Channel still exist and are enabled, its
Group and Channel IDs match current Provider values, and `provider_generation` equals the
current `configuration_generation`.

PST-P5. Current state uses the inclusive 30-minute window ending at `data_through`. Fewer
than 10 attempts is `insufficient_data`. Otherwise classify with integer cross-products:

- at least 95 percent: `operational`;
- at least 80 and below 95 percent: `minor_degradation`;
- at least 50 and below 80 percent: `major_degradation`;
- below 50 percent: `unavailable`.

PST-P6. The 24-hour success rate uses the inclusive window ending at `data_through`. Zero
attempts returns null. Otherwise return `floor(successes * 10000 / attempts)`.

PST-P7. Group state is the worst known Provider state in this order: unavailable, major,
minor, operational. Ignore insufficient Providers when one known state exists, and return
their count separately. A Group is insufficient only when every Provider is insufficient.

PST-P8. One immutable snapshot may be built at most once per 15-second UTC bucket. Every
aggregate and `data_through` uses that build time. `generated_at`, exact uncompressed bytes,
and ETag remain stable for the bucket. Browser SWR refresh is 30 seconds.

PST-P9. If PST-R8 is true, the response MUST set `data_complete = false`. The UI MUST show
a public data-quality warning and MUST NOT present affected percentages as complete.

PST-P10. A public Status source read or response serialization failure MUST return HTTP
`503` with code `status_source_invalid` and a fixed public message. The server MUST log the
underlying error. The response MUST NOT contain SQL, a table or constraint name, an
internal ID, or the underlying error text.

## 9. Qualification and release gate

PST-G1. Each intended node MUST have approved positive event-rate and simultaneous-dispatch
values based on 30 complete days. With shorter evidence, double the longest observed
five-minute event rate and simultaneous count. Aggregate-only evidence assigns the complete
aggregate to every node. No evidence requires explicit owner values.

PST-G2. Qualification event rate is
`max(100, 5 * sum(approved_node_peak_events_per_second))`. Multi-source allocation is
proportional to approved node peaks and sums exactly to the qualification rate.

PST-G3. A separate concurrency profile MUST hold HTTP and WebSocket dispatches up to the
configured semaphore, verify one additional dispatch waits without reservation, then
release and verify bounded progress.

PST-G4. Three separate 30-minute profiles are required: Primary database outage, five-source
Primary database outage, and five-source Replica shipment outage. Failure runs from minute
5 through 20. Input continues for ten minutes after restoration, then stops for drain.

PST-G5. During continuing-input recovery, committed throughput MUST exceed input by at
least 25 percent in every complete five-minute window. After input stops, backlog MUST drain
within 15 minutes.

PST-G6. Every generated counted ID MUST reconcile to one committed unique row, one durable
uncommitted final file, or an explicit qualification-only lost ledger entry with the latch.
Main profiles require zero lost entries and zero unaccounted IDs.

PST-G7. Each process memory increase MUST be at most 256 MiB and five-process aggregate
increase at most 768 MiB.

PST-G8. Qualification MUST inject process termination before rename, after rename, during
database transaction, after ambiguous commit, and during shipment, plus full quota,
unwritable directory, lost directory sync, database failure, clock skew, and four concurrent
duplicate Replica replays.

PST-G9. Gate D passes only when capacity preflight, concurrency bound, replay, crash,
fault-injection, recovery, memory, and ID-conservation requirements all pass for every
required topology.
