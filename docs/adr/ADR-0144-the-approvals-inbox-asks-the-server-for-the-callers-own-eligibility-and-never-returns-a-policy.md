# ADR-0144: The approvals inbox asks the server which approvals the caller may decide now, through the same rule that decides; it never returns a policy

**Status:** Proposed
**Decided by:** agent proposal (design 050, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 050 is complete
**Refines / supersedes:** refines the approval decisions of feature 014 (Requirement 014-007, Requirement 014-008: eligibility by subjects and roles, the initiator excluded by default) and `listRunApprovals`, which "deliberately omits the eligibility policy, initiating identity and decision actor"; supersedes none
**Enforced by:** planned in design 050 and not existing yet: the property test `inbox_eligibility_equals_the_decision_rule` (for generated policies, subjects, roles and initiators, an approval is listed as decidable exactly when `ApprovalRequest::decide` would accept that principal's decision); `tests/contract/workflow-studio/approvals_inbox.rs` (the response schema has no field that names a subject or a role of a policy, the initiator or a decider); the cross-tenant battery entry for the new route

## Context

A person who must approve a step today has to know which run waits for them. The server can list the approvals of one
run (`GET /runs/{runId}/approvals`) and decide one by id (`POST /approvals/{approvalId}/decisions`); there is no list
across runs. Temporal's Web UI has no inbox either (a person signals the workflow); Windmill sends an approval link per
suspended step. An inbox is the place a person goes to find what waits for them, so it must answer "which approvals may
I decide", and it must not become a way to learn who else may decide, or who started the run: the per-run list omits
those on purpose.

The eligibility rule lives in `crates/domain/src/execution/approval.rs` (`authorize_decision`: the caller holds
`approval:decide`, is named by subject or role in the policy snapshot, and is not the initiator when the policy
excludes it). The snapshot is stored with each request in `approval_requests` (`policy_subjects`, `policy_roles`,
`exclude_initiator`, `initiator`).

## Decision

1. A new operation `GET /namespaces/{namespaceId}/approvals` MUST list the namespace's approval requests, filtered by
   status (default `PENDING`), ordered by deadline then request id, with a bounded opaque cursor. It needs `run:read`
   and `approval:read`.
2. Each item MUST carry `youMayDecide`, true exactly when a decision by the caller would pass the same rule the
   decision applies at commit (Requirement 014-008), evaluated on the caller's current membership. The filter
   `decidable=true` MUST return only such items. The SQL predicate that filters MUST be derived from the domain rule
   and held to it by a property test; no second rule is written.
3. An item MUST NOT carry the policy's subjects or roles, the initiator, the decider or the reason code. It MAY carry
   what `listRunApprovals` carries, the workflow's identifier and name and the run's business identifier, and, when
   feature 026 has evaluated one, a flag that a summary exists (the summary itself is read through feature 026's
   route under its own disclosure rule).
4. When feature 029 is enabled, the listing MUST omit approvals of runs the caller cannot see (the evaluator's
   `visibleFolders`), like every other listing; `youMayDecide` stays the decision rule.
5. A page MUST read a bounded number of pending rows; when the bound is reached before the page is full, the answer
   returns what it found and a cursor to continue, never an error and never an unbounded scan.

## Considered options

1. **A server-side list with the caller's own eligibility (chosen).** One rule for listing and deciding, nothing of
   the policy leaves the server, and the inbox is exact.
2. **The Studio walks the runs and asks each one.** No new route. Rejected: one request per run, no ordering by
   deadline, and the Studio would still not know eligibility without trying a decision.
3. **Return the policy so the Studio computes eligibility.** Simplest server. Rejected: it discloses who may approve
   and who started the run, which feature 014 chose not to disclose.
4. **Push notifications only (feature 026 channels).** Rejected as the only path: a message can be missed, and a
   person needs one place that is right now. Feature 026's notifications and decision links complement the inbox.

## Consequences

- One migration adds a partial index on pending approvals by namespace and deadline; the decision path is unchanged.
- A person sees their own eligibility for every pending approval of a namespace they can read, which they could learn
  anyway by attempting a decision; nobody learns anything about another person.
- Rollback: the route is additive; removing it removes the inbox page.

## Evidence

The rule and the stored snapshot are read from `crates/domain/src/execution/approval.rs` and
`crates/infra-postgres/migrations/20261101000000_approval_progress.up.sql` on `develop` at 2026-11-24. The planned
tests are listed under **Enforced by**; none exists yet.
