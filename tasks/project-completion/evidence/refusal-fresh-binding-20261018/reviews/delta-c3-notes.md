# Fresh binding authority refusal C3

## Scope

Incremental correction for the final post-rejection oracle in the single fresh binding authority integration test. The repository was treated as read only, and the prior C1 and C2 artifacts were preserved.

## Corrected contract

- The completion is submitted through a newly connected native worker gRPC client using the existing public worker request helper, the original leased task, output, and idempotency key, with no worker registration or authority mutation.
- A generated W3C trace context is attached as request metadata so the denial audit has an exact causal identity.
- Fresh full authority still returns permission denied with the closed not-bound machine code and persists no completion, report refusal, stop, output, receipt, journal, head, outbox, or quota change.
- The service must append exactly one policy denial audit before returning. The test validates every audit column against the worker, tenant, RPC, action, resource type, derived closed refusal reason, supplied trace, and report time window.
- Removing only that validated audit from the post-denial snapshot must reproduce the complete post-unbind snapshot.

## Source basis

The gRPC observer waits for the permission denial audit task before returning the denied response. The application derives the reason from the closed access denial type and records the RPC as an endpoint. Permission denial audits are never deduplicated. Audit records are absent from the retained quota measurement and trigger list.

## Verification

`git apply --check` passed against the current C2-applied source. `git diff --no-index --check` passed. Per task constraints, no formatter, build, or test command was run.
