# ADR-0131: Migrations expand before a release and contract after it, and a binary refuses a schema it cannot use

**Status:** Proposed (2026-11-24; revised 2026-11-26 as design 053's record, and again after its full review round)
**Decided by:** agent proposal (design 053, split from design 048), to be accepted under Owner decision 2026-11-24-4 once design 053's review is complete
**Refines / supersedes:** refines [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the registry's gate also closes on a schema the binary cannot use; its census gates new behaviour, which each capability fences; a removal drains in two phases, the second only after every live process has applied the first; the heartbeat records the epoch a process has applied); replaces the stopped upgrade of `docs/runbooks/scaling.md` section 3 from release 0.3.0 on; supersedes nothing
**Enforced by:** `scripts/check-migrations.py` and its negative fixtures, and the merge-base check (Task 053-2); the schema-check tests as the runtime role and the architecture check of entry points (Task 053-3); `migrate`'s bounded-retry and no-process tests (Task 053-4); the two-phase tests, the barrier test of a beat overlapping phase 1 among them (Task 053-5); `scripts/test-mixed-writers.py` and the forward-merge check of hotfixes in the release workflow (Task 053-6)

## Context

Upgrades stop every writer today: "mixed old/new writers are unsupported" (`docs/runbooks/scaling.md`), while two
other runbooks say the opposite. No binary checks the schema it runs on. Every migration runs in one transaction with no
lock timeout and no concurrent index build, so one migration can queue behind a long transaction and block the hot path
behind it. The released-upgrade tests prove an old reader and an old worker against the new schema, not old and new
writers together.

## Decision

1. Every migration after 0.3.0's last one MUST only expand (the migrations of 0.3.0 run in a stopped upgrade; the
   lint's security rules apply from the lint's own creation): it leaves the schema usable by
   every older binary the registry admits, and adds no refusal an older binary can hit on an existing table unless the
   refusal is conditional on a capability's row. Contract migrations, which remove what an older release used, wait for
   the design that needs the first one, which adds their rules (a contract after its expand's release, recorded with
   its capability so that a deactivation cannot race it); until then the lint refuses them. A merged migration is never
   edited.
2. A transactional migration MUST take at most one lock that blocks inserts on a hot table, last, under a 2 s lock
   timeout and a 2 s statement timeout; on a hot table an index, unique constraint or primary key is built only
   concurrently; a migration without a transaction holds one concurrent statement and is idempotent; constraints are
   added `NOT VALID` and validated later; no table rewrite, type change in place, rename, volatile default, undefaulted
   `NOT NULL` column, or replaced function, trigger, policy or view that a released binary relies on; a definer function
   pins its search path and is revoked from public. The lint parses each statement with PostgreSQL's own parser. A
   waiver is a header naming the rule and the reason.
3. `migrate` MUST confirm the database as ADR-0130's one-run commands do, apply migrations one by one, retry a lock or
   statement timeout and a failed concurrent index a bounded number of times and then stop naming what failed, and
   index a partitioned table only by the procedure the design defines (the parent `ON ONLY`, each partition
   concurrently, then attached).
4. Every entry point that opens the database, except `migrate`, MUST ask the database which of the migrations it was
   built with are not applied, and while any is missing close its serving gate (`SCHEMA_TOO_OLD`), write nothing but its
   registry rows and the release of its shards, register only the capabilities the schema knows, and answer 503; the
   gate reopens within one heartbeat once the schema is right. The functions a binary calls before its migrations keep
   their names and signatures from the release before it.
5. A change that writes state release N-1 cannot process MUST be behind a registry capability, required of every kind
   that could meet the state and activated only when no live process lacks it, with a database-side fence conditional on
   the capability's row, or a stated harmlessness, for a process that was not live at the activation or at a
   deactivation. An older binary is kept away from newer state by capabilities alone; there is no gate on release
   versions.
6. A deactivation MUST drain in two phases: the first marks the capability `draining`, so that processes stop new uses,
   and can be undone; the second removes it only after every live process has applied the first phase's epoch,
   recorded by its heartbeat, and the capability's own drain check passes. A capability whose drain check cannot pass
   once used cannot be removed.
7. Release N MUST NOT be published unless N-1 and N pass the integrity oracle together, with every run reaching a
   terminal state, N's migrations applied during the load, an activation, a two-phase deactivation and a rollback, and
   the 99th percentile latency during the migrations within 2 s of a baseline; each capability adds its scenario. A
   hotfix's migrations are merged forward before the next minor release is cut.
8. The promise starts with 0.3.0 as N-1; the upgrade from 0.2.x to 0.3.0 stays a stopped upgrade.

## Considered options

1. **Expand-only migrations with a schema check and capabilities for new state (chosen).** The standard way to run old
   and new code on one schema, with the two checks that make it safe to forget nothing: a lint at review and a gate at
   run time.
2. **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
   durable-execution platform is chosen for.
3. **Blue and green databases.** Needs logical replication of the whole schema, whose triggers and definer functions
   fire again on apply.
4. **Documentation only.** The runbooks already contradict each other.
5. **A release-version gate beside the capabilities** (this record's earlier drafts: a compatibility table, an oldest
   compatible release, `develop` on the next pre-release). It refuses nothing a capability or sqlx does not already
   refuse, and needs one version order across SQL, Cargo, Python packaging and the Studio.

## Consequences

- Good: an upgrade is a rolling deployment; a binary on the wrong schema cannot corrupt state; a migration blocks inserts
  on a hot table for at most 2 s plus one commit; a deactivation cannot pass while a process still uses the capability.
- Bad: removing anything from the schema waits for the contract rules and then takes two releases; migrations are
  written in more, smaller files; a removal of a capability takes at least one heartbeat.
- Harder: authors must think about N-1 for every change, and each capability must name its fence and its scenario; the
  lint and the mixed-writer test make that mechanical.
- Compatibility: the columns and functions are additive and land in 0.3.0's stopped upgrade; existing migrations are
  before the lint's cut-off.
- Rollback: before the capability switch, roll the binaries back onto the expanded schema; after it, drain the
  capability in two phases first.

## Evidence

Design 053 and its review reports (design 048's earlier reports for the first drafts); `scripts/test-release-upgrade-profile.py`
and the `released-upgrade` CI job (what is proven today). To be added when built: the lint's fixtures, the schema-check
tests, the barrier test of phase 2 and the first mixed-writer run.
