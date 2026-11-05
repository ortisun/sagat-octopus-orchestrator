# Delegated decisions for release 0.3.0 — 2026-11-04

Refs: T014-014, T014-015, T014-040, T014-042, T014-043. Scope: three expert opinions, read-only.
No build, test or gate was run, so this closes no task.

The owner delegated the open decisions of the ledger: "Tome as decisoes por mim. Que os melhores
especialistas em arquitetura, dados, banco de dados, decidam." The assistant put each question to an
independent reviewer, gave them the question and the files and not its own leaning, and decided from
their opinions. The decisions are the rows OD-2026-11-04-3 to OD-2026-11-04-9 of
[owner-decisions.md](../../../../docs/owner-decisions.md). They are `Decided by: delegate`: reversible,
and the owner confirms one by changing that cell to `owner`.

| Opinion | Reviewer | Question |
| --- | --- | --- |
| [architecture-review.md](architecture-review.md) | `architecture-reviewer` | Accept ADR-0075? Is gap G721 closed by it? Is accepting separable from activating? |
| [database-review.md](database-review.md) | `correctness-reviewer`, as a PostgreSQL and transaction expert | Is the lock protocol of ADR-0075 sound in the code? |
| [data-review.md](data-review.md) | `general-purpose`, as a data-platform and performance expert | O460, O464 and O463 ceilings; how T014-042 can be closed honestly |

Where the decisions differ from an opinion:

- ADR-0075 takes the amendments of both the architecture and the database reviews. They do not
  conflict. The database review's first amendment (do not lock the group before knowing the member is
  the holder) is stricter than the current code, so the code is out of line with the amended ADR; that
  is activation condition (j), not a defect of the accepted guard, which stays off.
- The data review made index-only discovery unconditional before 0.3.0. The delegate made it
  conditional on the acceptance run failing the idle-tick thresholds, because the reviewer says the
  cost on large definitions is inferred from the SQL and not measured, and the acceptance run
  measures it (OD-2026-11-04-6).
- The data review proposed running a flat 10,000 items by 10 instances workload in place of the
  nested 100,000-instance one while nested scopes are closed. The delegate keeps the original
  acceptance criteria, as the owner decided on 2026-10-26, and makes the flat run extra evidence
  (OD-2026-11-04-7).

The reviewers' figures were taken from the performance report and its raw fragments. Three were
re-read by the assistant (see the header of the data review); every other figure and every
`path:line` is the reviewer's reading and is unverified.
