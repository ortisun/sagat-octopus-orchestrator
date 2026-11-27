# Targeted re-read 3 of design 048: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: e50e68867ab805a72dfc24c4e3e9144094960efc
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read
of the fixes of re-read 2, not a full review.

Re-read 2: Majors 1 and 3 and Nits 2, 3, 4 resolved; Major 2 partly (Major 1); Nits 1 and 5 partly.

## Major

1. `DbHandle`'s port has no named trait, crate or methods; "the port" names two things; `new(PgPool)` could reach
   production and silently never close the gate; Task 048-10 does not pass the real port in `apps/`.
2. Task 048-13's expressions cannot fire at zero (`count` over nothing returns nothing); the quorum mode's metric; no
   scrape of the Patroni members or CloudNativePG; no CloudNativePG expression.

## Nit

1. The savepoint guard's interface and its drop.
2. "The chart's migrate run" does not exist; `migrate` uses the owner's role.
3. The contract test has no list of rule files; each alert needs a runbook section; each file its own `promtool` mount.
4. Task 048-6 before Task 048-7 in the order; the table names one copy of `confirmed_abort`.

Counts: Blocker 0, Major 2, Nit 4

All six were fixed at `7a51e09a`.
