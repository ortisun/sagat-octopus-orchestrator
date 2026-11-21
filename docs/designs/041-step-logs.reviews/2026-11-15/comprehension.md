# EGM stage 6 (comprehension test) of design 041 (revision of 2026-11-13)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 879c550b1726870fa56392b1cff20272f114a5ed
Reviewer: egm-comprehension (stage 6 of egm-review)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Target: FeatureRange lines 1-828 and DetailedRange lines 829-1047 (split at `## Detailed Implementation Plan`, line 829)
Standard: none (self-containment test)
GoldfishContext: docs/constitution.md; ADR-0084; ADR-0085; specs/041-step-logs/research.md; the three reports of docs/designs/041-step-logs.reviews/2026-11-13/
Note: one fresh `egm:egm-comprehension` agent, dispatched directly as the skill requires. The paraphrase is kept as written; the findings are condensed.

**What this proposes**

Each of the six SDKs would capture the lines an activity writes through its existing per-attempt logger. Before a line leaves the worker process, the SDK removes the secret values of every attempt running in that process. It then buffers the lines without blocking and sends them in batches through a new unary gRPC call, `AppendStepLogs`, added in worker protocol 1.7. Before storing a batch, the server repeats the checks a worker report gets (permission, role, membership, binding, session and lease) without locking `runs` or `steps`. It also scrubs credential shapes and enforces limits per message, attempt, run and tenant from counters. Batches are stored at most once in three new PostgreSQL tables that have no foreign key to `runs`; the tenant's usage row is taken only at commit, except within 2 MiB of the tenant's limit. Lines are kept for a configurable number of days, purged by a housekeeping pass and can be deleted by an admin. They are read through a list route, a streams route and a live server-sent-events route, which need a new `run:logs:read` permission and, in classified namespaces, the PII reveal grant. Reads are audited at most once per window and shown in a new Studio Logs tab. Script steps get a marker that points to their existing script-execution log, and viewers can no longer read that log.

**How the existing system works, per this document**

What a run did is rebuilt from the append-only `run_events` journal. Platform processes log JSON to stdout, and five SDKs have an activity logger that writes only to the host process; Rust has none. Only script steps have a stored log, `script_executions.log`, which is capped, charged to `tenant_log_bytes` and readable by viewers under `script:read`. Only the worker process holds secret values, and Python's activity logger wraps the logger shared by the whole worker. Leases and completions lock the run row `FOR UPDATE` and have fixed statement budgets that CI checks for equality. Stored bytes are counted by per-row triggers and a commit-time finalizer that locks one usage row per tenant. Heartbeats check permission, binding and membership as well as the lease token, and nothing deletes a run.

The reviewer found both ranges paraphrasable, so neither is a Blocker; the weak points are undefined or overloaded terms and the description of the controls and locks.

## Blocker

None.

## Major

