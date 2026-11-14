# ADR-0091: A one-shot job with the owner's credentials hands the cron triggers over after a deployment, once the fleet is stable and converged

**Status:** Proposed (2026-11-14; revised the same day after three reviews)
**Decided by:** owner for the behaviour and the mechanism (OD-2026-11-14-4 "Automático no upgrade"; OD-2026-11-14-5 "Job pós-deploy"; OD-2026-11-14-6, the opt-out; OD-2026-11-14-7, the tenant audit); agent for the details (tasks T018-082 to T018-087). The first draft (a definer function the runtime role calls) was read by the architecture, security and correctness reviewers on 2026-11-14 and replaced; this text has not been reviewed again and is not accepted.
**Refines / supersedes:** amends ADR-0083 decision 4 (the handover is made by the operator's command, run by the operator or by a job the deployment runs on the operator's behalf) and its accepted limit on audit (D2042); extends ADR-0083 decision 5 (what the command does outside its transaction). **ADR-0082 is not changed**: requirements are still recorded only by the operator's command with the schema-owning role (decision 1), and the command's transaction keeps the bounds of decision 2. Amends `plan.md` of feature 018 ("the registry ships in a release before the handover can be used").
**Enforced by:** nothing yet; the checks are tasks (T018-085). Until they exist and run this ADR is a design (constitution, rule 11).

## Context

Feature 018 hands the firing of the cron triggers from the legacy cron pass to the schedule pass once and one way (ADR-0083). Today an operator runs `control-plane schedule-trigger-handover --apply` with the schema-owning role: in one bounded transaction it records `schedules-v1` for every live engine through the registry and moves the state (`apps/control-plane/src/handover.rs:344-449`; `crates/infra-postgres/src/instances.rs:56-63`, `:161-209`). The owner decided that it happens during the upgrade without an operator (OD-2026-11-14-4), then, after the reviews of the first draft, how: a one-shot job with the owner's credentials that runs the same command and waits, bounded, until its conditions hold (OD-2026-11-14-5).

What the reviews established, and what constrains the job:

1. **The runtime role writes most of what a census reads.** `sagat_app` holds `INSERT, UPDATE, DELETE` on `scheduler_members` (`crates/infra-postgres/migrations/20261017000000_scheduler_shards.up.sql:92`), and `instance_register` takes the kind, the scheduler owner and the capabilities from its caller (`20270113000000_instance_registry.up.sql:180-183`, `:228-236`). A function the runtime role may execute would decide on inputs its caller can forge.
2. **An engine that has just left is invisible.** The census counts rows seen in the last 30 s (`20270114000000_trigger_handover.up.sql:217-218`); a clean stop deletes the registry row (`20270113000000_instance_registry.up.sql:280-287`) and the scheduler membership (`crates/infra-postgres/src/partitioning.rs:90-91`). The command answers it today with `--expect-live N` (`handover.rs:163-189`), which a deployment with an autoscaler cannot give: the engine count moves between `minReplicas` and `maxReplicas` (`infra/helm/values.yaml:165-180`).
3. **The registry cannot see a 0.2 binary**, and the registry ships in the release that hands over, against the premise of its own migration (`20270113000000_instance_registry.up.sql:20-21`). A 0.2 engine still announces itself in `scheduler_members` (`v0.2.0:crates/infra-postgres/src/partitioning.rs`, lines 51-60); a 0.2 control plane leaves no row anywhere, but a trigger it publishes has no schedule until the reconciliation makes one (G2020), because only a control plane of this release makes the schedule in the publishing transaction.
4. **`migrate` runs before the engines and must finish first** (`infra/docker/compose.yaml:162-164`, `:205-207`); the Helm chart runs no migration at all (`infra/helm/templates/NOTES.txt:11`).
5. **The command maps only its own words to "try again".** A lock timeout on the epoch (`55P03` from `platform_requirement_activate`) or a statement timeout (`57014`) is a database error with exit code 1 (`handover.rs:481-494`; `apps/control-plane/src/lib.rs:96-110`).
6. **The audit.** The handover row records the operator and the database role and no mode (`20270114000000_trigger_handover.up.sql:186-189`); `audit_records.tenant_id` is `NOT NULL` (`20261008000000_identity.up.sql:55-66`), so a platform act reaches a tenant only as a row of that tenant.

## Decision

