## What and why

<!-- One coherent increment. Link the spec: specs/NNN-slug and the task ids. -->

Refs: <!-- T0NN-xxx, FR-0NN-xxx -->

## Evidence

<!-- Commands and their real results. Say what was not run and why. Do not paste a summary you did not verify. -->

- [ ] `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `python3 scripts/check-architecture.py` and `python3 scripts/check-agent-config.py`
- [ ] Suites for what changed, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`; failing test seen red first
- [ ] Contracts, generated references and Studio types regenerated and read (no blind bless)
- [ ] Per-SDK checks if an SDK or a scenario changed (`sdks/AGENTS.md`)

## Same delivery

- [ ] Spec, plan, tasks (ticked with evidence), `traceability.md`, runbooks and docs updated
- [ ] Tenant boundary, typed errors, audit and telemetry covered; no sensitive data in logs
- [ ] Failure cases tested: crash, duplicate, race, lost delivery, restart (rule 9)

## Decisions

- [ ] Constitution Check still holds, or the violation is justified in the plan
- [ ] ADR written or superseded if the decision is hard to reverse (`/adr`); owner decisions recorded in `docs/owner-decisions.md`
- [ ] Gaps found are registered (`G…`) rather than hidden; migrations follow `/migration`

## Risk and rollback

<!-- Compatibility with runs in flight and older readers; how to roll back. -->

## AI assistance

<!-- Tools used and what a person reviewed. Agent reports were verified by rerunning the gates on the merged tree. -->
