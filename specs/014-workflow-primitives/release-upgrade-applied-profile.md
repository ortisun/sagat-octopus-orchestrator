# Actual released-worker Applied reconciliation checkpoint

Refs: T014-016/017/040/043, FR-014-005/014/015/017. This test-only profile extends the separate
released FORWARD in-flight proof. Public capability, platform behavior and quotas are unchanged.

The immutable released source publishes reserve→charge with charge.reconcileWith=line.lookup,
completes reserve, leases charge and reports OUTCOME_UNKNOWN through its actual gRPC client.
The old scheduler creates the qualified charge~r lookup; the old worker polls that genuine
RECONCILIATION task and verifies exact forward target instance/node/provider idempotency key.
Both original task capabilities and the Applied completion request remain only in old memory.
Anonymous bounded inherited IPC carries metadata/hash commitments and refreshed subject auth.

The current coordinator checks the original WAITING forward's attempt/generation/reported token
commitment and default provider-key commitment, plus the lookup's exact live physical identity,
path/token/fence/wire expiry and600s lifetime margin. It applies ordinary schema20→23 migration,
starts current runtime-role services and rechecks before GO. The exact old generated client
reports {effect:applied,output:{hold:7}} and replays it. ACK flags differ as required; command
and status fields match. The old reader verifies retained forward output and immutable prefixes.

Current state must retain both forward and lookup as SUCCEEDED with original capabilities/
fences and no live lease fields. One StepCompleted belongs to the lookup; one Applied/Succeeded
ReconciliationConcluded belongs to the original forward. No synthetic forward StepCompleted
is required. The lookup's scoped receipt binds the exact report-wrapper fingerprint; only the
substituted forward result is retained/charged as its logical output. Current scheduler must
perform actual work and finish/replay after old child exit and stopped-owner takeover.

The existing runner binds selected coordinator, exact five overlays and both copied binaries
to frozen verified source. Run with Docker reachable, pinned toolchain and an owned absolute
CARGO_TARGET_DIR outside the repository:

```sh
python3 scripts/test-release-upgrade-profile.py --scenario applied-lookup
```

This does not prove quota-refusal/drain frontier, original-forward compensation under a stop,
expired/stale reports, mixed dispatcher activation, new public bodies, all-five SDKs or final
resource/release acceptance. Actual results will be recorded separately in the0.3 ledger.


The pre-migration forward also has waiting_kind=RECONCILIATION, failure_code=OUTCOME_UNKNOWN,
no live lease and exactly one matching reconciliation.requested with its original fence.
After reporting, both rows explicitly clear failure/waiting fields. Lookup row and lookup
completion journal retain no output; the Applied event payload points to the forward digest.
Logical-byte delta is measured immediately before scheduler work, since final aggregation
legitimately retains additional values. Scenario/transcript/compiled test/child/result mode
must agree. The old API is idle and no old scheduler tick occurs after the metadata barrier.


Shared ProfileMode::{Forward,AppliedLookup} makes selection explicit. The old wire ACK must
itself report RUNNING; the forward has exactly one total reconciliation conclusion as well
as one matching Applied verdict. A second process of the same exact old executable reads the
terminal run through real REST and native replay after current finalization, then recomputes
the original prefix hash. Result metadata therefore distinguishes this post-final reader from
the earlier old replay immediately after the report. Both phases use bounded anonymous IPC.
