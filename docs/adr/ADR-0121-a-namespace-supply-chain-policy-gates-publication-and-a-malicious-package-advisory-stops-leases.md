# ADR-0121: A namespace supply-chain policy gates publication, and a malicious-package advisory stops leases

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24, revised after review round 1), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** refines ADR-0057 decision 8 (the language policy checked at lease): the package blocks join the same lease statement; supersedes none
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
4. A block is a row keyed by tenant, artifact digest and scope (one namespace, or all). The advisory import inserts, in
   its own transaction, a block for every artifact that locks a package a new malicious record matches. A policy change,
   a waiver's expiry, a snapshot import and a private-name reservation each insert a durable re-evaluation job in the
   causing transaction; the job recomputes findings in pages with a cursor and writes or removes blocks (for every
   namespace for malware and reserved-name conflicts; for one namespace with `blockNewAtLease`).
5. The engine MUST NOT lease a script step whose artifact has a block for its namespace or for all: the step is failed in
   the lease transaction as non-retryable `SCRIPT_DENIED`, reason `PACKAGE_BLOCKED`, with its journal event, next work,
   outbox rows and audit record. An attempt already leased finishes. The check is part of the existing lease statement
   for script activities, with no row lock; feature 023 joins its language check into the same statement.
6. The finishing transaction of a version reads the ecosystem's snapshot row `FOR SHARE` and re-evaluates if the
   snapshot changed since evaluation; an import holds it `FOR UPDATE`. An import older than the stored snapshot, or with
   1 percent fewer records, is refused unless an operator forces it, audited. A malware block is removed only by an
   administrator, audited.

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

## Consequences

- The platform carries an advisory snapshot and two version orders behind an application port.
- An operator keeps the snapshot current; its age is a metric and a warning on publication.
- A blocked version stops at its next script step; the runbook says how to publish a fixed version and rerun failed runs.

## Evidence

None yet: this is a proposal. The tests named under "Enforced by" are required before acceptance.
