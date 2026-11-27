# Security re-read of the feature 025 fixes and the round 3 decisions (2026-11-27)

Doc: specs/025-ai-agents-mcp/
Reviewed-commit: e94c7e457a4625d609d4f97f0d5e10ec1255506f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative. A short re-read of the
fixes made after `2026-11-27-security.md` and of Decisions 025-017 to 025-019, under the approval rule of Owner decision
2026-11-25-2.

Counts: Blocker 0, Major 4, Nit 7

## Previous review

The previous report's header says "Major 9, Nit 11", but it lists 21 items: 9 Majors and 12 Nits (10 to 21). All 21 are covered below.

1. Model-written content leaving through outputs below the transcript level: fixed. spec.md:682-697, research.md:178, data-model.md:160-163, contracts/ai-api.openapi.yaml:131-134. One residual wording overclaim is new finding 8.
2. Unattended writes only one level deep: fixed. spec.md:565-572, 593-597, 769-777, data-model.md:153-166. The `mcp` part is now governed by Decision 025-018, which has its own gap (finding 2).
3. Blind approval: partly fixed. The default and the `operation` / database `execute` cases are fixed (spec.md:635-658, plan.md:230, contracts/errors.md:29-31). A declared narrowing reopens it; repeated as finding 1.
4. Admission not checked against what publication approved: fixed for ceilings, allowlist and snapshot pin. spec.md:447-460, data-model.md:74-75, contracts/mcp-client.md:39-50, contracts/agent-runner-protocol.md:85-88, quickstart.md:99, tasks.md:666-677. Tool classifications were left out (finding 2).
5. Credentials re-sent on a GET redirect: fixed. spec.md:735-738, contracts/agent-runner-protocol.md:239-243, contracts/mcp-client.md:85, quickstart.md:103.
6. `ResolveSecrets` open for managed leases: fixed. spec.md:489-493, contracts/agent-runner-protocol.md:55-59, data-model.md:116-120.
7. Receipts keyed by an undefined caller: fixed. spec.md:803-811, data-model.md:84-85, contracts/mcp-server.md:53, 89, 118.
8. Model- and provider-written strings on run-read surfaces: fixed. spec.md:574-584, 717-731, data-model.md:196-203, contracts/agent-stream-events.md:34-36, contracts/model-turn.schema.json:13, 25-26.
9. Client secrets and signing keys reaching the runner: fixed for the runner. spec.md:461-479, research.md:163, 179, contracts/agent-runner-protocol.md:69-72. The broker route that replaces them has two problems (findings 3 and 4).
10. Third-set wording: partly fixed. It is now stale the other way, because 5.1.0 is adopted in this tree (finding 10).
11. `sagat.ai.` prefix not reserved: fixed. spec.md:551-555, contracts/agent-runner-protocol.md:37-43, tasks.md:640-647.
12. Mint replay and scope by name: fixed. spec.md:788-795, contracts/ai-api.openapi.yaml:158-163, data-model.md:68.
13. Grants and definer rules: fixed for the feature's own tables. data-model.md:227-257. The amended feature-014 table lacks grants (finding 6).
14. Verbatim-only scan: fixed in substance. spec.md:698-711, 981-985, research.md:207. The published limit misses two bounds (finding 7).
15. Recheck after the bounded wait, and batches: fixed. spec.md:821-826, 836-838, contracts/mcp-server.md:25, 115.
16. Remote callers draining the AI budget: fixed. spec.md:836-842, plan.md:283-284, data-model.md:82.
17. Database `execute` with `approval: "none"`: fixed. spec.md:563-565.
18. Operations with no permission: fixed. contracts/ai-api.openapi.yaml:75, 125, 145, plan.md:231-233.
19. Whose reveal grant is rechecked: fixed. spec.md:508-514, plan.md:226.
20. Source of the runner's egress policy: fixed. spec.md:739-742, contracts/agent-runner-protocol.md:244-246.
21. Payload endpoints and agent payloads: fixed. spec.md:722-725, contracts/ai-api.openapi.yaml:188-190.

## New findings

