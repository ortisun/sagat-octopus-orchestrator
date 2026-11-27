# Targeted re-read 1 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 4edac72bf227158a20786a0acbe2d677ac0cc26f
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`correctness-full.md`, at 1e283510), not a full review. The reviewer read files only and ran
nothing; the PostgreSQL points come from its reading of `postgres.c` and `xact.c`.

Full round: Majors 3, 4, 5, 6, 7, 8, 10, 11, 12 resolved; Majors 1, 2, 9 partly (below); Nits 1 to 8 resolved; Nit 9
moot.

## Major

A. An error to `COMMIT` can come from a commit that was applied: PostgreSQL sends the error before the abort turns into
   a `PANIC`, and sqlx returns it before ReadyForQuery. Fix: only an error followed by an idle ReadyForQuery is a
   rollback.
B. A confirmation that fails before `COMMIT` is never retried, so the gate can stay closed for good. Fix: a level-
   triggered task with a capped backoff, counted in the budget.
C. `migrate` and the exempt commands cannot run: the identity function is missing on a fresh or 0.2.x database, and
   read-only sessions refuse DDL and a concurrent index. Fix: a read-write profile for `migrate`.
D. The superseded set does not stop connections to a superseded server (the confirmed identity stays set; an unknown
   outcome supersedes the current server; a stale null token supersedes a fresh confirmation). Fix: observation tickets;
   move the confirmed identity to the superseded set; an unknown outcome only sets the reason.
1. (Full-round Major 1, partly.) Callers on `&mut *tx` hide a statement's error from `WriteTx`, and sqlx shows `COMMIT`
   answered `ROLLBACK` as success.
2. (Full-round Major 2, partly.) How test fixtures confirm.
3. (Full-round Major 9, partly.) Fault (3) needs 60 + 300 + 30 = 390 s.

## Nit

1. The lease bound is 19 s less a request's latency and the clocks; 90 s uses two intervals where one is needed.
2. The lost-lease pass's 15 s comes from the model before retries.
3. Success criterion 048-003's drill is Task 048-14.
4. ADR-0130's decision 4 still names classes.
5. The confirmation ignores its own read's flags.
6. The admission rule contradicts the registry's admission.

Counts: Blocker 0, Major 7, Nit 6

All thirteen were fixed at `70822f28`.
