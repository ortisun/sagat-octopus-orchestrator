# Scoped output refusal: durable discovery amendment

Status: bounded correction accepted after fresh C1 (None); implementation/runtime acceptance pending.

## Finding and decision

The Q7b implementation review found that the selected report transaction creates zero event_outbox rows. Q section11.2 item5 incorrectly required an unconditional report outbox record. Existing persistence emits outgoing obligations for actual task releases and other explicit events; this refusal releases no task and starts no drain. telemetry_pending is a durable journal-head marker, not an outgoing message or scheduler trigger.

Correct that wording to the existing durable collection discovery. Do not add a fictitious READY hint or announce a drain before it exists. Preserve atomicity for every actual outgoing obligation. The later native collection drain still writes its existing collection.drain.started record with its own transition. This introduces no wire message, flag, grant, profile, polling mechanism or execution shape.

Scope remains the existing supported depth1 root ForEach report shape. Generic domain preparation remains dormant for loops and other nesting until their matching consumer is reviewed. Existing report-specific authorization, lease/cancellation/expiry and scanned-secret-version policies remain required; this amendment changes none of them.

## Source evidence

- Constitution I requires state, scheduling and their outgoing announcement to share a transaction; VII requires every outgoing message to be recorded with its cause. No existing outgoing obligation may be omitted or reconstructed from telemetry.
- ADR-0002 makes journal, logical queues and outbox authoritative in PostgreSQL. ADR-0017 decision1 requires telemetry_pending to advance with a changed journal; its title describes outbox as obligations.
- infra-postgres/maintenance.rs collection_candidates includes CANCELLING Runs and explicitly selects their OPEN collections. scheduler/passes.rs tick/collections invokes that query and advance_collection_in_view through the owned-shard window.
- Canonical specs/001-platform-foundation/contracts/events.asyncapi.yaml lines44-48 describes primitive hints as observational; platform execution does not consume them.
- Effects.rehint/rehint_instances announce existing READY work. They cannot truthfully represent this stopped success.

Inference: this corrects an inaccurate clause to existing semantics and does not diverge from an ADR. The cold-discovery and atomicity tests below are still mandatory before activation.

## Exact accepted Q amendments

Replace section11.2 item5:

> In one transaction, persist the first RUN_DATA_LIMIT stop if absent as a root-only phase, then persist the genuine success/verdict events as a scoped-only phase, with zero output references, zero blob commits, existing reported-lease evidence, typed audit, original ACK receipt and transactional journal/projection updates. Each nonempty persist advances telemetry_pending in that transaction; the final committed head equals the final journal sequence. The refusal audit/report provenance belongs to the scoped phase even when a prior stop makes the root phase unnecessary. Preserve any existing stop byte-for-byte and skip the empty root persist. Neither phase commits separately; there is no ordinary root settlement or expression hydration.
>
> This report transaction releases no task, starts no collection drain and creates no outgoing-message obligation; its expected new event_outbox row count is zero. The committed stop, success metadata and collection state are the durable scheduling condition. Every applicable outgoing obligation remains atomic with its creating transition. The later actual drain writes collection.drain.started in the drain transaction. Do not fabricate a READY-task hint or announce a drain during report refusal.

Replace section11.2 item7:

> After commit, ordinary Scheduler::tick discovers the CANCELLING OPEN collection through collection_candidates under its owned-shard window, then applies the reviewed Q8/Q9 decision and native transaction using StoppedForRunDataLimit. This requires no report notification, broker delivery or surviving report-process memory. Existing cancellation/different-failure stops retain their supported drain policy; an existing DRAINING collection resumes its recorded obligations. A cold restart after report commit and before drain must recover from durable rows. The actual later drain retains its own atomic journal, audit, telemetry and outbox requirements.

In section11.4 bounded refusal recovery, distinguish report faults from drain faults:

> Report audit, receipt and telemetry faults, including an injected error after root-only persist and before scoped persist, roll back the whole report transaction. Separately, failure of the actual drain outbox insertion rolls back that drain while retaining the previously committed report and stop.

Append to section11.5:

> Q7b uses existing durable collection discovery and introduces no report-specific message. This corrects unconditional report-outbox wording under Constitution I/VII and ADR-0002/0017, without relaxing atomicity for actual outgoing obligations. Activation still requires Q8/Q9 and cold-discovery evidence.

## Required acceptance, not results

1. Real ordinary and Applied refusal: exact stop/success/verdict/receipt/reported lease/audit/final telemetry head; zero new payload/blob references, forward releases or report outbox rows. Include first and prior stops.
2. Commit refusal, reconstruct services/process state and shard ownership, and invoke normal Scheduler::tick without direct drain calls or hint delivery. It discovers and drains both E>limit and E<=limit<E+candidate. Use genuine old-producer state for compatibility claims.
3. Inject between-persist, report audit/receipt/telemetry failures; prove full rollback and one valid retry. Lost-ACK replay adds no effect/audit/outbox/head advancement.
4. Reject the later actual collection.drain.started insertion: no partial drain, original committed report remains, then one valid drain retry.
5. Withhold primitive delivery and repeat cold discovery and journal verification, in default/runtime-role gates. Existing lost-hints fixtures are patterns, not acceptance for this new path.

No tests/builds were executed for this document. No deployment or task closure is inferred.

Fresh bounded C1 reviewed originalbd5ff35f52ef12bc8d0cd406d7ac81e97d2007409aef9e317a023f93ae295395 and found no required issue; source/constitution/ADR mapping accepted. This status update adds no test result.
## Coupled producer checkpoint — 2026-10-18

The reviewed worker refusal path and collection scheduler/native producers now have focused
[combined integration evidence](evidence/collection-finalization-integration-20261018/reproduce.md).
Both genuine successful forward and Applied refusals pass through durable discovery and 17 real
compensations in default/runtime invocations. Report replay preserves its ACK and writes no new
report outbox row. These changes are a coupled deployment dependency; the PostgreSQL/scheduler
and worker commits are not independent release candidates.

This checkpoint does not close Q9/Q10. Genuine old-produced effective-over-limit discovery with
full concurrency, report transaction faults/current-secret races, cold recovery/uncertain COMMIT
and the remaining release gates still require their separately recorded acceptance.


## Report/drain transaction-fault checkpoint — 2026-10-18

Required acceptance 3/4 now has focused current-version [evidence](evidence/refusal-transaction-faults-20261018/README.md)
for the post-root/pre-scoped journal failure and actual drain-outbox failure. The first rolls
back the entire report; the second preserves the committed report and rolls back its drain.
Same-key replay and normal retry/compensation pass, as do strict integration lint/fmt/architecture.
This does not replace the remaining audit/telemetry points, current-secret/authority races,
cold report recovery/unknown COMMIT, immutable-old known-success cases or rollout gates.
