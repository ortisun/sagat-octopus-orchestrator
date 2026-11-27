# Review: feature 018, the automatic handover (ADR-0091) — security

- **Doc:** `docs/adr/ADR-0091-a-one-shot-job-hands-the-cron-triggers-over-after-a-deployment.md` (the spec), with `docs/constitution.md`
- **Reviewed:** the diff of `crates apps infra tests` against the merge base with `develop` (`3268f621`), the working tree of `feature/018-schedules-default-on` on 2026-11-15 after head `19ad4a51` (before the fixes below)
- **Reviewer:** `security-reviewer` (independent read-only agent, given the diff and the ADR, not the author's conclusions)
- **Recorded by:** the author of the slice, from the reviewer's full report (summarised by finding, every finding kept)
- **Answered in:** the commit that adds this file (D2306, D2311, G2308)

## Findings

| Finding | Severity | How it is answered |
|---|---|---|
| 1 — `schedule_trigger_handover_role_narrow()` ignores schema privileges and database ownership: on a database first made before PostgreSQL 15 PUBLIC keeps `CREATE` on `public`, so `sagat_handover` could create `public.make_interval(integer)` and shadow the built-in a definer with `public` on its `search_path` calls (G2280), running as the owner | Major | **Fixed.** A role that may create in any schema, or owns a database, is `WIDER` (`up.sql`, `schedule_trigger_handover_role_narrow`); the log line and runbook 6.1 give `REVOKE CREATE ON SCHEMA public FROM PUBLIC`. Test, red first (`NARROW` where `WIDER` was expected): `trigger_handover_role::a_role_that_can_create_in_a_schema_or_owns_the_database_is_wider_and_loses_its_login` (PUBLIC's `CREATE` on `public`, a schema granted to the role, the database owned by it) |
| 2 — a role found wider keeps its login and password | Minor | **Fixed.** The grant function's `WIDER` branch also runs `ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL` (a `42501` leaves it to the operator). Same test, red first (`rolcanlogin` stayed true) |
| 3 — the `down` drops the retirement row and leaves a live login | Minor | **Decision D2306.** Built as asked, it broke the shared test cluster (every rollback test runs this `down`, and the role is the cluster's): a schema `down` must not alter a role of the cluster. Runbook 6.4 gives `ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL`; the same test runs that statement read from the runbook |
| 4 — the operator's free-text `--operator` name reaches every tenant's audit as the actor of an `OPERATOR` handover | Minor | **Owner (G2308).** ADR-0091 decision 7 chose the handover row's actor; whether tenants see `system:operator-handover` instead is the owner's |
| 5 — `runtime_privileges.rs` names a test that does not exist; the member test checks only `42501`, which a missing `EXECUTE` also gives | Nit | **Fixed.** The real names; the member test asserts the wrapper's own message ("made only by the role sagat_handover") |
| 6 — the job holds `SAGAT_HANDOVER_PASSWORD` in a plain `String` | Nit | **Fixed.** `zeroize::Zeroizing`, as `migrate` does |

## Checked by the reviewer and found sound

What `sagat_handover` may run and read; every new function's `search_path`, `PUBLIC` execute and schema-qualified calls; no argument weakens a condition (infinite intervals included); the wrapper's `session_user`, its recomputation under the epoch lock and its 12 s freshness; the note's closed set; the cap of sixteen runs under an advisory lock; what the runtime role gains (`SELECT` on the attempt rows and the count view) and the forgeries already accepted in G2243; the audit ids (`gen_random_uuid()`, owner-only) and `AUDIT_CONFLICT`; the password path (the verifier's alphabet checked before it is formatted, statement logging off, ASCII passwords, the password apart from the URL, the `FOR SHARE` check of `LEGACY` and the retirement); the retirement (`NOLOGIN PASSWORD NULL`, sessions ended, recorded when refused, `28P01` never a retirement); the Helm Job and Secret (no token, the NetworkPolicy, hardening, the controller UID); compose's environment and the smoke script (the password never printed, unset at the end); the metrics' closed labels.
