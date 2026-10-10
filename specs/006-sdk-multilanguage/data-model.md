# Data Model: TypeScript, Go, Python, C# and Java SDKs

## Context

This feature adds **no table, no column and no migration**: an SDK keeps no journal and no durable state of its
own (`plan.md`: "do not make the SDK persist its own journal"). Everything below is either a **wire type** that the
platform already owns (REST `/v1`, `worker.proto` 1.x — reference schema in `specs/001-platform-foundation/`) or an
**in-process value** of an SDK, whose lifetime is a call or a worker host. The behaviour of all of it is
[`sdks/SPEC.md`](../../sdks/SPEC.md); the five SDKs and the conformance suite
(`tests/contract/sdk-conformance/`) are held to it by [ADR-0015](../../docs/adr/ADR-0015-sdks-meet-the-server-by-minor-version-and-are-held-to-one-conformance-suite.md).

## Entities

### Owned by the platform (the SDKs only carry them)

- **Run / Step / Attempt, JournalEvent, Lease, Signal, Event, Timer, SagaEntry/Compensation:** as in
  `specs/001-platform-foundation/data-model.md` and the features 002–004. An SDK reads them over REST, never writes
  them directly: a transition is the platform's, in its own transaction (principle I).
- **Worker session / binding:** one live session per worker name and identity; the binding of an identity to a
  namespace and its activities is created by an administrator (`NOT_BOUND` otherwise).
- **Workflow document:** immutable after publication; the builders of the five SDKs produce the same document and the
  same canonical bytes (RFC 8785) for the same workflow, so a digest taken in one language is valid in another.

### Owned by an SDK (in memory only)

| Value | Fields (all of them) | Lifetime and rule |
|---|---|---|
| **Config** | base URL, gRPC target, namespace, token or token provider, TLS options (CA, client certificate and key), timeouts, retry policy | Immutable once a client or host is built. A credential is held where no printing, logging, serialization or equality can reach it. `http`/`https` only, for REST and for gRPC (`http://` only for loopback or development; any other scheme is `VALIDATION_FAILED`); never a verification-off option; a redirect is never followed. |
| **Canonical error** | `code` (one of the 19 in `sdks/error-catalog.json`), `status`, `retryable`, `traceId` (only if 32 hex), `reason`, `retryAfterMs` (≤ 60 s), `remediation`, a cause | Never carries a payload, a token or a secret; the server's message is cut to 512 characters. The mapping from HTTP and gRPC is data (the catalog), not code. |
| **Call context** | traceparent, correlation id, idempotency key (generated once per call, kept across its retries), deadline | Created per call; the trace the caller already has becomes the run's trace. |
| **Retry state** | attempt number, next delay (jittered exponential, capped), a budget of 4 attempts for a call | A call is repeated only if it is a `GET`, or a `POST` that carries an idempotency key or an event id, **and** the error's code is `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL`. Never `QUOTA_EXCEEDED`; a 4xx claiming `retryable` is not believed. |
| **Worker host** | registration (`accepted_protocol_version`, worker id, session), in-flight tasks (≤ `max_inflight`), heartbeats, secrets of each attempt | Restartable. States below. |
| **Task in flight** | `TaskLease` (task, run, step, attempt, generation, lease token, expiry), the parsed step document, the cancellation token and its reason (`run-cancelled`, `lease-lost`, `shutdown`), its heartbeat | From receipt of the lease to the report (or its loss). The secrets are wiped when the handler returns. |
| **Step document** | `{run:{id}, node, input, steps:{<node>:{output}}}` | Read once per task; an integer beyond 2^53−1 in another step's output raises when the handler reads `steps`, never rounds. |

## State machine of a worker host

```text
Created ──start()──▶ Registering ──accepted──▶ Running ──stop(timeout)──▶ Stopping ──▶ Stopped
   ▲                      │                       │  ▲                                    │
   │                      └─ UNAUTHENTICATED ─────┘  └── UNAUTHENTICATED: register again ┤
   └──────────────────────────────── start() again (the host is restartable) ──────────────┘
```

A task in flight: `Leased → Running ⇄ (heartbeat) → Reporting → Done`, with three exits that report nothing —
`lease-lost` (stale heartbeat or report), `shutdown` (the host stopped and the handler raised its cancellation
error: the lease lapses and the platform leases the step again) and a dropped duplicate report.

## Invariants

- The SDK never decides a retry of a **step**: the platform does. It decides only whether to repeat a *call*, by the
  rules above.
- A report of one attempt is idempotent and never contradicts itself: `done-<task_id>` / `failed-<task_id>`, the same
  content on every retry.
- The host uses only the features of the protocol minor the server accepted (`1.0`: no step identity in the
  heartbeat and no secrets; below `1.2`: `task_kind` is not read).
- The host never runs more tasks than it asked for, never busy-polls, and cancels a handler's token without
  interrupting it. It does not fence a lease by its own clock (a recorded gap).
- A secret value, a token or a payload is in no log, error, metric label or printed form.
- External effects stay at-least-once: the SDK hands the activity an idempotency key and a reconciliation hook.
- The five SDKs produce identical results for the same scenario against the same platform, and the platform ends in
  the same state, read without any SDK.

## Migrations

None. The compatibility rule is the protocol's: a minor version is additive; the server keeps the forms of every minor
it accepts (protocol-negotiation scenario, `contract_grpc_006`).
