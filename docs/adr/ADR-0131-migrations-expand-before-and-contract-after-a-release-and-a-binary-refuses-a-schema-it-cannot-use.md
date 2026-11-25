# ADR-0131: Migrations expand before a release and contract after it, and a binary refuses a schema it cannot use

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 048), to be accepted under Owner decision 2026-11-24-4 once design 048's review is complete
**Refines / supersedes:** refines [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the registry's gate also closes on a schema mismatch, and its census gates new behaviour); replaces the stopped upgrade of `docs/runbooks/scaling.md` section 3 from release 0.3.0 on; supersedes nothing
**Enforced by:** `scripts/check-migrations.py` and its negative fixtures, the checksum test (Task 048-12), the handshake tests as the runtime role and the architecture check of entry points (Task 048-13), `migrate`'s refusal tests (Task 048-14), `scripts/test-mixed-writers.py` in the release workflow (Task 048-15)

## Context

Upgrades stop every writer today: "mixed old/new writers are unsupported" (`docs/runbooks/scaling.md`), while two
other runbooks say the opposite. No binary checks the schema it runs on. Every migration runs in one transaction with no
lock timeout and no concurrent index build, so one migration can queue behind a long transaction and block the hot path
behind it. The released-upgrade tests prove an old reader and an old worker against the new schema, not old and new
writers together.

## Decision

1. Every migration after the lint's cut-off MUST declare `expand` or `contract` on the line after an optional
   `-- no-transaction`, and a contract names its expand and the capability that must be active before it runs. An expand
   migration MUST leave the schema usable by release N-1; a contract removes only what N-1 no longer uses and ships in a
   release after its expand. Applied migrations are never edited (their checksums are tested).
2. A transactional migration MUST take at most one lock that blocks inserts, last, under a 2 s lock timeout; a migration
   without a transaction holds one concurrent statement and is idempotent; constraints are added `NOT VALID` and validated
   later; no table rewrite, type change in place, rename, volatile default, undefaulted `NOT NULL` column, `SET NOT NULL`
   outside a contract after a validated check, or replaced function, trigger, policy or view in an expand migration; a
   definer function pins its search path and is revoked from public. A waiver is a header naming the rule and the reason.
3. `migrate` MUST apply migrations one by one, retry one that met a lock timeout, refuse a contract whose capability is
   not active or while a process older than the previous release lives, and refuse to run from an older release.
4. Each release MUST record, from a manifest made when it is cut, the oldest release whose binaries may run on its schema.
   Every entry point that opens the database MUST call an owner-defined handshake at start, on every new connection and at
   every heartbeat, and close its serving gate, write nothing and answer 503 when its release is too old or a migration it
   was built with is missing.
5. A change that writes state release N-1 cannot process MUST be behind a registry capability whose activation raises the
   oldest compatible release in the same transaction, and whose deactivation lowers it.
6. Release N MUST NOT be published unless N-1 and N pass the integrity oracle together, with N's migrations applied during
   the load, an activation, a deactivation and a rollback, and the 99th percentile latency during the migrations within the
   lock timeout of a baseline.
7. The promise starts with 0.3.0 as N-1; the upgrade from 0.2.x to 0.3.0 stays a stopped upgrade.

## Considered options

1. **Expand and contract with a handshake and a fleet gate (chosen).** The standard way to run old and new code on one
   schema, with the two checks that make it safe to forget nothing: a lint at review and a gate at run time.
2. **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
   durable-execution platform is chosen for.
3. **Blue and green databases.** Needs logical replication of the whole schema, whose triggers and definer functions
   fire again on apply.
4. **Documentation only.** The runbooks already contradict each other.

## Consequences

- Good: an upgrade is a rolling deployment; a binary on the wrong schema cannot corrupt state; a migration waits at most
  its lock timeout for its one blocking lock, which it takes last.
- Bad: every schema change that removes something takes two releases; migrations are written in more, smaller files.
- Harder: authors must think about N-1 for every change; the lint and the mixed-writer test make that mechanical.
- Compatibility: the marks and the table are additive; existing migrations are treated as of their release.
- Rollback: before the capability switch, roll the binaries back onto the expand schema; after it, switch the capability
  off and drain the runs that used it first.

## Evidence

Design 048 and its review reports; `scripts/test-release-upgrade-profile.py` and the `released-upgrade` CI job (what is
proven today). To be added when built: the lint's fixtures, the handshake tests and the first mixed-writer run.
