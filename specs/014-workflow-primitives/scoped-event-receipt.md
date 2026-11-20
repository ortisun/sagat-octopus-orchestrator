# Scoped event receipt and value transaction checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017. This native-port checkpoint follows
start/timeout/drain and pure satisfaction. It does not activate public bodies or dispatcher
discovery/production; old-dispatcher cutover and released-profile proof remain required.

## Current authority and immutable input

Add the same optional scoped-delivery operation to full and exact-path transactions, defaulting
to explicit unsupported for older adapters. It takes existing WaitingStep and ClaimedEvent
types and returns canonical value bytes only when a new exact receipt was staged. Neither
input grants authority. Under the existing chain → own Run entry and prepared revision fence,
validate current global/ancestor eligibility (pause permits outcome, stop/drain does not),
tenant/namespace/run/path/node, canonical stored UUID, pinned WaitEvent and the captured/current
type/key/deadline/since binding. A root candidate keeps the old receiver and is refused here.

Lock/read the current inbox row in the same transaction, using its full tenant/namespace/source/
event key. Match every canonical-value field with the supplied admission snapshot: received_at,
occurred_at, type, subject, correlation key and payload, in addition to the keyed identity. Retry
attempts/claim TTL are liveness, not new authority or admission identity. Check stored payload
digest by recomputing the shared existing admission fingerprint from the locked row (type,
subject, correlation and payload); do not mistake it for a raw payload hash. occurred_at is
outside that legacy fingerprint and is compared separately. Check valid bounded JSON object
before encoding. Generate the retained document from the
current database row with the shared canonical encoder, never from caller-provided document
bytes or a stale claim. Preserve existing root lookback/retention admissibility; do not invent
a new claim-generation guarantee. The native binding check and pure command both use the same
inclusive lower bound, with no upper deadline until another outcome commits.

Keep entity locking before deferred aggregate quota flush (ADR-0027). In particular, retain the
Run/inbox/step/output-ledger entity locks through receipt and value commit; take no aggregate
tenant usage lock and then acquire another entity lock. Reuse existing quota delta/commit checks.
Prove real ingress/expiry/wait interactions with held locks; do not assume claim TTL fences old
dispatchers. This checkpoint does not implement that separate activation barrier.

## Receipt and completion witness

The new consumer is disjoint from legacy root/trigger keys: wait-instance:{run}/{stepUuid}.
Fill existing event_deliveries instance UUID/path columns; legacy root/trigger rows and strings
stay byte-for-byte unchanged. Insert once per current event/consumer. On conflict, validate the
existing row's namespace/key, run, node, null trigger, instance UUID and both path arrays. An
exact duplicate returns no value; mismatched/missing metadata is corruption, not ordinary replay.
The conflict read is a plain SELECT: the runtime role has no UPDATE grant on receipts. This
read authorizes no outcome and returns no value; the parent inbox remains locked. Concurrent
removal cannot create a successful delivery through this no-work path. No schema or grants.

Maintain a private transaction witness of the exact instance, receipt and canonical output
digest. Native scoped WaitSatisfied and its one output require that witness and exact source
binding CAS. Generic completion/resolve/failure events cannot manufacture an event value or
impersonate receipt authorization. A receipt staged without its matching outcome/value, or a
failed persistence whose error was ignored, must refuse commit. Consume the witness only after
the complete successful write. A duplicate/stale/foreign candidate retains no receipt or value.
Arm a persistence failure bit before every validation/await in root, full scoped and bounded
writers. On complete success restore its prior value; preserve any earlier failure across
successful pre-receipt retries. Receipt staging refuses such prior history, and any attempted
receipt makes an armed transaction refuse subsequent persistence and commit even after a
previous successful witness consumption. Legacy no-receipt retry behavior remains unchanged.
One receipt operation maximum is allowed per transaction: a second operation is refused before
any second insert, even if the first witness was consumed. Never overwrite the witness. Test
two staging attempts followed by one outcome as well as commit of a single orphan receipt.

Extend logical output charging and journal payload association for authorized WaitSatisfied,
and allow its normal Succeeded physical shape only with output digest, no timeout/failure and
no subscription/scan/worker capabilities. Handled timeout remains output-free. Paused successful
delivery may remain unrouted; subsequent explicit cold routing keeps the original value.
Existing namespace/tenant, immutable classification rules, retention/read-capability and payload
projection paths must protect the value; prove secret/PII-marker behavior before acceptance.

Receipt, payload, retained/logical usage, journal, projection, route, audit and telemetry/outbox
commit or roll back together. No worker task/lease represents a waiting event. A lost commit
acknowledgement/duplicate must retain one value/outcome; event/timeout/cancel follows first commit.

## Required evidence

Use the named immutable structured-codec fixture with actual admitted activity → waiting member,
and real EventService admission/claim under sagat_app. Prove full/path and same-name member
isolation, exact canonical value/receipt metadata and legacy root compatibility; no-value/orphan
receipt/generic-event refusal; stale/foreign/tampered admission and binding; current quota and
classification protection; rollback at late stages; duplicate/lost acknowledgement; paused cold
resume; both event/timeout/cancel lock orders; source cleanup under current locks; no worker
ownership. Native discovery/public bodies and canonical release tasks stay closed until all
later activation, SDK, nesting, rollout and resource gates pass.
