# ADR-0121: A namespace supply-chain policy gates publication, and a malicious-package advisory stops leases

**Status:** Accepted (2026-11-26, delegate (OD-2026-11-24-4), Owner decision 2026-11-24-312), with the approval of design 046; proposed 2026-11-24
**Decided by:** agent proposal (design 046, 2026-11-24, revised after review rounds 1 to 3 and the re-reads of 2026-11-26), accepted on 2026-11-26 under Owner decision 2026-11-24-4 with the approval of design 046 at `cd82af93` (Owner decision 2026-11-24-312), on the approval rule of Owner decision 2026-11-25-2
**Refines / supersedes:** refines ADR-0057 decision 8 (the language policy checked at lease): the package blocks join the same lease statement; supersedes the clause of ADR-0057's 2026-11-24 amendment that placed a `runnable` pair on the artifact row (corrected there; feature 023, Decision 1628)
**Enforced by:** planned tests of design 046: `verdict_order_*`; `matcher_agrees_with_the_version_order_*`; `a_malware_import_blocks_the_next_lease`; `a_blocked_child_script_step_fails_at_lease`; `a_blocked_step_with_no_materialized_runner_fails`; `a_malware_import_during_materialization_stops_the_start`; `a_snapshot_imported_during_a_build_is_seen_before_signing`; `the_switch_rechecks_age_under_the_lock`; `a_finish_between_staging_and_switch_is_matched`; `a_held_import_refuses_matching_finishes`; `an_older_snapshot_is_refused`; `policy_change_reaches_the_lease`; `waiver_expiry_reaches_the_lease`; `a_waiver_expiring_during_a_build_is_seen_at_finish`; `reevaluation_resumes_after_a_crash`; `reevaluation_never_removes_a_malware_block`; `a_page_never_removes_a_fresher_reservation_block`; `a_waiver_cannot_cover_malware`; `an_attempt_already_started_finishes`; `activation_requires_the_block_check`. Until they exist: review only. Also `a_lock_written_by_a_transaction_open_across_the_barrier_is_matched` and `a_large_import_is_held`. And `the_switch_is_not_starved_by_overlapping_writers`, `the_barrier_is_not_starved_by_overlapping_writers`, `publications_during_the_switch_never_hold_it`, `a_late_writer_with_a_lower_sequence_gets_a_reevaluation_row`, `a_lease_racing_the_block_pass_is_left_to_admission`, `a_step_ready_after_the_block_pass_fails_at_materialization`, and `reevaluation_never_removes_a_malware_block` for a malware block written by a re-evaluation page.

## Context

A lock pins what runs, but not whether it should. Windmill applies no policy to the packages a script imports. A
version that was clean at publication can be found vulnerable or malicious later. Blocking every version that gains a
vulnerability would stop production work on a disclosure the owner may judge harmless; blocking nothing leaves known
malware running. Artifact rows are immutable (`script_artifacts_immutable`), and one artifact may serve several
namespaces with different policies.

## Decision

1. Each namespace has a supply-chain policy, the `packages` section of its script policy document (feature 023's
   `languages` section beside it): per ecosystem enabled or not, allow and deny rules, a release-age quarantine, a
   vulnerability threshold, denied licences, install-script handling, a layer ceiling; waivers are rows with an expiry of
   at most 90 days. Every change is audited.
2. Every publication evaluates the lock against the policy, the gateway's metadata and the advisory snapshot (OSV
   records of npm and PyPI, which the gateway fetches as an upstream and the control plane imports, or which an operator
   imports offline). A version that fails is refused with a
   typed reason before it is signed. The report is stored with the version, outside the bundle.
3. A package matched by a malicious-package advisory (an OSV `MAL-` identifier or alias, or a malware type) is always
   refused, at publication and at lease; no policy or waiver can allow it.
4. A block is a row keyed by tenant, artifact digest, namespace (or all), reason and source. An advisory import stages its
   records outside any lock, is refused when older or shrinking and held for an operator when it would newly block more
   than 50 artifacts, and then switches in one short transaction that inserts a block for every artifact locking a
   package a new malicious record matches. A policy change, a waiver's change or expiry, a snapshot switch, a reservation
   and an upload's deletion each insert a durable re-evaluation job in the causing transaction; the jobs of a
   tenant are worked in creation order, in pages with a cursor, holding the ecosystem's snapshot lock shared and the
   policy and reservation-revision rows `FOR SHARE`, and write or remove only their own kinds of block, never one evaluated at fresher revisions. The switch
   rechecks age and size under the ecosystem's snapshot lock (an advisory lock that decisions take shared and the switch exclusively), and a held import still refuses the finishes and materializations
   it would block.
5. The engine MUST NOT lease a script step whose artifact has a block for its namespace or for all: the check is in the
   typed admission (`Admit`, `Wait`, `Deny`) that both lease paths call (`lease_one` and the scoped claims), and a blocked
   step fails permanently in the lease
   transaction as `SCRIPT_DENIED`, reason `PACKAGE_BLOCKED`, with its journal event, next work, outbox rows and audit
   record. The same check refuses a materialization and the start of a run container, and a block pass fails the ready steps
   of a blocked artifact that no runner advertises. An attempt already started
   finishes. Feature 023 joins its language check into the same statement.
6. The finishing transaction of a version, on both the new-build and the cache-hit paths, takes the ecosystem's snapshot
   lock shared and the namespace's policy and the tenant's reservation revision `FOR SHARE`, and answers `StaleInputs`
   when any differs from what the application evaluated against (the application evaluates again); the switch takes the
   snapshot lock exclusively, and the policy and reservation writers take their rows `FOR UPDATE`. A malware block, whatever wrote it (the switch or a re-evaluation page), is released only by a platform operator, audited,
   once its advisory is withdrawn (a database trigger refuses otherwise).

## Considered options

1. **Gate at every publication, block malware at once, report other findings, block them at lease only where the policy
   asks (chosen).** Known malware never runs; a disclosure becomes a visible finding with a clear owner.
2. **Gate at publication only.** Malware found later keeps running. Rejected.
3. **Block every new finding at lease.** One disclosure can stop a business process whose exposure is nil. Kept as an
   option of the policy, off by default.
4. **Query an online vulnerability service at each publication.** No offline mode, a third party sees every package a
   tenant uses, an outage blocks publication. Rejected.
5. **A column on the artifact row.** Artifact rows are immutable and one column cannot hold two reasons or a namespace
   scope. Rejected in review.
6. **One import transaction replacing every record.** It would hold the row every finish needs for the whole import.
   Rejected in review.
7. **Order the switch with the snapshot row's `FOR SHARE` and `FOR UPDATE`.** New shared row lockers can pass a waiting
   `FOR UPDATE`, so a stream of publications could starve the switch. Rejected for a per-ecosystem advisory lock, whose
   requests queue in order (design 046, Decision 046-078).

## Consequences

- The platform carries an advisory snapshot and two version orders behind an application port.
- An operator keeps the snapshot current; its age is a metric and a warning on publication.
- A blocked version stops at its next script step; the runbook says how to publish a fixed version and rerun failed runs.

## Evidence

None yet. The direction is accepted with design 046's approval (Owner decision 2026-11-24-312); the build owes the tests
named under "Enforced by", and one that fails against a decision above reopens it.