1. FR-041-002 (lines 96-98) promises lines "in the order in which the platform committed" them; section 5 (lines 587-592) and ADR-0084 rule 9 order by (`stored_xid`, `id`), and the id is assigned at step 1, not at commit. "Stored order" in FR-041-010 (line 148) is never defined. *Recommendation:* restate FR-041-002 as transaction-id order, not commit order; define "stored order" by pointing to section 5.
2. At least four controls decide whether capture happens: `SAGAT_STEP_LOGS_ENABLED` (false until T041-10, then true), a per-namespace capture switch, "an option per worker process" (never said to be on/off or the FR-041-003 level), and `StepLogPolicy.enabled` returned once per registration. How they combine, which wins, whether one worker serves several namespaces, and what a mid-session namespace switch does are not said; O041-001 "on by default" sits next to a deployment default of false. FR-041-001, FR-041-003, FR-041-020, O041-001, section 1 (lines 423-425), section 10 (lines 733-735), Open Questions (lines 290-292). *Recommendation:* one table in FR-041-020 of each control, its scope, its default before and after T041-10, and how they combine.
3. "Generation" (lines 94, 108, 412, 526-531, 1020) and "instance path" (lines 94, 413, 526-527, 676-679) are part of every line's identity and of the stream key, but never defined. *Recommendation:* define both at first use in FR-041-002 with a pointer to where the engine assigns them.
4. "Classified" has two meanings: storage classification (`log`, `protected_system` in `storage-classification.json`, Fact 4 line 64, section 3 lines 546-548) and data classification (`x-sagat-classification`, FR-041-013 and FR-041-021, lines 165-167, 201-204). *Recommendation:* always qualify ("storage-classified table", "PII-classified namespace").
5. "Stream" means one attempt's log, the SSE tail, the graph stream `/runs/{runId}/stream` and the "stream caps"; `/logs/stream` and `/logs/streams` differ by one letter; the first meaning is defined only in FR-041-010, after use in FR-041-006 (lines 117-120, 150, 154-163, 400, 526, 608-612). *Recommendation:* define "log stream" and "graph stream" once; rename the tail route (`/logs/tail` or `/logs/follow`).
6. The lock order (lines 514-519) begins with "the namespace secrets lock (shared)", which is not defined and is taken by none of the ten numbered steps; the content and log policy fences are described only as "shared advisory locks". A reader cannot connect the order to the steps T041-13 tests. *Recommendation:* name the step that takes each lock; one sentence on what each fence protects, with file references.
7. "Is 1.7 reachable where 1.6 is not?" (lines 293-295) depends on `protocol_ceiling` and on "scoped support", "database support" and a "log tables" proof that are not defined; the item names no closing task although the list says each is "closed by a named task"; "the sandbox runner keeps offering `1`" is unclear. *Recommendation:* define the store proofs with code references; name the task or move the item into the Technical Plan.
8. "Owner decisions needed" says "None open" (line 299) but also sends the ADR-0084 and ADR-0085 revisions to the owner (lines 319-320, T041-2), whose `Owner-gate:` cites OD-2026-11-13-14, a decision taken before the revision. The reader cannot tell whether the revised ADR text is in force. *Recommendation:* list the acceptance of the revision as open and point T041-2's gate at the pending row.

## Nit

9. FR-041-011 (lines 157-158) cites "the horizon of FR-041-010", which FR-041-010 does not mention.
10. Features 005, 008, 015, 020, design 038, ADR-0014/0015/0023/0027/0054, gaps G69 and G1618, `sdks/SPEC.md` §7.1/§9/§10 and the PII reveal grant (ADR-0033, not linked) are cited by number without a link or gloss (lines 81, 139, 166-167, 181, 262-273, 308, 430, 436, 468, 543, 582, 623-625, 791, 812-813).
11. Roles `runner` and `database-runner`, the sandbox, database and lake runners, "shed before discovery" and "the reserve kept for lease calls" are not explained (lines 164-165, 184, 437-438, 444-445, 459-460).
12. The owner's quotes are in Portuguese without a translation (lines 13-14, 22, 301, 322, 324).
13. CUJ 4 (lines 357-358) promises a note that "the worker did not send any", but a pre-1.7 attempt and an attempt that logged nothing both have no stream row; "ended without its final flush" (lines 157, 187-188, 702) needs the attempt's end, which `final_at` alone does not show.
14. CUJ 6 (lines 371-374) picks "iteration 17", but FR-041-010 has no iteration filter; say it is chosen through the streams list and applied as `stream`.
15. Small unexplained details: "50/1/11 or 47/1/9" (line 61-62), "its three constants" against six figures (lines 216-217), TypeScript's "four methods" against five levels (line 629), Python's "before the identifiers are appended" (line 631).
16. In the plan: "the three checkers" (line 1001), "the four lock files" (T041-20), "the Rust-driven browser journey" (T041-24) and `.claude/skills/migration` (T041-5) are not explained; "follow-up task" (T041-9) is used for a task added inside the plan, while the repository uses the term for work appended after approval.

Counts: Blocker 0, Major 8, Nit 8
