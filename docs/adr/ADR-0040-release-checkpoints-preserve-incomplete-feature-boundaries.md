# ADR-0040: Release checkpoints preserve incomplete feature boundaries

**Status:** Accepted (2026-10-25; implemented-task release authorized by the owner)
**Decided by:** Release authorization: Marcelo’s quoted instruction below. Boundary interpretation: implementation agent, independently reviewed by `review_release_notes_and_scope`.
**Refines / supersedes:** Only ADR-0036's unqualified final release-prerequisite sentence. Every runtime, authority, migration and recovery decision in ADR-0036 remains in force.
**Enforced by:** The public IR refusal tests `collection_publication_keeps_nested_bodies_closed`, `an_approval_in_a_collection_or_loop_body_waits_for_scoped_compensation` and `a_database_node_is_root_only_and_never_beside_scopes_or_continuations`; the repository CI and release build/package/DR gates. Release-claim accuracy is review-only: automated task counts cannot establish the meaning or completeness of a durability/capacity guarantee.

## Context

The owner requested: “Quero que mergeie todas as tasks já implementadas. Crie as versoes e releases necessárias”. This authorizes integration and publication of implemented work; it does not accept missing tests, pending production ceilings or general nesting.

The feature-014 task plan distinguishes desired story slices from a complete feature release. Eight tasks remain open: T014-014–017, T014-040 and T014-042–044. ADR-0036's final sentence instead calls complete worker/provider/blob/control/finalizer wiring and accepted large-workload evidence unqualified release prerequisites. A checkpoint for completed tasks needs an explicit boundary rather than silently removing that obligation.

The public parser admits restricted subsets and rejects nesting it cannot execute. The source certifier is a separate boundary: a private proof of an ordinary child with a ParentLink neither adds a production caller nor grants a nested capability. Managed database nodes are root-only and cannot share a definition with collections, loops or continuation policy.

The current 10,000-item declaration maximum is not an accepted production capacity ceiling. The feature-014 performance record includes quadratic work (G461, 1h54m) and idle-tick growth (G468); later bounded progress proofs do not establish the complete large-workload acceptance. No release label may turn those open criteria into a measured production guarantee.

## Decision

1. Release 0.2.0 MUST describe the restricted publication subsets identified in its integrated evidence. It MUST NOT declare feature 014 complete or its pending capacity/control/finalizer criteria satisfied.
2. Full worker/provider/blob/control/finalizer integration and accepted large-workload evidence MUST remain prerequisites for the complete feature-014 guarantee. This replaces only the broad final release-boundary sentence of ADR-0036.
3. Every exposed subset MUST pass its security, durability, compatibility and repository CI gates before merge/publication. A checkpoint MUST NOT waive a failing, flaky or skipped required gate.
4. This decision MUST NOT change a publication capability, permission, quota, runtime default or unsupported-path refusal. Accepted declaration syntax MUST NOT be presented as accepted production capacity.
5. Pending tasks and unverified working-tree work MUST remain explicitly excluded. SDK package version 0.2.0 MUST remain independent of wire capability negotiation.
6. Operators MUST apply migrations once as database owner, hold new capability publication/starts until compatible engines and readers are deployed, and retain compatible readers/workers until new-capability runs drain during rollback. They MUST preserve required schema/history and the coordinated OpenBao recovery boundary.

## Considered options

1. **A bounded checkpoint with explicit incomplete guarantees** — delivers the implemented work the owner requested while retaining the full feature's prerequisites and all technical gates.
2. **Wait for every feature-014 task before releasing any completed feature** — conflates the separate PostgreSQL/OpenBao and classification deliveries with unfinished broader execution work, contrary to the implemented-task checkpoint scope.
3. **Declare the whole feature complete or relax gates for the checkpoint** — rejected: missing integration/resource evidence cannot be manufactured by versioning, marketing or passing a smaller fixture.

## Consequences

The CLI and five SDK packages use one 0.2.0 version under the existing pre-1.0 breaking-minor policy. The owner did not explicitly name this number. Distribution remains GitHub Releases only, with a matching Go submodule tag; publication deploys no environment.

Users must consult the restricted workflow runbook and provisional capacity guidance. The eight open tasks retain their evidence and criteria. No source proof activates a production path. CI must still verify the integrated candidate, including existing security, tenant, fault and compatibility contracts.

The event-read change requires client changes. Standalone unreleased 017 migration histories remain refused; there is no checksum rewrite or invented bridge. Application rollback alone does not restore an OpenBao cutover: retain PostgreSQL/blob references, Raft state/tombstones and the intent-binding key, following the quiesced recovery procedure. Old-process compatibility evidence remains limited to its actual root-run/read scenarios.

## Evidence

- [Feature-014 tasks](../../specs/014-workflow-primitives/tasks.md), desired-slice versus full-release ordering and T014-042's measured limitations.
- [Workflow runbook](../runbooks/workflow-primitives.md), supported public subsets and migration/reader constraints.
- [Integrated gate record](../../specs/016-payload-classification/release-gates-2026-10-25.md), exact named local gates and Linux diagnosis; it does not substitute for complete green CI.
- [Prior-release compatibility](../../tests/integration/payload-classification/compatibility-report.md), immutable v0.1.0 processes/Studio and the scope of old-reader proof.
- [Release notes](../releases/0.2.0.md) and [release runbook](../runbooks/release.md), actual artifact, migration and publication boundaries.
- PRs [45](https://github.com/ortisun/sagat-octopus-orchestrator/pull/45), [47](https://github.com/ortisun/sagat-octopus-orchestrator/pull/47) and [50](https://github.com/ortisun/sagat-octopus-orchestrator/pull/50). Merge and publication require their actual integrated-head gates and the release-candidate evidence; links alone are not passing results.