1. **Who acts: a one-shot job with the owner's credentials, as `migrate` is (OD-2026-11-14-5).** It runs `control-plane schedule-trigger-handover --auto`: the same binary, the same checks and the same activation transaction as the operator's `--apply`, plus the conditions of decision 2. In the compose stack it is a service with `restart: "no"` that runs after the control plane and the engines have started; in Helm it is a Job with the hooks `post-install,post-upgrade`, `backoffLimit` 2, `activeDeadlineSeconds` longer than its `--wait`, a `podFailurePolicy` that fails the Job at once on exit codes 2 and 3, and the owner's `DATABASE_URL` from a Secret of its own (`schedules.autoHandover.ownerDatabaseSecret`). The runtime role MUST get no new privilege, and no long-running process MUST hold the owner's credentials. A deployment of its own runs the command (manifests not in this repository): the runbook says what the job does so that it can be reproduced.

2. **The condition: the command's census, made stable, with two detectors.** `--auto` MUST hand over only when, in `--stable N` consecutive rounds (default 3) spaced at least one heartbeat apart (15 s), all of these hold, and then again under the locks of the activation:
   - every engine seen within `--horizon` (default 10 min, at least 60 s) lists `schedules-v1` (a row that went silent within the horizon still counts);
   - the set of live engine instances is the same in every round and has at least `--min-live M` members (default 1; Helm passes the engine's `replicas`, or the autoscaler's minimum);
   - no `scheduler_members` row whose `expires_at` is within the horizon (expired rows within it count as present) has an owner that no engine row seen within the horizon names as its `scheduler_owner` (an engine the registry cannot see: a 0.2 binary);
   - no trigger-owned schedule was made, revived or removed by the reconciliation within the horizon (only a control plane older than this release leaves that work to it; the triggers older than the feature make the same mark right after an upgrade, so the handover waits a horizon after the backlog is made);
   - at least one control plane is live in the registry;
   - the drift view is empty.

   Under the locks the activation keeps its own checks (`handover.rs:297-325`) with `expect_live` set to the stable count, so an engine that registered or went silent after the last round refuses it with nothing recorded. The rounds are read by the owner (the registry's token column and history are not the runtime role's to read). `--expect-live N` stays and, given, replaces `--min-live` with the exact count (D2241 says the trade-off).

3. **No transaction is open across a wait.** Each round is short reads with no transaction held between them; the waits between rounds hold no connection; the activation is the existing bounded transaction (ADR-0082 decision 2) and is attempted only after a stable round. `--wait` (default 2 h, at most 24 h) bounds the whole job.

4. **Timeouts are a closed `BUSY` and are retried.** `55P03`, `57014`, `HANDOVER_BUSY` and the step bound of the activation (`StoreError::Unavailable` from `within_the_step`) MUST map to `BUSY`, which the wait loop retries at the next round and the operator's `--apply` reports as "run the command again". The statement timeout (3 s) MUST be longer than the longest lock wait of any one statement (2 s) plus its work, and the step bound (3 s) longer than the waits while the epoch is held (the table, 2 s; the handover row never waits, since two handovers serialise on the epoch) plus the work; both stay below the 5 s statement timeout of the processes that wait for the epoch (ADR-0082 decision 2).

5. **A blocked handover says why.** The closed reasons are `ENGINE_LACKS_CAPABILITY`, `ENGINE_UNREGISTERED`, `TOO_FEW_ENGINES`, `FLEET_CHANGING`, `RECONCILIATION_RECENT`, `NO_CONTROL_PLANE`, `UNCONVERGED`, `BUSY` and `DEADLINE`. Each round writes the last reason, its count and the time to an owner-written singleton `schedule_trigger_handover_attempt` (`sagat_app` may read it); every engine reads it with the handover state and exports `trigger_schedule_handover_blocked{reason}` (1 for the last reason while `LEGACY`, every series created at zero, all 0 once handed over). The alert `SagatTriggerHandoverBlocked` MUST be `max without (reason) (trigger_schedule_handover_blocked) == 1` for one hour, so that a change of reason does not restart its timer. A test MUST hold the reason codes of the SQL, the Rust enum, the alert's runbook anchors and the rows of runbook 6.3 to one set. The job prints each change of reason and exits **0** when it handed over, when it was already handed over and when the automatic handover is switched off; **2** when it is declined for a reason no wait cures (a permission, a state this build does not know); **3** at the deadline, with the last reason; **1** for a database failure.

