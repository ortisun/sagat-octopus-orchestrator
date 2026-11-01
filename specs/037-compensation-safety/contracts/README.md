# Compensation safety — proposed contract delta

This is a design delta, not another canonical wire contract. Canonical sources remain
[worker.proto](../../001-platform-foundation/contracts/worker.proto),
[OpenAPI](../../001-platform-foundation/contracts/openapi.yaml),
[workflow schema](../../001-platform-foundation/contracts/workflow.schema.json) and
[SDK SPEC](../../../sdks/SPEC.md). Implementation has not changed them yet.

## Delivery 1

- No new RPC, REST resource, workflow field, public status, database table or protocol minor is proposed.
- CompleteStep asserts the handler's final business result. Provider accepted/pending is reported
  through existing FailStep `error_code=OUTCOME_UNKNOWN`; a retryable flag does not override review.
  Compensation output remains discarded; embedding a pending marker in discarded output is invalid.
- Existing ReconcileStep `not_applied` now holds for review; it does not authorize automatic retry.
  Proposed persisted/API-visible failure code: `RECONCILIATION_NOT_FINAL` (1–64 allowed code characters).
  `applied` must mean final applied outcome and preserve required original output. `unknown` reviews.
- Original/target and compensation idempotency keys retain existing exact instance identity.
- Existing explicit saga `onCompensationFailure: fail` concerns known failure only. Collections/loops
  gain no policy field. `highRisk` remains readable but never licenses unsafe recovery.
- Retry of an unknown original or compensation via the existing review API returns its existing
  typed state-conflict response, even for SagaOverride. Privilege alone is not outcome resolution.
  Authorized Resolve records outcome/reason first; existing Skip remains compensation-only. No new
  in-place retry-after-resolution is promised. Known-failure retries retain their existing rules.
  The refused transaction writes no state/outbox/idempotency-success/success-audit record. Existing
  request tracing and retained review history explain the conflict; no new durable conflict audit
  is promised. Existing permission-denial audit behavior and mandatory atomic audit for accepted
  human decisions remain intact. Cancellation is not an override.
- All six SDKs must preserve OUTCOME_UNKNOWN through error mapping. Shared conformance adds
  `compensation-safety` scenarios for pending, negative lookup, known fail, cancellation and manual
  provenance. Expected output demonstrates runtime behavior, not simulated provider qualification.
- Existing operator views must display retained state/failure code and allow inspection of current
  resolution evidence. No manual-provenance UI redesign or new terminal status is required.

This is a breaking behavioral contract despite unchanged message shape. Fresh coordinated installation
with matching tested clients is the proposed cutover. Do not run mixed server generations or silently
reinterpret in-flight data. Any discovered need for a wire shape/negotiation change returns to design
review; do not take a protocol number from another feature.

## US04 boundary

No public qualification field is accepted or activated in delivery 1. Existing schema/parser rejection
must cover unknown fields; use negative contract tests rather than an inert stub. Worker binding,
plugin installation, `compensateWith`, `reconcileWith` and author-supplied risk flags are not qualification.
A real provider/capability and immutable adapter evidence package must precede the later exact
reference, authority/revocation, publication/use and SDK-wire design. No provider endpoint is called
by this feature's synthetic conformance tests.


### Delivery-1 secret-output compatibility

The owner retained the feature-009 ordinary `SECRET_IN_OUTPUT` final-failure contract on
2026-11-01. Source-aware recovery for successful storage rejection must not be generalized to
that path: secret-bearing completion may still fail/route onError without active review, and the
secret payload is never retained. Existing managed-database protections remain unchanged. See
[owner decision](../../../docs/owner-decisions.md); a later change needs cross-feature approval.
