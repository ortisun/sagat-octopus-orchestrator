# ADR-0131: Migrations expand before a release and contract after it, and a binary refuses a schema it cannot use

**Status:** Proposed (2026-11-24; revised 2026-11-26 as design 053's record, and again after its full review round)
**Decided by:** agent proposal (design 053, split from design 048), to be accepted under Owner decision 2026-11-24-4 once design 053's review is complete
**Refines / supersedes:** refines [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the registry's gate also closes on a schema the binary cannot use; its census gates new behaviour, which each capability fences; a removal drains in two phases, the second only after every live process has applied the first; the heartbeat records the epoch a process has applied; a capability's writes are fenced in the database); amends the "Enforced by" line of [ADR-0132](ADR-0132-the-journal-is-partitioned-by-the-month-a-run-was-created-and-a-run-owns-its-payloads.md): `journal-buckets-v1` cannot be removed by its drain check and is fenced by a trigger on `runs`, with no permanent flag (design 049's Task 049-20); replaces the stopped upgrade of `docs/runbooks/scaling.md` section 3 for upgrades from 0.3.0 on (0.3.0 as N-1); supersedes nothing
**Enforced by (pending until built):** `scripts/check-migrations.py` and its negative fixtures, the merge-base checks and the catalog test of every definer function (Task 053-2); the schema-check tests as the runtime role and the architecture check of entry points (Task 053-3); `migrate`'s bounded-retry, recorded-index and no-process tests, with `concurrent_runners_do_not_corrupt_or_fail` kept (Task 053-4); the two-phase tests, the barrier tests of a paused beat and of a use in flight among them, and the changed registry tests (`the_runtime_role_reads_the_registry_and_calls_its_four_functions_only`, the runtime-privilege audit) (Task 053-5); `scripts/test-mixed-writers.py` and the forward-merge check of hotfixes in the release workflow (Task 053-6)

## Context

Upgrades stop every writer today: "mixed old/new writers are unsupported" (`docs/runbooks/scaling.md` and the 0.2.0
release notes), while `docs/runbooks/control-plane.md` says the opposite. No binary checks the schema it runs on. Every migration runs in one transaction with no
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
2. A transactional migration MUST begin with a 2 s lock timeout and take at most one lock that blocks inserts on a hot
   table, last, under a 2 s statement timeout (one migration may widen the three closed capability lists, which changes
   only the catalog); on a hot table an index, unique constraint or primary key is built only
   concurrently; a migration without a transaction holds one concurrent statement and is idempotent; constraints are
   added `NOT VALID` and validated later; no table rewrite, type change in place, rename, volatile default, undefaulted
   `NOT NULL` column, or replaced function, trigger, policy or view that a released binary relies on; a definer function
   pins its search path and is revoked from public. The lint parses each statement with PostgreSQL's own parser. A
   waiver is a header naming the rule and the reason.
3. `migrate` MUST confirm the database as ADR-0130's one-run commands do, hold sqlx's migration lock, apply migrations
   one by one, retry a lock or statement timeout and a failed concurrent index a bounded number of times and then stop
   naming what failed, record a concurrent index only once it is valid, and index a partitioned table only by the
   procedure the design defines (the parent `ON ONLY`, each partition concurrently, then attached). The integration
   suites run the same loop.
4. Every entry point that opens the database, except `migrate`, MUST ask the database which of the migrations it was
   built with are not applied, and while any is missing close its serving gate (`SCHEMA_TOO_OLD`), write nothing but its
   registry rows and the release of its shards, register only the capabilities the schema knows, and answer 503; the
   gate reopens within one heartbeat once the schema is right. The functions a binary calls before its migrations keep
   their names and signatures from the release before it.
5. A change that writes state release N-1 cannot process MUST be behind a registry capability, required of every kind
   that could meet the state and activated only when no live process lacks it, with a stated harmlessness or a fence
   for a process not live at the activation, and always a database-side fence, conditional on the capability's row, on
   every write of its state, which the deactivation's second phase waits for. A process uses a capability only inside
   its capability window and only while the registry counts it. An older binary is kept away from newer state by capabilities alone; there is no gate on release
   versions.
6. A deactivation MUST drain in two phases: the first marks the capability `draining`, so that processes stop new uses,
   and can be undone; the second, under the epoch lock and the capability's row lock (and the written table's share
   lock where the fence is a trigger), removes it only after every live process of its kinds has applied the first
   phase's epoch, recorded by its heartbeat and bounded by the current epoch, every write in flight has landed, and the
   capability's own drain check passes; its refusals are named, and a row that never applies can be evicted by the
   owner. A capability whose drain check cannot pass once used cannot be removed.
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
   compatible release, `develop` on the next pre-release). It refuses nothing a capability or the loop does not already
   refuse, and needs one version order across SQL, Cargo, Python packaging and the Studio.
6. **A one-phase removal, or a second phase trusting the applied epochs alone.** A process may still write after the
   removal, and a use begun before the drain can commit after it; only a fence in the database orders the two.
7. **A permanent flag on capabilities** (the earlier drafts). The drain check that cannot pass once used says the same,
   where the removal already runs it.
8. **The handshake returning every applied migration on every heartbeat, and registration returning the requirement
   set.** The gate already reads the set again when the epoch moves; the schema cannot go back once it is right.
9. **Squawk for the lint.** Its rules overlap ours only in part; one parser with our rules keeps one owner per rule.

## Consequences

- Good: an upgrade is a rolling deployment; a binary on the wrong schema cannot corrupt state; a migration blocks inserts
  on a hot table for at most 2 s plus one commit; a deactivation cannot pass while a live process has not applied it or a write of the capability's state is in flight,
  and no write lands after it.
- Bad: removing anything from the schema waits for the contract rules and then takes two releases; migrations are
  written in more, smaller files; a removal of a capability takes at least two
  heartbeats (about 24 s); each capability's write carries its fence.
- Harder: authors must think about N-1 for every change, and each capability must name its fence and its scenario; the
  lint and the mixed-writer test make that mechanical.
- Compatibility: the columns and functions are added, and the registry's guard, its two feature guards,
  `platform_requirement_remove`, `schedule_trigger_handover_complete` and `lease_withdrawal_required()` replaced in
  place, all in 0.3.0's stopped upgrade; the migrations of 0.3.0 and before are outside the online rules.
- Rollback: before the capability switch, roll the binaries back onto the expanded schema; after it, drain the
  capability in two phases first.

## Evidence

Design 053 and its review reports (design 048's earlier reports for the first drafts); `scripts/test-release-upgrade-profile.py`
and the `released-upgrade` CI job (what is proven today). To be added when built: the lint's fixtures, the schema-check
tests, the barrier test of phase 2 and the first mixed-writer run.
