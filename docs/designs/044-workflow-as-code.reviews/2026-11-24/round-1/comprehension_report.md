# egm-review Stage 6 (Comprehension Test), round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 44133eb732bb25f93904c0d144a7b5af1fa26d9f
Reviewer: egm-comprehension, FeatureRange 1-1156, DetailedRange 1157-1369
Note: the paraphrase is kept as the reviewer wrote it, lightly shortened; every finding and severity is kept, condensed.

## What this proposes

From release 0.3.0, a workflow can be written as an ordinary function in any of six SDK languages, alongside the
existing graph. The function runs in the tenant's own worker as a "decider". Each "workflow task" is a lease of a
reserved step `~workflow`. In it, the worker runs the function against the run's history until it blocks, then returns a
list of commands. Each command that does work is an existing IR node fragment, which the server parses and commits with
the journal, steps, timers and outbox in one transaction. Replay must be deterministic, so the design adds a determinism
runtime per language, a seeded random generator and a recorded clock, a hash chain of accepted commands kept on the
server, and runs that are put on "hold" instead of failing. Each run pins one code version, and so one code digest. It
changes code only through an operator's "move", which applies after a worker of the target digest has replayed the run
without a mismatch. Signals, asynchronous updates, queries the engine answers, sagas planned by the engine, children,
continue-as-new, sticky caches and eager dispatch are all defined on top of this.

## How the existing system works, per this document

Today a workflow is a declarative IR graph, built in the Studio or by an SDK builder, and a Rust engine interprets it.
SDKs are explicitly not workflow runtimes. The engine leases steps to workers over a gRPC protocol with leases, fences,
attempts and generations. Each step completion is committed in one PostgreSQL transaction (`PgRunTx::persist`) with the
next work and an outbox, and the run's history is an append-only journal (`run_events`) with a per-run sequence. Other
features supply compensation as derived steps (ADR-0010) and rules for uncertain effects (feature 037), declared state
with queries the engine answers and continue-as-new chains (feature 014), resets and batch operations (feature 018),
immutable published versions with a digest, and an SDK conformance suite. A graph run refuses a signal that nothing is
waiting for, and graph updates are synchronous CEL.

## Findings

1. Major (128-131, 694-695, 989-990, 1214): "binding" is never defined; define it and link the mechanism.
2. Major (many lines): features 003 to 043 and ADR-0003, 0010, 0018, 0020, 0050, 0082, 0089 are not linked; add a
   references table.
3. Major (71, 150, 376, 605, 717, 725, 776, 814, 829, 866-869, 1018): "sequence" means the journal sequence, the command
   number and the visible position; name each.
4. Major (133-134, 596, 716, 723-730, 984): the sticky lease's "events after the last completion" drops the events
   committed between the last start and the last completion; state the window once.
5. Major (141-142, 270-271, 770-792): the statement budget does not match the list (up to 13 or 14).
6. Major (229-231, 714, 788, 866-870): `run_version` undefined; if renewals or hints advance it, a move never applies;
   no outcome after three re-checks.
7. Major (156-160, 503-504, 530-531, 567, 793, 828-835, 906-907, 1000): failure and hold causes are not classified;
   `COMMAND_INVALID` missing from Requirement 044-008; retry scope differs between Requirement 044-008 and 6.3.
8. Major (242-245, 828-841): no exit from a `HISTORY_LIMIT` hold.
9. Major (545-546, 840, 1004, 1235-1238): a reset gets a new seed but replays a prefix that used the old one.
10. Major (123-126, 347-349, 665-676, 684-687): the manifest and how each language produces the digest before
    publication are undefined; Python's digest depends on what the worker loaded.
11. Nit: "capability envelope" not in Terms.
12. Nit: attempt and generation not distinguished.
13. Nit: "activation" used for events and for a period; "yield" undefined.
14. Nit: "chain" means the command chain and the continue-as-new chain.
15. Nit: unexplained references (AGENTS rule 3, Delivery Constraints, the migration checklist, design 043's table, the
    four lock files, the runtime role, the two heavy builds, "this machine").
16. Nit: the reference hosts are not specified.
17. Nit: "the register" means the design register and the protocol register.
18. Nit: "the reserved activity name marks the step as not an activity" reads as a contradiction.
19. Nit: `sticky_until` defined from a future moment.
20. Nit: no request field for the eager workflow task offer.
21. Nit: a signal over the 4 MiB retained bytes: the refusal is not stated.
22. Nit: "strongest isolation of the six" against the table.
23. Nit: gaps cited by number only.
24. Nit: no task owns the server side of the history export route.

Counts: Blocker 0, Major 10, Nit 14
