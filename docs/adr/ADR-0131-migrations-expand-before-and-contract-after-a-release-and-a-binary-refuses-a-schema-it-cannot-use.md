# ADR-0131: Migrations expand before a release and contract after it, and a binary refuses a schema it cannot use

**Status:** Proposed (2026-11-24)
**Decided by:** agent proposal (design 048), to be accepted under Owner decision 2026-11-24-4 once design 048's review is complete
**Refines / supersedes:** refines [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the registry's gate also closes on a schema mismatch, and its census gates new behaviour); replaces the stopped upgrade of `docs/runbooks/scaling.md` section 3 from release 0.3.0 on; supersedes nothing
**Enforced by:** `scripts/check-migrations.py` and its negative fixtures (Task 048-12), the schema handshake tests (Task 048-13), `scripts/test-mixed-writers.py` in the release workflow (Task 048-15)

## Context

Upgrades stop every writer today: "mixed old/new writers are unsupported" (`docs/runbooks/scaling.md`), while two
other runbooks say the opposite. No binary checks the schema it runs on. Every migration runs in one transaction with no
lock timeout and no concurrent index build, so one migration can queue behind a long transaction and block the hot path
behind it. The released-upgrade tests prove an old reader and an old worker against the new schema, not old and new
writers together.

## Decision

1. Every migration MUST declare `expand` or `contract`. An expand migration MUST leave the schema usable by release
   N-1; a contract migration MUST remove only what N-1 no longer uses and MUST ship in a release after its expand.
2. A migration MUST set a lock timeout, build indexes on existing tables concurrently, add constraints `NOT VALID` and
   validate them later, and MUST NOT rewrite a table, change a type in place, rename, or add a volatile default.
3. Each release's last migration MUST record the oldest release whose binaries may run on the schema. A process MUST
   close its serving gate, write nothing and report not ready when its release is older than that, or when a migration
   it was built with is missing; it re-checks at every instance heartbeat.
4. A change that writes state release N-1 cannot process MUST be behind a registry capability, switched on only when the
   registry's census shows no live N-1 process.
5. Release N MUST NOT be published unless N-1 and N engines and control planes pass the integrity oracle together on
   N's expand schema under load.
6. The promise starts with 0.3.0 as N-1. The upgrade from 0.2.x to 0.3.0 stays a stopped upgrade.

## Considered options

1. **Expand and contract with a handshake and a fleet gate (chosen).** The standard way to run old and new code on one
   schema, with the two checks that make it safe to forget nothing: a lint at review and a gate at run time.
2. **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
   durable-execution platform is chosen for.
3. **Blue and green databases.** Needs logical replication of the whole schema, whose triggers and definer functions
   fire again on apply.
4. **Documentation only.** The runbooks already contradict each other.

## Consequences

- Good: an upgrade is a rolling deployment; a binary on the wrong schema cannot corrupt state; a migration cannot
  stall the hot path for more than its lock timeout.
- Bad: every schema change that removes something takes two releases; migrations are written in more, smaller files.
- Harder: authors must think about N-1 for every change; the lint and the mixed-writer test make that mechanical.
- Compatibility: the marks and the table are additive; existing migrations are treated as of their release.
- Rollback: before the capability switch, roll the binaries back onto the expand schema; after it, switch the capability
  off and drain the runs that used it first.

## Evidence

Design 048 and its review reports; `scripts/test-release-upgrade-profile.py` and the `released-upgrade` CI job (what is
proven today). To be added when built: the lint's fixtures, the handshake tests and the first mixed-writer run.
