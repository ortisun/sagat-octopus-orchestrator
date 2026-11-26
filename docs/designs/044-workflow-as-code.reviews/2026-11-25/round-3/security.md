# Security review of design 044, round 3 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 2249e90100039f5c012b6217e4e5bb454ace1296
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer judged round
1's findings 1, 2, 4, 5, 8, 10, 12, 14, 16, 18, 20, 21, 22 and 23 closed, 3, 6, 7, 9, 11, 13, 15 and 17 partly closed,
Nit 19 not closed; it found no cross-tenant path, no new egress and no open metric label.

## Major

1. The reserved names meet four binding checks that know only activity patterns (`worker.rs:799`, `worker.rs:313-320`,
   `worker/scoped_claims.rs:108`, `ensure_activity_bound` at `worker.rs:2750-2756`, which returns `Ok(())` for a node
   that is not an activity); a `*` binding matches the reserved names. Check them against `workflowTypes`, close
   sessions on revocation, fail closed on an unknown kind.
2. Overlapping envelope entries have no precedence; a prefix entry with secrets sends them to any activity code invents
   under the prefix. Refuse overlaps or fix longest match; refuse secrets on prefix entries.
3. Script entries are digests only, so a script's secrets, schemas and compensation have no publisher source; make
   script entries objects; the schema field is `artifactDigest`.
4. A wait-for-event command chooses its event type and look-back at run time; add `envelope.events`.
5. Nothing ties an approval decision to the effect it guards; an entry field that requires approval before release, or
   state that approvals in code are advisory.
6. A replayed update `POST` returns another principal's receipt and its 409 confirms guesses; apply the read rule to
   replays.
7. The headroom is undefined after round 2; updates have no headroom; rejected updates can push a run to the limit.
8. Update and worker-query arguments have no schema or size bound; set-state is not validated against the state schema.
9. The overlay spec keeps literal business values (correlation key, approval subject, business identifier) that the
   Studio draws for viewers; store them as classified payloads.
10. Every workflow lease receives the whole unredacted history, more than the export; record this disclosure class,
    say so on the binding screen, audit `ReadHistory`.
11. The namespace pause has no permission and does not stop live sessions; enforce it at claim and completion.
12. A batch move does not re-authorize its creator per item.

## Nit

13. The move function's grants, schema-qualified names, its own check of a replay-check row, a reset of a moved run,
    and `steps.activity` writable by the runtime role.
14. The plain fragment digest in `step.released` and `steps.input_digest` confirms guesses if shown.
15. Gap 044-016's reasoning: outside data does reach Python workflow code; cap keys per object when decoding.
16. "Only a worker that runs that digest" overclaims: the digest is declared.
17. The child `version` comes from the fragment; pin it in the envelope.
18. Target grammars undefined; idempotency after authorization; another tenant's identifiers get the same 404.
19. Audited no-ops can write 256 records per completion; per-principal shares of the namespace bounds.
20. "Fits the target's envelope" undefined; the publisher checked at resolution.
21. `ReadHistory` must not hold the run lock while streaming; `WriteBlob` under a workflow lease undecided.

Counts: Blocker 0, Major 12, Nit 9
