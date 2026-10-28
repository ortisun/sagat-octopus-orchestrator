# Scoped event entry admission checkpoint

Refs: T014-014–017/040, FR-014-004/005/014/015/017.

Admit a collection body whose pinned entry is WaitEvent using the existing Scheduler dispatch
kind. Its membership has no activity/outbox identity; its physical step starts PENDING with no
worker, event binding or clock decision. Reserve the other body instances through the existing
Run/tenant admission transaction and scope revision. Do not charge an event receipt or payload
at admission. Native body identity, namespace, canonical UUID and current scheduler authority
are unchanged. An Activity dispatch cannot impersonate a WaitEvent entry.

The collection scheduler later starts this exact pending wait through the existing correlation
snapshot, revision fence and own-shard database-clock acknowledgement. Full replay, path snapshots
and frontier readers recognize only a matching Scheduler membership and admission journal pin.
They must refuse a mismatch between declared kind and persisted dispatch/worker metadata.
Current quotas still control new iterations, including zero; already admitted work may finish.
Pause prevents new admission/start. Cancellation drains worker-free pending/waiting entries and
retires reservations without waiting for a nonexistent worker. Existing event delivery/deadline
callers decide outcomes; this adds no new event authority or timeout behavior.

Tests use internal immutable structured definitions and actual scheduler admission in both
full/frontier modes. No public publication capability or worker protocol changes. Actual old
readers, public acceptance and five-language runtime evidence remain separate gates. This uses
existing nullable Scheduler schema and grants; no new migration or parallel dispatch vocabulary.


The first native acceptance fixture enters the real collection before admission and pins a body
with only WaitEvent → Return, then root End. Both scheduler modes must first persist a pending
Scheduler membership, pass full/path cold reads, start two distinct item keys, receive actual
inbox admissions through EventDispatcher, finish the whole Run and retain no task.ready hints
or held iteration slots. Initial tests fail2 at actual admission with InvalidProgress (5.20s
harness/61.50s command); extending the three kind checks passes2/0/0 (5.54s/72.55s).

Additional current controls cover pause/resume before admission, cancellation after pending
admission before any subscription, explicit-zero policy after admission, forced admission
journal rollback/retry and CHECK-valid worker-dispatch corruption refused by all three readers.
These fixtures do not claim a public parser path or a released reader upgrade.


Admission refusal preserves the existing internal command contract: a direct paused admission
returns typed Primitive(InvalidTransition), while current zero iteration capacity returns
Primitive(AtCapacity). The normal scheduler discovery excludes paused runs and tallies capacity
as skipped. These are not new Waiting-return guarantees. The expanded test gate initially
passed5/fail2 (22.18s/76.16s) because its assertions incorrectly expected Waiting for those
typed refusals. Corrected controls assert the exact errors, unchanged execution state and
actual Scheduler.tick with failed=0. No production error mapping is relaxed to satisfy them.


The corrected seven-case native gate passes7/0/0 (26.64s harness/81.05s command). A separate
later single-connection/cache-reset case proves actual durable candidate discovery and scheduler
ticks start both pending waits without worker hints (1/0/0,2.46s/60.18s); it does not depend on
a direct per-owner advancement to start the subscriptions. Production kinds are unchanged
after the original two-case correction. Broader gate provenance is in implementation-0.3.md.
