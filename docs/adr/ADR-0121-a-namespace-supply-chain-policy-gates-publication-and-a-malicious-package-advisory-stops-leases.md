# ADR-0121: A namespace supply-chain policy gates publication, and a malicious-package advisory stops leases

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24, revised after review rounds 1 and 2), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** refines ADR-0057 decision 8 (the language policy checked at lease): the package blocks join the same lease statement; supersedes the clause of ADR-0057's 2026-11-24 amendment that placed a `runnable` pair on the artifact row (corrected there; feature 023, Decision 1628)
**Enforced by:** planned tests of design 046: the policy and verdict-order unit tests; the matcher property tests against both version orders; `a_snapshot_marking_a_locked_package_malicious_blocks_its_next_lease`; `a_snapshot_imported_during_a_build_is_seen_before_signing`; `policy_change_reaches_the_lease`; `waiver_expiry_reaches_the_lease`; `reevaluation_resumes_after_a_crash`; `an_older_snapshot_is_refused`; `a_blocked_artifact_fails_at_lease`; `an_attempt_already_leased_finishes`. Until they exist: review only.

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
   records of npm and PyPI, imported by the control plane or by an operator). A version that fails is refused with a
   typed reason before it is signed. The report is stored with the version, outside the bundle.
3. A package matched by a malicious-package advisory (an OSV `MAL-` identifier or alias, or a malware type) is always
   refused, at publication and at lease; no policy or waiver can allow it.
4. A block is a row keyed by tenant, artifact digest, namespace (or all), reason and source. An advisory import stages its
   records outside any lock, is refused when older or shrinking and held for an operator when it would newly block more
   than 50 artifacts, and then switches in one short transaction that inserts a block for every artifact locking a
   package a new malicious record matches. A policy change, a waiver's change or expiry, a snapshot switch, a reservation
   and an upload's deletion each insert a durable re-evaluation job in the causing transaction; the job works one
   namespace at a time, in pages with a cursor, holding the policy and snapshot rows `FOR SHARE`, and writes or removes
   only its own kinds of block.
5. The engine MUST NOT lease a script step whose artifact has a block for its namespace or for all: the check is in the
   lease statement (`RunTx::script_execution_admitted_instance`), and a blocked step fails permanently in the lease
   transaction as `SCRIPT_DENIED`, reason `PACKAGE_BLOCKED`, with its journal event, next work, outbox rows and audit
   record. The same check refuses a materialization and the start of a run container. An attempt already started
   finishes. Feature 023 joins its language check into the same statement.
6. The finishing transaction of a version, on both the new-build and the cache-hit paths, reads the snapshot, the
   namespace's policy and the tenant's reservation revision `FOR SHARE` and re-evaluates if any changed since the
   evaluation; their writers take them `FOR UPDATE`. A malware block is released only by a platform operator, audited,
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

## Consequences

- The platform carries an advisory snapshot and two version orders behind an application port.
- An operator keeps the snapshot current; its age is a metric and a warning on publication.
- A blocked version stops at its next script step; the runbook says how to publish a fixed version and rerun failed runs.

## Evidence

None yet: this is a proposal. The tests named under "Enforced by" are required before acceptance.
