# Targeted architecture re-read of design 050 for approval, round 3 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: d8a16e9a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 2 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Round 2's findings are closed or no longer apply.
Counts: Blocker 0, Major 1, Nit 4

1. **Major. Technical Plan 6 "Cursor sealing", Task 050-4.** The adapter's key loading, reloading and failures are undefined: replicas that load only at start disagree after a rotation; `seal` cannot fail and `open`'s `None` would turn a database fault into a `400`. Fix: load at start, reload on a timer and once on an unknown id, a fallible port mapping to `503`, an empty table answering `503`, a cross-replica test.
2. **Nit. Task 050-4.** Key creation in `apps/control-plane` would miss the harness and is not safe under concurrent migrations; the rotation's audit has no home. Fix: create it in `sagat_infra_postgres::migrate` with a singleton guard; `--operator` and the deployment-wide record.
3. **Nit. Technical Plan 6, Constitution Check.** `aes-gcm` and `hkdf` are not dependencies of `infra-postgres`. Fix: list the change; the `hkdf` version; `aes-gcm` in the workspace dependencies.
4. **Nit. Architecture decision 0144.** Keeping a key in PostgreSQL is not among the considered options, and "SELECT only" has no check. Fix: an option; two tests in "Enforced by".
5. **Nit. Wording.** A test name contradicts the kept previous key; a Risks row still says "purpose key". Fix: rename; reword.
