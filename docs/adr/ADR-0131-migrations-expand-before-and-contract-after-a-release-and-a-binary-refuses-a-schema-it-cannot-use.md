# ADR-0131: Migrations expand before a release and contract after it, and a binary refuses a schema it cannot use

**Status:** Proposed (2026-11-24; revised 2026-11-26 as design 053's record)
**Decided by:** agent proposal (design 053, split from design 048), to be accepted under Owner decision 2026-11-24-4 once design 053's review is complete
**Refines / supersedes:** refines [ADR-0082](ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md) (the registry's gate also closes on a schema mismatch, its census gates new behaviour, the oldest compatible release is computed from the capabilities in use, and a removal drains in two phases); replaces the stopped upgrade of `docs/runbooks/scaling.md` section 3 from release 0.3.0 on; supersedes nothing
**Enforced by:** `scripts/check-migrations.py` and its negative fixtures, the merge-base check and the checksum test (Task 053-2), the handshake tests on fixed versions as the runtime role and the architecture check of entry points (Task 053-3), `migrate`'s bounded-retry and refusal tests (Task 053-4), the oldest-compatible and two-phase tests (Task 053-5), `scripts/test-mixed-writers.py` in the release workflow (Task 053-6)

## Context

Upgrades stop every writer today: "mixed old/new writers are unsupported" (`docs/runbooks/scaling.md`), while two
other runbooks say the opposite. No binary checks the schema it runs on. Every migration runs in one transaction with no
lock timeout and no concurrent index build, so one migration can queue behind a long transaction and block the hot path
behind it. The released-upgrade tests prove an old reader and an old worker against the new schema, not old and new
writers together. Builds from `develop` carry the last release's version.

## Decision

1. Every migration after the lint's cut-off MUST declare `expand` or `contract` on the line after an optional
   `-- no-transaction`, and a contract names its expand and the capability that must be active before it runs. An expand
   migration MUST leave the schema usable by release N-1; a contract removes only what N-1 no longer uses and ships in a
   release after its expand; a new unique index or constraint on an existing table is a contract. A merged migration is
   never edited (checked against the merge base, and by checksums from each release).
2. A transactional migration MUST take at most one lock that blocks inserts on a hot table, last, and every lock under a
   2 s lock timeout; a migration without a transaction holds one concurrent statement and is idempotent; constraints are
   added `NOT VALID` and validated later; no table rewrite, type change in place, rename, volatile default, undefaulted
   `NOT NULL` column, `SET NOT NULL` outside a contract after a validated check, or replaced function, trigger, policy or
   view in an expand migration; a definer function pins its search path and is revoked from public. A waiver is a header
   naming the rule and the reason.
3. `migrate` MUST apply migrations one by one, retry a lock timeout and a failed concurrent index a bounded number of
   times and then stop naming what failed, refuse a contract whose capability is not active or while a process older
   than the previous release lives, and refuse to run from an older release.
4. A build from `develop` MUST carry the next minor's pre-release version, so that no two release lines share one. Each
   release records, from a manifest made when it is cut, the oldest release whose binaries may run on its schema; the
   effective oldest compatible release is computed from that and the capabilities in use, never stored as a counter.
5. Every entry point that opens the database MUST call an owner-defined handshake at start, at each confirmation of
   ADR-0130 and with each registry heartbeat, and close its serving gate, write nothing and answer 503 when its release is
   too old or an expand migration it was built with is missing; the gate reopens within one heartbeat once the schema is
   right.
6. A change that writes state release N-1 cannot process MUST be behind a registry capability, activated only when no
   live process is older than the release that introduced it, with a database-side fence or a stated harmlessness for a
   process that was silent at the activation; a removal MUST drain in two phases, the second only after every live
   process has heartbeated since the first.
7. Release N MUST NOT be published unless N-1 and N pass the integrity oracle together, with N's migrations applied during
   the load, an activation, a two-phase deactivation and a rollback, and the 99th percentile latency during the
   migrations within the lock timeout of a baseline.
8. The promise starts with 0.3.0 as N-1; the upgrade from 0.2.x to 0.3.0 stays a stopped upgrade.

## Considered options

1. **Expand and contract with a handshake and a fleet gate (chosen).** The standard way to run old and new code on one
   schema, with the two checks that make it safe to forget nothing: a lint at review and a gate at run time.
2. **A maintenance window per upgrade (today).** Safe and simple, and it gives up the main operational property a
   durable-execution platform is chosen for.
3. **Blue and green databases.** Needs logical replication of the whole schema, whose triggers and definer functions
   fire again on apply.
4. **Documentation only.** The runbooks already contradict each other.
5. **A stored oldest compatible release** (this record's first draft). Two capabilities and a removal can leave it wrong.

## Consequences

- Good: an upgrade is a rolling deployment; a binary on the wrong schema cannot corrupt state; a migration waits at most
  its lock timeout for its one blocking lock, which it takes last.
- Bad: every schema change that removes something takes two releases; migrations are written in more, smaller files; a
  removal of a capability takes at least one heartbeat.
- Harder: authors must think about N-1 for every change, and each capability must name its fence; the lint and the
  mixed-writer test make that mechanical.
- Compatibility: the marks, the table and the columns are additive; existing migrations are treated as of their release.
- Rollback: before the capability switch, roll the binaries back onto the expand schema; after it, drain the capability
  in two phases first.

## Evidence

Design 053 and its review reports (design 048's earlier reports for the first drafts); `scripts/test-release-upgrade-profile.py`
and the `released-upgrade` CI job (what is proven today). To be added when built: the lint's fixtures, the handshake
tests and the first mixed-writer run.