1. Major — Narrowed `decisionFields` give a blind approval that still counts as attended, so the version can be exposed over MCP (previous finding 3, reopened)
   Where: spec.md:644-647, 769-772; research.md:181; contracts/agent-node.schema.json:586; data-model.md:153-159; contracts/ai-api.openapi.yaml:135-138; quickstart.md:101
   What: The quickstart's own `issue_refund` activity tool can carry `decisionFields: []`, and narrowing is allowed for any tool that "may run with `approval: \"none\"`". The unattended flag counts only `approval: "none"`. So a version whose refund tool every approver approves blind can still be exposed over MCP or a public link.
   - Scenario: a remote caller injects text into the run input. The model requests `issue_refund` with an attacker's account. The `operator` sees every field masked, the view says `decidable: true`, the operator approves and the refund runs.
   Why: The reason narrowing is allowed ("the author could have declared it unattended") does not hold on remote surfaces, where unattended is refused. This is the effect Decision 1296 forbids, plus a click, and it is recorded as a human approval.
   Correction: Count a write tool whose `decisionFields` leave out any input field toward `agent_unattended_writes`, transitively. Refuse it at exposure save, at call time, for public links and at child admission under `remote_root` (`UNATTENDED_WRITE_TOOL`). Add a quickstart row for this case.

2. Major — Reclassifying an MCP tool as `write` never reaches published versions
   Where: tasks.md:734-735; contracts/mcp-client.md:46-50; spec.md:454-459, 566-569; research.md:180; data-model.md:58
   What: The pin rechecks ceilings and the allowlist against the current revision, but not `toolClassifications`. Task 025-102 says outright that "a classification change needs a new revision and a republication to take effect".
   - Scenario: an administrator classifies `sync_records` as `read` in revision 1. An author publishes it as `effect: "read"` with no approval, and the workflow is exposed over MCP. The administrator learns the tool deletes data and saves revision 2 with `write`.
   - Every run, remote ones included, keeps calling the tool without approval. It is also retried on timeout as a `read` tool, which repeats the effect.
   Why: This contradicts "Restrictions saved later still apply at once ... A later revision never widens" (spec.md:454-459) and the reason given for Decision 025-018.
   Correction: At admission, in both `ResolveMcpCall` phases and in the exposure check, accept `read` without a policy only when both the pinned and the current revision classify the tool `read`. Otherwise refuse admission (`AI_DESTINATION_RESTRICTED`) or the call before dispatch. Add the case to Task 025-097 and Task 025-102.

3. Major — The AWS broker route does not fit the text of the third declared set
   Where: docs/constitution.md:223-230, 485; research.md:179; spec.md:474-479; contracts/agent-runner-protocol.md:95-102; docs/owner-decisions.md:28
   What: 5.1.0, adopted in this tree, admits for a declared resource only:
   - its non-secret settings;
   - "for an OAuth credential, only its current access token";
   - any other slot-bound secret, but only as a first-set declared secret.

   The Entra grant fits. An AWS key pair is not an OAuth credential, and a Bedrock short-term API key is a SigV4-presigned request the broker derives, not a slot-bound secret. Decision 025-017 calls it "a bearer access token in that set's sense". That is a delegate's reading of a security exception, recorded as delegate row OD-2026-11-27-401. The 5.1.0 adoption text requires every finding that touches a security exception to be "fixed or put to the owner".
   Why: Secret delivery would rest on a reading the constitution does not state.
   Correction: Put the reading to the owner, or amend Principle V so it admits a broker-derived bearer credential with a stated expiry of at most one hour and never a signing key. Gate Task 025-101 on that row.

4. Major — The resource authorities of the third set are not checked, and which declared set a broker token falls under is unclear
   Where: docs/constitution.md:227-233; plan.md:224-225, 238; contracts/agent-runner-protocol.md:61-76, 100-101; spec.md:479-487; tasks.md:703; specs/021-integration-resources-triggers/plan.md:161-162
   What: Since 5.1.0, a secret bound by a declared resource's slot reaches an executor "under that set's checks and the resource's authorities". Those are the publisher's and the initiator's permission to use the resource and to cause effects through it. The set checks only `secret:use` and `ai:use`. It never says which permission is "use the resource" for the `llm_*` and `mcp_server` types (feature 021 defines `resource:use` and `resource:effect`), nor which is "cause effects" for `write` MCP tools.
   - Scenario: a developer holding `ai:use` and `secret:use`, but not `resource:use` on provider P, pins P and the credential is delivered.
   - The broker token is also unplaced. agent-runner-protocol.md returns it in the `bearer` slot inside a section titled "first declared set", while Task 025-100 says "third declared set". In spec.md:479, "Their disclosure fits ... first declared set" now reads as referring to the broker modes.
   Correction: Require the publisher's and the initiator's `resource:use` on every pinned provider and MCP server, and `resource:effect` for `write` MCP tools, at publication, admission and both resolve phases. Alternatively, record a decision that maps `ai:use` to those authorities. Split credential delivery into a first-set path (static keys) and a third-set path (broker token), each with its own checks.

