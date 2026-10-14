# ADR-0029 — Worker task streams spend finite receive credit

Status: **Accepted and implemented; server/five-SDK integration verified, comparative measurement recorded with explicit limits** | Date: 2026-10-12 | Feature: 006 | Refines ADR-0015; preserves ADR-0002 and ADR-0023

## Context

NFR-006-PERF requires measurement of streaming and backpressure. Workers currently obtain tasks
with unary `PollTasks`; streaming blobs in protocol 1.3 does not satisfy this task-transport
requirement. All five official hosts already calculate their available execution slots and share
the same server claim, lease, heartbeat and reporting semantics.

A continuously producing server stream cannot infer application capacity from HTTP/2 flow control.
A client can keep reading while every handler slot is occupied, and a server's view of a lease can
expire while its disconnected handler still runs. Repeatedly filling a stream from the database's
in-flight count alone would therefore exceed the host's capacity. A new transport must also avoid
holding tenant request permits or database connections while it waits for work.

The existing gRPC admission layer releases its ticket when the service future returns response
headers. That bounds unary work, but does not bound a streaming response's lifetime. Authentication
also returns a principal snapshot: carrying it throughout a stream would miss membership or role
changes. These facts require explicit stream lifetime limits and fresh authentication at each claim.

## Decision

**1. Protocol 1.4 adds `StreamTasks(PollTasksRequest) returns (stream PollTasksResponse)`.**
The canonical schema remains `specs/001-platform-foundation/contracts/worker.proto`. Existing
message fields and methods retain their meanings. For this new method, `max_tasks` is positive
finite receive credit, at most 64, for the entire call. Each nonempty frame spends credit by the
number of leases it carries. The server may send several frames as work becomes available; it
cannot replenish the credit itself. A stream closes after spending the credit, exhausting its
aggregate 8 MiB serialized task-input budget, or a five-second receive window. A normal underfilled
window ends with an empty frame carrying a positive `retry_after_ms`. The detailed wire and failure
contract is [stream-completion.md](../../specs/006-sdk-multilanguage/stream-completion.md).

**2. Every lease uses the canonical claim path.** Streaming passes its remaining task and byte
budgets into the same application claim used by polling. It introduces no broker-based authority,
stream cursor, durable delivery acknowledgment or alternative reducer. The transition, journal,
ready work, timer/subscription updates and outbox still commit under the documented run transaction.
No database transaction, pooled connection or ordinary tenant request concurrency permit is held
while waiting for work or for the response consumer.

The current poll reads the session's in-flight count before claiming separate runs. Concurrent
calls can race that read. Completion of this slice must fence and recheck session admission in the
canonical transactional path shared by unary polling and streaming; a process-local stream lock
cannot establish a multi-replica limit. Registration replacement, binding revocation, draining and
activity quota lock ordering are part of that review and of the real PostgreSQL tests. This design
does not claim that the present implementation already provides this stronger admission property.

**3. The receive window is a bounded resource.** Acquire separate process, tenant and subject
stream permits after initial authentication; retain them until the response body ends or is dropped.
These permits limit idle streams independently of ordinary request permits and are shed without
queueing. Defaults are 32 per process, 8 per tenant and 2 per subject in a tenant. They are resource
bounds, not subscription entitlements across replicas. The call remains discovery work and cannot
consume the capacity reserved for heartbeat and result reporting. Each active claim also receives
the normal short-lived discovery admission protection. The response producer has at most one
queued frame, reserves its queue place before claiming, and owns a cancellation/deadline path.

**4. Authenticate every claim, and end at credential expiry.** Run the existing bearer pipeline,
including membership lookup, budgets and current role, before each claim attempt. Use the newly
resolved principal and release its request permit after the claim. Do not retain the initial
principal as authority. The worker session, identity binding, namespace, drain state and capability
minor remain checks of the canonical application/store path. The bearer token captured for a call
cannot be refreshed inside that call; the next receive window obtains the SDK token source again.
The stream ends at token expiry even when no work is available. A revocation committed before a
subsequent claim is observed before that claim; a claim already admitted may finish, subject to its
existing transactional fences. No zero-duration revocation guarantee is invented for a JWT provider
that itself has no immediate revocation mechanism.

**5. A disconnected stream leaves its committed leases to the existing recovery machinery.**
Lease commitment precedes delivery. Losing the response can strand a lease until it expires;
stream closure never proves that a handler did not start. Never release a committed lease merely
because a write, stream or client was cancelled. An uncommitted claim rolls back. A committed,
unreceived task is recovered by the lease reaper with a later generation; stale reporting remains
rejected. Critical external effects still require idempotency or reconciliation.

