# egm-review, Stage 6 (Comprehension Test) of design 050 (2026-11-25)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: ea803052
Reviewer: egm-review (Stage 6, egm-comprehension)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's paraphrase is kept in full; its findings are condensed by the author to one paragraph each, with every severity, location and fix kept.

Counts: Blocker 0, Major 7, Nit 14

## What the feature does

The server now runs and exposes much more than the Studio can show: switches, approvals, loops and collections, an audit trail, version activation and rollback, namespace settings, event delivery and Go and Java scripts; the Studio misses most of this and still offers two old node kinds (`foreach`, `humanApproval`) that the server refuses. For release 0.3.0 the Studio catches up: it offers every node kind the server can publish, including body graphs for loops, never a refused kind, and keeps a refused node read-only in a draft; it adds a "Waiting for me" inbox of the run approvals and publication requests the person may decide, eligibility answered by the server without revealing anyone else's; a tenant-wide audit log page with a bounded download; and the operator screens Temporal or Windmill users expect (Queries and Updates, loop iterations and switch choices, the run's family, versions, namespace settings, event delivery health with redrive, a command palette, Go and Java built, a journal download). All on Astryx, accepted with a performance run and a bundle budget test.

## How it is built

One registry of node kinds checked by a unit test against the workflow schema, each entry with scopes, fields and handles, the capability it needs and a note on where it publishes; the palette hides kinds whose capability is absent from `GET /version`'s new list, or, for `icebergWrite`, from the database drivers list; the publishable subset is decided only by the server's conversion, shown by live validation, with body problems pointed at `/bodies/{name}/nodes/{index}`; bodies on their own canvas; legacy nodes kept read-only with an "Add an X here" action; the run view groups scoped instances and draws iteration counts and statuses the server computes from recorded outcomes; decisions and Updates use a per-intent key hook; `collectPagesBounded` serves the downloads and the inbox. Server side: two namespace-wide lists (pending approvals, open publication requests) with `youMayDecide` from a domain `may_decide` the decision itself calls, each page bounded with a cursor of the last row scanned, an ended run's approval answering `409 RUN_ENDED`, and `AuditAction::ALL` held to an OpenAPI list. Ordered slices keep parallel Studio branches apart; nothing changes in the IR, the worker protocol or the SDKs.

## Findings

1. **Major. Overview against Technical Plan 3 and Decision 050-009.** The Overview says loops sit "behind runtime switches", which the design later rejects. Fix: say the loops publish in fixed shapes on every server, collection operators behind design 042's switch, lake writes when `lake.enabled`.
2. **Major. Terms, Requirement 050-003, Decision 050-009, Technical Plan 4.** "The codec", the authority of the editor slice, is never defined, and "the codec's reason" and "the server's validation" may or may not be the same. Fix: a Terms entry and one name.
3. **Major. Requirement 050-002, Technical Plan 3.** `workflow.step-destinations` is shipped but nothing consumes it and "step-output destinations" is unexplained. Fix: say what it is, which field reads it and what is hidden without it.
4. **Major. Technical Plan 3.** The instance-registry paragraph needs undefined terms (member, beat, epoch, gate, `Member::decide`), ADR-0082 is not linked, and "registry" also means the Studio's node registry. Fix: define them in two sentences with the link, say "instance registry" every time, and say plainly this work is not design 050's.
5. **Major. Requirement 050-005, Technical Plan 4, CUJ 2.** "Add here" adds a node "linked to" or "linked from" the legacy node's predecessor without saying which predecessor, whether the predecessor's link moves, or how a read-only node is deleted. Fix: spell out the edit with one wording.
6. **Major. Task 050-12 against Decision 050-007 and Requirement 050-019.** The task still says "the built languages from the server" with tests of the rejected server field. Fix: rename and use the requirement's tests.
7. **Major. The plan's introduction against Tasks 050-6, 050-9, 050-10.** The stated lane boundaries are broken (Task 050-6 changes `crates/infra-ir` and `crates/application`; Task 050-9 needs Task 050-4 and adds server projection fields; Task 050-10 needs Tasks 050-4 and 050-5). Fix: list each task's predecessors and assign the server parts.
8. **Nit. Overview.** "Three small things" followed by four; `youMayDecide` unglossed. Fix: "four"; gloss it.
9. **Nit. Throughout.** "IR" is never expanded; most feature and design numbers are neither linked nor glossed; feature 029's access terms are opaque. Fix: expand IR; a short list of referenced features.
10. **Nit. Open Questions, Decisions, Owner decisions needed.** The "delegate" label is promised but not on the decisions; review findings are cited without links; "the coordinating session", "the owner", "the draft" assume history; two formats of owner-decision identifiers. Fix: link the files, label or drop, say who the owner is, "an earlier draft of this design".
11. **Nit. Gaps.** Gap 050-005 before 050-004. Fix: numeric order.
12. **Nit. Technical Plan 4.** "The Studio needs no change" while the notes in `registry.ts` change. Fix: the behaviour needs no change, only the notes.
13. **Nit. Requirements 050-003, 050-004, the `forEach` row.** A field whose capability is absent (the stage list) has no stated handling. Fix: read-only with the reason, kept on save.
14. **Nit. Requirement 050-008, Technical Plan 5, Task 050-9.** "Admitted iterations", drain reasons, reconciliations and the `~c`, `~r` suffixes are unexplained; two counting rules appear only in test names. Fix: define them and state the rules.
15. **Nit. Overview, The Problem, Requirements 050-014 and 050-015, Technical Plan 4.** Chain, chain-addressed, `chainPolicy`, segments, declared state, `expectedStateRevision`, `reapplyOnReset` undefined. Fix: Terms entries linked to feature 014 and ADR-0031.
16. **Nit. Non-functional requirement 050-001, Task 050-4, the Verification Plan.** "The cross-tenant battery", "the runtime role", `SAGAT_TEST_AS_RUNTIME_ROLE=1` unexplained. Fix: gloss them.
17. **Nit. Requirements 050-013 and 050-019, Decision 050-006.** "The Studio may be served from another build (`SAGAT_STUDIO_DIR`)" is never explained. Fix: explain it once.
18. **Nit. CUJ 3.** "Not listed under 'decidable'" suggests a section the page does not have. Fix: "not listed".
19. **Nit. Technical Plans 4 and 6, Requirement 050-011.** "Source activities", "recovery activities" undefined; the basic-graph row refuses "an approval or a loop combined with a collection"; "superseded" and "recounts" unexplained. Fix: define, move the refusal, a clause each.
20. **Nit. The plan.** Task lines say `T050-1` while prose says "Task 050-1". Fix: one form.
21. **Nit. Verification Plan, Task 050-4.** "Registry-derived capabilities" fits neither registry; the cancellation test is a red test in the task but a probe in the plan. Fix: "derived from the start-up switches"; say whether a failure blocks the task.
