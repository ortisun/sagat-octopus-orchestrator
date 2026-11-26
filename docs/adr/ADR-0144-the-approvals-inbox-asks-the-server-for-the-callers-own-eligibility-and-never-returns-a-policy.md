# ADR-0144: The approvals inbox asks the server which approvals the caller may decide now, through the same rule that decides; it never returns a policy

**Status:** Proposed
**Decided by:** agent proposal (design 050, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 050 is complete
**Refines / supersedes:** refines the approval decisions of feature 014 (Requirement 014-007, Requirement 014-008: eligibility by subjects and roles, the initiator excluded by default) and `listRunApprovals`, which "deliberately omits the eligibility policy, initiating identity and decision actor", and Architecture decision 0018 decision 4 (who may approve a publication request), on which `PublicationRequest::may_decide` is built; supersedes none
**Enforced by:** planned in design 050 and not existing yet: the domain property test `may_decide_equals_decide` (for generated policies, subjects, roles and initiators, over pending requests with fresh keys, `may_decide` is true exactly when `ApprovalRequest::decide` would accept that principal's decision, and the same for `PublicationRequest` over every state); `tests/contract/workflow-studio/rest/approvals_inbox.rs (in `contract_rest_workflow_studio`)` (the response schema has no field that names a subject or a role of a policy, the initiator or a decider; the publication inbox item has exactly the fields of decision 6); the cross-tenant battery entry for the new route

## Context

A person who must approve a step today has to know which run waits for them. The server can list the approvals of one
run (`GET /runs/{runId}/approvals`) and decide one by id (`POST /approvals/{approvalId}/decisions`); there is no list
across runs. Temporal's Web UI has no inbox either (a person signals the workflow); Windmill sends an approval link per
suspended step. An inbox is the place a person goes to find what waits for them, so it must answer "which approvals may
I decide". It must add no disclosure: the policy of an approval is already readable in the workflow version
(`definition:read`) and the initiator and deciders are actors of the journal (`run:read`), so withholding them from
the item is not a confidentiality boundary; the inbox keeps the per-run list's field set and adds only the caller's
own eligibility.

The eligibility rule lives in `crates/domain/src/execution/approval.rs` (`authorize_decision`: the caller holds
`approval:decide`, is named by subject or role in the policy snapshot, and is not the initiator when the policy
excludes it). The snapshot is stored with each request in `approval_requests` (`policy_subjects`, `policy_roles`,
`exclude_initiator`, `initiator`).

## Decision

1. A new operation `GET /namespaces/{namespaceId}/approvals` MUST list the namespace's pending approval requests,
   ordered by deadline then request id, with a bounded opaque cursor and no filter by workflow or by run (the per-run
   and per-workflow lists already serve those reads). It needs `run:read` and `approval:read`. Settled approvals stay
   on the per-run list and in the audit trail.
2. Each item MUST carry `youMayDecide`, computed by `ApprovalRequest::may_decide(&Principal)`, extracted
   from the decision's own check and called by `decide` with the same inputs, on the caller's principal built from the
   current membership as the decision builds it; an approval whose run has ended is left out of
   the list by a run-status predicate of the scan (a run's end cancels its pending approvals, so such a row is a
   defect the probe test finds); the decision's adapter refuses an ended run only after authorization, so the one
   changed answer is that an ineligible caller on an ended run gets the eligibility refusal and its denial audit
   record instead of a retryable `503`. The flag is a point-in-time hint; the decision re-checks. The filter `decidable=true` MUST return only
   such items. The items of the existing per-run list (`GET /runs/{runId}/approvals`) MUST carry the same
   `youMayDecide`, computed by the use case as `may_decide` for a `PENDING` request of a run that has not ended and
   false otherwise (that list also returns settled requests), so the run view needs no second route. The adapter returns bounded pages of pending rows with their policy snapshots; the use case filters;
   no SQL predicate repeats the rule, and the snapshots never leave the use case.
3. An item MUST NOT carry the policy's subjects or roles, the initiator, the decider or the reason code. It MAY carry
   what `listRunApprovals` carries, the workflow's identifier and name and the run's business identifier; feature 026
   adds its summary flag additively.
4. When feature 029 is enabled, the listing MUST omit approvals of runs the caller cannot see (the evaluator's
   `visibleFolders`), like every other listing; the predicate applies to the rows a page examines, and the cursor is
   sealed (decision 5), so a hidden row's key never reaches the caller; the count of hidden pending rows in a page's
   window is the one aggregate disclosed, as feature 029 allows (Requirement 029-017); `youMayDecide` stays the decision rule.
5. A page MUST examine a bounded number of pending rows, counted before any data or access predicate; when the bound
   is reached before the page is full, the answer returns what it found and a cursor to continue, never an error and
   never an unbounded scan. When the page fills, the cursor is the key of its last item; when it does not, the key of
   the last row examined; it is null only when no row remains. The cursor is sealed with AES-256-GCM-SIV under a purpose key
   derived by HKDF-SHA256 from the deployment's key ring (labels `approval-inbox-cursor` and
   `publication-inbox-cursor`, the key id and the tenant as `info`), with the tenant, the namespace and the caller as associated data, so it reveals no row
   key and opens only for its caller; without a key ring the routes answer `503` and never return an unsealed cursor.
6. The same holds for a namespace list of the open publication requests that still need an approval under the current
   effective policy (`PENDING`, and `APPROVED` with fewer approvals than now required), ordered by request time, with
   `PublicationRequest::may_decide` (the request open, not the requester, not the last editor, not a person who already
   decided) and `publication:approve`. Its items carry the request's identifier, workflow, kind, state, draft revision,
   requester, request time, the approvals recorded and the approvals the current policy requires, and `youMayDecide`;
   they MUST NOT carry the decisions or the last editor, which stay on the per-workflow list under the same
   `definition:read`. The use case resolves `approvalsRequiredNow` once per page by the same pure policy function that
   publication's recount uses; the store returns raw rows. Decision 3 concerns approval policies and does not forbid naming the requester, whom the
   per-workflow list already names. The items of `GET /workflows/{id}/publication-requests` gain the same
   `youMayDecide`.

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

- Two migrations add partial indexes, on pending approvals by namespace and deadline and on open publication requests by
  namespace and request time. The decision path keeps its rule and its
  answers; its ended-run refusal moves after authorization.
- A person sees their own eligibility for every pending approval of a namespace they can read, which they could learn
  anyway by attempting a decision; nobody learns anything about another person.
- Rollback: the route is additive; removing it removes the inbox page.

## Evidence

The rule and the stored snapshot are read from `crates/domain/src/execution/approval.rs` and
`crates/infra-postgres/migrations/20261101000000_approval_progress.up.sql` on `develop` at 2026-11-24. The planned
tests are listed under **Enforced by**; none exists yet.