6. **The opt-out is a deployment value (OD-2026-11-14-6).** `schedules.autoHandover: false` leaves the Helm Job out of the chart; its compose twin, `SAGAT_SCHEDULES_AUTO_HANDOVER=false`, makes the service exit 0 at once saying that the handover is left to the operator. Both default to on. No engine reads either.

7. **Audit (OD-2026-11-14-7).** The handover row MUST gain `changed_mode` (`OPERATOR` | `AUTOMATIC`, null only while `LEGACY`; the migration backfills `OPERATOR` on a row already handed over) and `changed_epoch` (the requirement epoch of the handover). In the activation's transaction the census MUST be copied into an append-only, owner-only child of the registry history keyed by the epoch (instance id, kind, binary version, `started_at`, `last_seen` of every engine and control plane seen within the horizon; at most 1,000 rows); the history row itself is generic and is not widened. **After** the commit the same command MUST write one `audit_records` row per tenant that has a cron trigger (action `schedule.trigger_handover`, actor the `--operator` subject, resource `platform`/`schedule_trigger_handover`, reason the mode), with an id derived from the tenant and the epoch and `ON CONFLICT DO NOTHING`, so that a rerun writes only what is missing; a run that finds the state handed over writes the rows of the recorded epoch before it exits 0, which is how a job that died between the commit and the rows completes them. The command logs one line with the mode, the epoch, the engines and the **count** of tenants, never their ids, and the process increments `trigger_schedule_handovers_total{mode}`.

8. **One-way, unchanged.** Nothing here adds a way back (`20270114000000_trigger_handover.up.sql:105-121`, `:225-238`). The new migration's `down` MUST refuse while any handover row has a mode.

## Considered options

1. **(a) A `SECURITY DEFINER` function the engines call** (the first draft). Lost: `sagat_app` writes the scheduler memberships and chooses the kind, owner and capabilities of a registry row (Context 1), so the function would decide on facts its caller can forge, and it narrowed ADR-0082 decision 1.
2. **An engine-only database role that alone may execute that function.** Lost: the engines still write the memberships and the registry rows, so the forgery stays; and splitting the runtime role touches every grant of every feature.
3. **(b) `migrate`, waiting until the condition holds.** Lost: the engines start after `migrate` has finished (Context 4), so it would wait for engines that wait for it.
4. **(c) The control plane at start-up.** Lost: it runs as the runtime role, so it needs (a); and the triggers have rarely converged when it starts.
5. **(d) A long-running process with the owner's credentials** (a sidecar or a scheduler loop). Lost: the owner credential would live as long as the platform; a one-shot job holds it for minutes.
6. **(e) A one-shot job running the tested command (chosen, OD-2026-11-14-5).** The owner's credential is held as long as `migrate` holds it, the runtime role gains nothing, the checks are the command's, and an operator can run the same command by hand.
7. **(f) Defer the automatic handover to 0.4.0** and keep the manual command in 0.3.0. Not chosen by the owner (OD-2026-11-14-4, OD-2026-11-14-5).

## Consequences

Good: a default upgrade hands over by itself after the last process has started, the backlog of the reconciliation is made and a horizon has passed; the runtime role and ADR-0082 are unchanged; a blocked handover says why in a metric whose alert does not restart; tenants see it in their audit.

Bad and accepted:
- a 0.2 engine started after the handover still ignores a pause (G2240); an old control plane that publishes nothing for a horizon and then publishes loses that trigger's first ticks (G2241);
- the job decides on rows the runtime role can write: it can delay the handover, or hide a 0.2 engine (a fake registry row naming its membership) or an old control plane (a mark cleared), but it cannot record the requirement (G2243);
- an engine that stopped cleanly leaves no row, so `--min-live` is the only guard against a fleet that is partly down (G2245);
- a fleet that never holds still for `--stable` rounds (an autoscaler, a crash loop) is never handed over by the job (G2242);
- the tenant set of the audit is read after the commit (G2244);
- a Helm `post-upgrade` hook makes `helm upgrade` wait for it and a hook that fails marks the release failed; with `--atomic`, Helm rolls the release back to the previous images, which is safe only because a failed job never handed over (G2246, O1956); one `helm upgrade` rolls engines and control planes together, so OD-2026-11-14-3's order is not what a single release does (G2246);
- the compose stack has no health checks on the control plane and the engine, so the service can only wait for them to start (G2247).