5. Nit — Tool classifications are keyed by name only
   Where: data-model.md:58; contracts/ai-api.openapi.yaml:308; research.md:180
   What: An administrator classifies `get_record` as `read`. The server later changes that tool's schema and description to a destructive one and states no annotations, so no mismatch is audited. A new publication that pins the new snapshot under the same revision accepts `read` without approval.
   Correction: Store, with each classification, the description and input-schema digests it was given for. Treat a pinned snapshot whose digests differ as unclassified, that is `write`.

6. Nit — No grants for the escalation columns of `approval_requests`
   Where: data-model.md:129-142, 227-250; crates/infra-postgres/migrations/20261101000000_approval_progress.up.sql:167-169 (verified)
   What: `sagat_app` has column-level UPDATE on 7 columns only. The sweeper's writes to `escalated_at`, `context_checked_at` and `no_decider_reason` fail under the runtime role, so escalation and the "never unnoticed" surface silently stop.
   Correction: Add `GRANT UPDATE (escalated_at, context_checked_at, no_decider_reason)` in the Task 025-032 migration, list the table in data-model.md section 8, and cover it in the runtime-role sweep.

7. Nit — The published scan limit (Gap 025-004) leaves out two bounds
   Where: research.md:207; spec.md:698-711, 981-985; docs/owner-decisions.md:15; crates/application/src/worker/secrets.rs:225, 238-240, 252-254 (verified)
   What:
   - A secret value that is not UTF-8 is never looked for, and an output that is not UTF-8 is not scanned at all.
   - The base64 and hexadecimal forms apply only to "the values their attempt received". A `child` tool call's attempt receives none, so secrets of the child's activities reach the transcript after only the verbatim and JSON-escaped check at their own step. The same holds for earlier steps whose output enters through `context`, and only if that step declares secrets.

   The accepted text says base64 and hexadecimal are detected, without that scope.
   Correction: State both bounds in Requirement 025-020, Gap 025-004 and the runbook text of Task 025-069. Add a base64 canary through a child tool as a known gap in Task 025-042.

8. Nit — The inherited-level claim says more than the rule does
   Where: spec.md:696-697, 991-995; checklists/requirements.md:109-110; docs/adr/ADR-0059-agent-steps-are-engine-driven-loops-of-durable-derived-steps.md:121-122
   What: An activity that returns an agent's `summary` creates a new value under its own carrier. So `public` on that activity's output reaches an MCP exposure, yet NFR-025-004 says "never ... in any output carrier they reach".
   Correction: Limit the claim to expressions and child outputs, and add activity and script pass-through to the threat model.

9. Nit — The redacted view leaks structure, and pointer semantics are undefined
   Where: contracts/ai-api.openapi.yaml:394-397; contracts/agent-node.schema.json:577-586; spec.md:640-648
   What:
   - Paths of masked nodes such as `/lines/0` to `/lines/n` disclose array length and which optional fields are present, which "never its length" is meant to prevent.
   - It is not defined whether an object or array pointer covers its descendants, or how array items are written.
   - It is not stated that the eligibility answer comes before `APPROVAL_INPUT_WITHHELD`, so an ineligible caller could learn the request's nature.

   Correction: Mask an array or optional member as one node at its closest always-present ancestor. Define pointers as covering every descendant, with array items written `/-`. State that feature 014's eligibility refusal comes first.

10. Nit — Status text is stale or contradictory
    Where: spec.md:21-22, 310; plan.md:73, 405; research.md:145, 223; contracts/ai-api.openapi.yaml:81; contracts/mcp-client.md:69; contracts/errors.md:35; tasks.md:446, 472, 763-764; checklists/requirements.md:92-93; docs/adr/ADR-0059-agent-steps-are-engine-driven-loops-of-durable-derived-steps.md:98
    What: These lines still wait for 5.1.0 to merge. docs/constitution.md:483 shows 5.1.0 adopted, and spec.md:59, contracts/agent-runner-protocol.md:93 and ADR-0060:122 say it merged. research.md:223 still calls Owner question 025-001 "the owner's, not a delegate's" beside a delegate decision.
    Correction: Reword all of these to match the adopted state and the delegate decision.

11. Nit — (Suspicion, not checked against AWS.) Task 025-101 asserts something the AWS key format may contradict
    Where: tasks.md:717-718
    What: An Amazon Bedrock short-term API key is a base64-encoded presigned URL that, I believe, carries `X-Amz-Security-Token`. If so, the asserted "neither ... nor a session token" cannot pass. This is not a security exposure, because the session token is useless without the secret key.
    Check: base64-decode a key minted by `aws-bedrock-token-generator` (after the `bedrock-api-key-` prefix) and look for `X-Amz-Security-Token`. If present, drop "nor a session token" from the task.
