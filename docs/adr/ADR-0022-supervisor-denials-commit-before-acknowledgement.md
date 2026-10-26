# ADR-0022 — Supervisor denials commit before acknowledgement

Status: **Accepted** | Date: 2026-10-11 | Feature: 009 | Refines ADR-0013

## Context

ADR-0013 made the sandbox supervisor the only network path available to an untrusted script. It
also made denial evidence part of the terminal execution report. That timing leaves a durability
gap: the supervisor can tell the script that a request was denied, then the runner can crash before
the terminal report reaches the control plane. The network remains closed, but the acknowledged
security decision has no durable audit record.

The existing alternatives do not close the gap. Namespace denial recording is deliberately
asynchronous and lossy. Sending an early terminal execution record would freeze the first record for
an attempt and corrupt the later real result. Persisting URLs or hostnames as an idempotency key also
lets an untrusted script create unbounded audit keys and may store secrets embedded in a destination.

## Decision

The sandbox runner synchronously calls `POST /v1/script-runner/incidents` before returning a policy
denial to script code.

* The wire body contains a production or test lease target and one of seven closed egress
  categories. It cannot carry actor, digest, reason, hostname, URL, headers or payload.
* The control plane derives tenant and actor from authentication. For production it validates the
  actual run, step, namespace, script activity and artifact, attempt, generation, token hash,
  database-clock expiry, worker session and current binding. For tests it validates the locked test
  row, status, namespace, token hash and database-clock expiry.
* Production serialization is refined by [ADR-0038](ADR-0038-production-incidents-follow-execution-lock-entry.md):
  applicable member chain → exact own run → binding EXCLUSIVE → own step → audit dedupe.
  The original binding-before-run order has a reproduced cycle with ordinary report entry.
  Test incidents still lock their test-run row before audit deduplication. Current lease/binding
  validation and the commit-before-ACK boundary remain as decided here.
* The audit reason is the closed category. The resource key identifies the trusted attempt. The
  existing synchronous audit insert deduplicates the same category and bounds an attempt to seven
  rows without a new table or migration.
* The endpoint returns `204` only after commit. A duplicate after a lost response is also `204`.
  A stale fence is rejected; a storage failure is unavailable. The host treats incident-reporting
  failure as unavailable and does not acknowledge a denial.
* This is an audit-only transaction. It changes no execution state, schedules no task and writes no
  outbox event.
* Terminal execution diagnostics remain for compatibility. Once this attempt already has durable
  category evidence, the adapter suppresses only its URL-bearing `fetch …` audit duplicate. It does
  not suppress a different step at the same attempt number, secret denials or host-call limits.

The precise guarantee is: **a supervisor policy denial is durably recorded before the supervisor
acknowledges it to the script**. A crash before commit has no successful acknowledgement. This does
not promise that raw kernel socket failures or unacknowledged attempted calls are durably visible.

## Why this and not the alternatives

| Alternative | Why not |
|---|---|
| Keep terminal-only denial audit | A runner crash after the host response loses acknowledged evidence. |
| Record a fake terminal failure early | The first terminal row is idempotent and would replace the real outcome with fabricated state. |
| Use asynchronous namespace denial recording | It is intentionally lossy and cannot establish commit before acknowledgement. |
| Store destination text as incident identity | It is attacker-controlled, unbounded and can contain secrets. |
| Add an incident table and outbox | The append-only audit trail already supplies durable, deduplicated evidence; no external effect follows this audit-only decision. |

## Consequences

The first denial of each category costs one control-plane request and one PostgreSQL transaction;
retries cost the same validation but deduplicate. This is bounded by the sandbox host-call limit.
The network stays closed when the control plane is unavailable, while the script receives an
unavailable failure rather than a policy denial.

Revocation committed before incident validation wins. A denial committed before completion or
revocation remains attributable and durable. Existing clients and stored execution records remain
compatible because the endpoint is additive and terminal diagnostic fields do not change.

## Review criteria

Open a new ADR if denial acknowledgement no longer waits for the audit commit, if categories become
caller-defined, if a destination is persisted, if validation no longer uses the current fenced
lease and binding, or if this audit-only operation starts changing execution state or emitting an
outbox event.