Operational cost: a few reads every 15 s for at most `--wait`; one bounded activation; one insert per affected tenant after it.

Compatibility with runs in flight: none changes; the gate of the legacy pass, the shared key, the five-second control window and the reconciliation are ADR-0083's.

**What the release notes of 0.3.0 MUST say** (D2248): the handover is automatic after a deployment, under which conditions, and how to see that it happened (`trigger_schedule_handed_over`, the log line, `SELECT state, changed_mode, changed_at FROM schedule_trigger_handover`, the tenant audit); it is one-way (no command back, the schema `down` refuses, no engine below 0.3.0 after it); `schedules.autoHandover: false` or `SAGAT_SCHEDULES_AUTO_HANDOVER=false` keeps it manual; `helm upgrade` needs a `--timeout` longer than the job's wait; the accepted losses G2005, G2020 and G2021; an engine with `SAGAT_SCHEDULES_ENABLED=false` holds it (`ENGINE_LACKS_CAPABILITY`).

**Rollback.** Before the handover: the opt-out, or binaries back to 0.2 and the schema `down` (which refuses once a registry history row exists). After it: not supported, as today.

## Test plan (T018-085; every test written first and seen failing for the right reason)

Races are held by an open transaction or a held lock and watched in `pg_stat_activity` (`wait_event_type = 'Lock'`), never timed.

- **An engine that registers without `schedules-v1`, both orders**: a registration held open makes the activation wait on the epoch and then refuse it; an activation held open makes the registration wait and answer `CAPABILITY_MISSING`.
- **A publication in flight, both orders**: `BUSY`, nothing recorded, retried at the next round; or the publication waits and its trigger has its schedule.
- **The job and the operator at once**: one hands over, the other answers "already handed over"; one epoch, one history row, one census copy, one audit row per tenant.
- **The departure case**: an engine stops cleanly between two rounds (`FLEET_CHANGING`); between the last round and the lock (the activation refuses with `expect_live`); an engine that crashed without `schedules-v1` within the horizon (`ENGINE_LACKS_CAPABILITY` until the horizon passes); fewer than `--min-live` (`TOO_FEW_ENGINES`).
- **The detectors**: a live and a recently expired 0.2 membership (`ENGINE_UNREGISTERED`), modelled by the 0.2 `announce` statement; a trigger inserted without its schedule, as a 0.2 control plane does, and made by the reconciliation (`RECONCILIATION_RECENT` for a horizon, backdated as the owner, not slept).
- **Timeouts**: `57014`, `55P03` and the step bound are `BUSY` and retried; a crash (backend terminated) at each wait records nothing; a crash after the commit and before the audit rows is completed by the next run, once.
- **The job's exit codes**: 0 for handed over, already handed over and switched off; 3 at the deadline with the last reason; 2 for the runtime role's credentials.
- **The runtime role**: nothing new is executable by `sagat_app`; it reads the attempt row; the privilege audit's `ALLOWED` list gains the owner-only statements of the job with their reasons.
- **The migration** (`.claude/skills/migration/SKILL.md`): additive; a row handed over before it is backfilled `OPERATOR`; `down` refuses for any mode; up, down, up with data in both roles.
- **The deployment**: `helm template` renders the Job with its hooks, limits and Secret, and leaves it out with `schedules.autoHandover: false` (`contract_scalability_ha`); a compose smoke test runs the service to exit 0 and, with the twin set to false, to exit 0 doing nothing.
- **The released-v0.2 upgrade**: from `v0.2.0` with cron triggers and a 0.2 engine's membership: migrate; 0.3.0 engines and a control plane; the job reports `ENGINE_UNREGISTERED`, then `RECONCILIATION_RECENT`, then hands over; one run per tick throughout; a pause is refused before and honoured after the control window.
- **The alert and the reasons**: a `promtool` rule test (CI runs every `*_test.yaml` since G2022 was closed on `feature/018-schedules-default-on`) in which a change of reason does not restart the hour; the one-set test of decision 5.

## Evidence

The code and the lines cited in Context; OD-2026-11-14-3 to OD-2026-11-14-7 in `docs/owner-decisions.md`; `specs/018-schedules-bulk-operations/research.md` D2240 to D2250, G2240 to G2247 and O1954 to O1956; tasks T018-082 to T018-087. No test of this design exists yet.
