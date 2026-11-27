# ADR-0054: Run content leaves the hot store only through a verified archive under a namespace retention policy, and the run row stays as a stub

**Status:** Accepted (2026-11-24); its implementation waits for the principle-I amendment below
**Decided by:** agent proposal of 2026-10-26, accepted by an agent, delegate (Owner decision 2026-11-24-4),
after the readiness review of feature 020 on 2026-11-24 (`specs/020-visibility-analytics-archival/research.md`
§5a, Decision 020-014). Deleting customer data is irreversible; the default (no policy, nothing deleted)
and the stub rule were owner questions O1080 and O1082 of feature 020, decided on 2026-11-24 as this
record proposed (delegate, Owner decision 2026-11-24-4). Accepting the record fixes the design; it does
not amend the constitution, which a delegate may not do, so no code of it merges before the amendment.
Refined with the acceptance: archive destinations (decision 1), step logs of feature 041 (decision 6),
the instance registry for rollback (Consequences), and the journal-partitioning note (Considered option 3).
**Refines / supersedes:** refines ADR-0023 (proposes the decision of its open G69 for run-purpose blobs
only: `STEP_OUTPUT` and `RUN_INPUT` follow the run's retention; `DATA_SNAPSHOT_PART` blobs of feature 024
are excluded and their retention is ADR-0058's), ADR-0027 (purge releases content counters through the
existing triggers), ADR-0039 (after the source identity guard, rerun and reset read the purge state and
refuse a purged source) and the append-only enforcement of `run_events` and `payloads` (migration
`20261009000100_execution`). Supersedes none. Revised 2026-10-27 after independent review.
**Refined in part by:** [ADR-0132](ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md) (2026-11-24, design 049): a run owns its payloads, so the purge of one run deletes by its key with no cross-run reference check; whole months may also be dropped once every run in them is purged (option 3 below, for whole months only); references come from the registry this decision already plans. The stub, the eligibility rule and the verified archive are unchanged.
**Requires a constitution amendment:** retention changes an obligation of principle I, so it is a MINOR
amendment, not a clarification (see "Required amendment of principle I" below). This ADR proposes the
text; it does not edit the constitution. Nothing in it may be implemented before that amendment's pull
request is accepted (O1080 and O1082 were answered on 2026-11-24).
**Enforced by:** planned checks of feature 020 — the runtime-role privilege sweep (the runtime role still
cannot `DELETE` from `runs`, `steps`, `run_events` or `payloads`), the append-only trigger test with and
without a purge authorization, the reference-registry migration test (every catalog foreign key to
`payloads`/`blobs` and every content column with run provenance is covered), the archival fault suite
(`tests/fault/visibility-analytics-archival/archival.rs`) and the migration down/up test. None exists yet.

## Context

Nothing is ever deleted today: `run_events` and `payloads` reject `UPDATE`/`DELETE` by `ALWAYS` triggers,
runs and steps have no `DELETE` grant, committed blobs are never deleted (ADR-0023, G69), and
feature 002 G28 and feature 010 G241 record that the journal and payloads grow without bound. Customers
need bounded retention (cost, data minimization under LGPD), auditors need to read old runs, and
Temporal and Windmill both expire run history after a retention period (Temporal can archive it first).

Forces: deletion cannot be undone; an archive that was written but not verified is not evidence;
payloads are content-addressed and shared between runs; runs are linked by foreign keys (parent, root,
rerun provenance, chains); external sinks and customer buckets are outside the platform's control.

## Required amendment of principle I (proposal, MINOR)

Principle I today: the journal is append-only, and no component other than PostgreSQL holds the only copy
of a transition. Retention breaks both for some runs: it removes whole journals, and with
`archive = REQUIRED` the verified archive in object storage becomes the only copy; with `archive = NONE`
there is no copy at all. Proposed addition to principle I (governance: MINOR, a principle's guidance
materially expanded; Sync Impact Report and dependent documents in the same pull request):