**6. Hosts spend local capacity and preserve partial delivery.** Each host has one receive call
at a time and requests `min(free local slots, 64)` credits. It dispatches and starts heartbeating
each received lease immediately, without waiting for the stream to end. A task keeps its slot until
the handler/report lifecycle releases it, including after transport failure or re-registration.
Frames received before an error are not discarded or restarted. The next call recomputes free
slots. Empty/underfilled windows and transport failures follow bounded wait/backoff rules, and host
shutdown cancels an outstanding receive call while preserving the existing drain behavior.

Hosts use streaming when the accepted minor is at least 1.4 and unary polling for accepted 1.0–1.3.
An explicit `poll` transport option remains available for diagnostics and comparative benchmarks.
An `UNIMPLEMENTED` response permits fallback to polling for that registration, with a bounded
diagnostic, including when some earlier frames were already received. Other refusals retain their
canonical behavior and are not hidden by transport fallback. Neither transport re-registers merely
because a normal finite receive window ended.

**7. Evidence must distinguish transport savings from database work.** Both transports still
discover database work at the configured idle retry cadence. A task stream can reduce idle network
calls without reducing claim queries. Report both, together with delivery latency, throughput,
memory/backlog bounds, partial-delivery recovery and heartbeat progress under stream saturation.
The same streaming conformance scenarios must pass through the public hosts of all five SDKs.
No result is claimed by acceptance of this ADR.

Feature 001 contract/authentication changes and feature 002 admission changes are approved
dependencies of this feature-006 completion slice. Existing 009 and 010 security/admission
contracts are reused; their guarantees must not be bypassed or silently changed.

## Consequences

- Finite receive credit bounds how much a stalled or reconnecting host can be offered. Reopening a
  receive window has a small RPC cost, and long idle periods use fewer network calls than polling.
- A five-second window permits frequent token refresh and simple cancellation; it is not an
  end-to-end latency target. Database detection cadence and current contention still affect latency.
- There is no new persistent stream state. PostgreSQL remains sufficient to recover every committed
  lease, at the price of consuming an attempt if a lease response is lost.
- Stream limits and claim admission are separately testable. Ordinary request permits are available
  while streams are idle, and the process bounds long-lived response bodies explicitly.
- Shared types avoid additional wire mappers, while new method semantics are recorded explicitly in
  the proto, SDK specification, conformance cases and architecture documentation.

## Alternatives considered

1. **One unary result wrapped in a stream.** Simpler, but gives no successive delivery as work arrives
   and would not establish useful task-stream behavior. The selected method can send multiple
   nonempty frames inside one finite window.
2. **An unbounded server stream refilled from database in-flight counts.** Rejected: database lease
   counts do not describe a disconnected handler's local execution capacity.
3. **A bidirectional stream with replenishable credits.** Suitable if measurement justifies the
   additional state machine. It adds credit ordering, reconnection and reader/writer coordination
   in five runtimes without changing the durable claim. Finite windows provide explicit credit now.
4. **NATS delivery directly grants work.** Rejected by the PostgreSQL authority and fencing contract.
5. **A process-local lock makes the worker limit exact.** Rejected: calls for one session may reach
   different engine replicas and may mix polling with streaming.

## Review criteria

Before marking G125 or NFR-006-PERF complete, execute the server security/fault matrix, all-five
shared conformance and the comparative benchmark in the completion contract. Review any change to
total credit, stream/body admission, claim transaction fences, partial-delivery semantics or minor
fallback against this ADR. Replenishable or indefinitely live streams require a new decision with
measurements, failure behavior and migration/compatibility evidence.


### Transport deadline enforcement

The body retains its stream permits until completion or drop. An independent connection watchdog
bounds an HTTP/2 peer that stops reading: hyper may wait for flow-control capacity without polling
the response body, so a body-local timer alone cannot reclaim it. The watchdog gives the body/token
deadline a 10 ms transport grace to deliver its terminal status, then closes the entire HTTP/2
connection if the body has not completed. No new claim is admitted during this grace. Normal
completion/drop disarms that body's watchdog, so a later stream can reuse the connection.

This deliberately cancels other RPCs multiplexed on the same stalled connection. Existing
heartbeat/result retries reconnect with their normal fencing and idempotency semantics; the
transport does not replay a handler or release its durable lease. tonic/h2 does not expose a
per-response reset through this adapter once flow control stops body polling. The real zero-window
HTTP/2 regression checks held caps, bounded cleanup and watchdog disarming on a healthy connection.