> *Retention.* A namespace's explicit retention policy MAY remove the content of a **terminal** run whose
> tree is terminal and whose effects are settled. With archival required, the content is removed only
> after an archive copy was written and verified; that archive then is the **only** copy of those
> transitions and is no longer protected by the database's transactions or backups. With archival
> disabled, removal is **deletion** with no copy. A journal is never modified while its run can
> change, and no entry is ever rewritten; removal is of a whole terminal journal, by one audited
> privileged path.

## Decision

1. **Opt-in policy.** A namespace without a retention policy MUST keep every run (today's behavior). A
   policy sets `hotRetentionDays` (clamped to the operator ceiling), `archive` (`REQUIRED` or `NONE`),
   `archiveValues` (`METADATA_ONLY`, `UP_TO_INTERNAL`, `UP_TO_PII`; `secret` never) and a destination:
   the platform blob store, written and verified by the engine, or (revised 2026-11-24) an
   `object_storage` connection of feature 035, pinned by the policy revision and written create-only,
   read back and reported by the `lake` profile of the managed executor under a lease of the job, so the
   engine never holds a customer storage credential. `UP_TO_PII` requires a principal
   holding the reveal grant. Every change is audited, and the purge MUST refuse to apply a policy
   revision that is not an audited change by a human principal holding `retention:manage` (this raises
   the bar against an automated or compromised writer; it does not defend against a compromised runtime
   role that can also write audit rows).
2. **Eligibility on the database clock.** Only terminal runs, whose whole tree is terminal, whose chain is
   final, past retention, and applied by every non-`DELETED` analytics sink, paused ones included (or
   past its hold, recorded as a retention gap) are eligible. A run that ended `COMPENSATION_FAILED` or
   `PARTIAL`, or holds an unresolved unknown outcome or manual review, MUST NOT be eligible before an
   audited resolution — explicit erasure included: retention never erases the evidence of an effect
   nobody has settled (constitution III).
3. **Verified before delete.** With `archive = REQUIRED`, content MUST NOT be deleted before the archive
   object (`sagat-run-archive-v1`, deterministic name) was written and re-read with matching size and
   SHA-256. A mismatch blocks the run for a person; it is never retried into deletion.
4. **Privileged, audited purge.** The runtime role MUST NOT gain `DELETE` on runs, steps, journal or
   payloads. Purge MUST go through one `SECURITY DEFINER` function that verifies the job state and token,
   rechecks eligibility under the run-tree lock order and records a per-transaction authorization the
   append-only triggers consult. No session setting or argument can authorize a deletion.
5. **Stub.** The run row MUST remain with ids, workflow/version, status, times, tree/chain/rerun links and
   an archive pointer; `correlation_id` and 018's `business_id` are cleared, `started_by` is replaced on
   explicit erasure, and `input_digest` stays non-null by pointing to the tenant's tombstone payload, so
   no constraint is relaxed and an older engine reads a stub as a run without history. Hot-only
   endpoints answer `NOT_FOUND` with reason `RUN_PURGED`. The first purge chunk locks the run row `FOR
   UPDATE`, which conflicts with the ADR-0039 guard's key-share lock; rerun and reset admission then read
   the purge state after that guard and refuse a purged or purging source with `CONFLICT`/`RUN_PURGED`.
6. **Shared content and blob purposes.** References MUST be found from the database catalog (every
   foreign key to `payloads` or `blobs`, whichever feature added it) plus a registry of non-FK copies
   carrying run provenance, each with its purge and erasure rule: feature 020 owns this content-store
   registry (`run-content-references.json`), covering 018 retained Update arguments, 019 checkpoints, 022
   cache entries (invalidated by `source_run_id`, found through its hit-provenance table), 025
   transcripts and request staging blobs, 026 stream events and 030 operation messages, and (added
   2026-11-24) feature 041's step-log streams and lines, which have no foreign key to `runs` and are
   deleted in the run's purge; an archive holds their text only under `UP_TO_PII`, because log text is
   read as `pii` in every namespace; each draft
   registers its store with a pointer to feature 020. A test fails on an unregistered referencing column. A payload or blob
   still referenced by a retained row MUST survive; it is reported, not hidden. Only run-purpose blobs
   (`STEP_OUTPUT`, `RUN_INPUT`) follow the run to ADR-0023's deletion path (this proposes closing G69 for
   them); `DATA_SNAPSHOT_PART` blobs and 024's snapshot metadata (`data_snapshots`, parts, inputs, audits,
   publications, and the run-scoped data links) MUST NOT be deleted by run purge even when the producing
   run is purged, because a published snapshot may still use them; ADR-0058 owns their retention.
7. **Tombstones.** Purge and erasure MUST emit a `run_tombstone` to every analytics sink; the platform
   MUST NOT claim that data in sinks or customer buckets was deleted.

## Considered options

1. **Opt-in retention, verified archive, stub row (chosen).** Irreversible deletion only after evidence;
   foreign keys and provenance intact; the default deletes nothing.
2. **Fixed default retention (for example 30 days) for every namespace.** Rejected as a default: it would
   delete existing customer data at upgrade. It remains available as a namespace setting (O1080).
3. **Partition and drop by time (detach journal partitions).** Rejected for v1: partitions do not align
   with run lifecycles (a long run spans many), it needs the journal re-keyed (G241), and it cannot
   verify an archive per run before dropping. *2026-11-24:* the partitioning of the journal tables now
   gets its own design (owner decision of 2026-11-24, OD-2026-11-24-1). The purge here is defined per
   run and assumes no layout; if that design partitions by time, it may drop a partition whose runs
   are all `PURGED` instead of deleting row by row, never bypassing decisions 2 and 3.
4. **Delete runs completely (no stub).** Rejected for v1: it breaks tree, chain and rerun foreign keys
   (ADR-0039 guards rely on them) and audit references; stubs cost a small row per run (G1089, O1082).

## Consequences

- G69 and the payload/blob half of G28 get a decision; journal partitioning (G241) stays open.
- Stubs accumulate; a stub-retention rule is a later decision.
- A concurrent run that reuses a payload digest being purged can see one retryable failure (tested).
- Payload deletion needs indexed reference checks on every column referencing `payloads`; missing indexes
  add write cost to hot tables (benchmark gate, feature 020 G1099).
- Audit records keep identifiers after purge; audit retention is outside this decision.
- Rollback: removing a policy stops new eligibility; jobs already `PURGING` finish, others block. Purged
  content is recoverable only from its archive. Rolling engines back below feature 020 after a purge is
  supported with degraded reads only (stubs show no history); it is tested. While the `retention-v1`
  capability is a recorded requirement of the instance registry (ADR-0082), an older engine or control
  plane stays up with a closed gate, so a rollback first removes the requirement (revised 2026-11-24).
- 018 must allow the purge function to clear `business_id` and must write attribute rows for the
  `search_attributes.upserted` events a reset restores (feature 020 G1103). 018 is built: its reset
  registry lists the rows, its guard `runs_business_id_guard` does not consult a purge authorization yet,
  so feature 020's retention migration replaces that function, and `RUN_PURGED` is `CONFLICT` in both.

## Evidence

The design was accepted on 2026-11-24 against the code it relies on (the append-only triggers and
grants of `20261009000100_execution`, the ADR-0039 guard in `PgRunStore::create`, the blob deletion path
of `crates/infra-postgres/src/blobs.rs`, 018's guard and reset registry); nothing of it is built.
Required before the implementation is accepted: the accepted amendment PR of principle I, crash at each job transition, corrupted and missing archive objects, concurrent
digest reuse, tree/chain eligibility, sink hold, runtime-role privilege sweep, erasure report and the
purge benchmark. Specification: [specs/020-visibility-analytics-archival](../../specs/020-visibility-analytics-archival/spec.md),
research D1112–D1118 and Decisions 020-007, 020-011, 020-012 and 020-014.
