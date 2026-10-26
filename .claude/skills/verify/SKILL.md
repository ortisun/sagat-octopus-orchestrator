---
name: verify
description: Run the pre-merge checklist (format, clippy, architecture, agent config, the right test suites, runtime-role sweep, contracts and generated files, per-SDK lints) and report real results. Use before declaring work done, before opening a pull request, or after merging branches.
when_to_use: "done?; ready for PR; run the gates; verify the merge; what does CI check"
---

Evidence over assertion (AGENTS.md rules 12 and 17). Run what applies to the change, report each command with its real result, and stop at the first red to explain it. Use a private `CARGO_TARGET_DIR`, check `df -h`, and never run more than two heavy builds at once.

## Always
1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `python3 scripts/check-architecture.py && python3 scripts/check-agent-config.py`
4. The suites of every crate and feature you touched: `cargo test -p sagat-tests --test <kind>_<feature>`; also `integration_platform_foundation` (it holds the runtime-privilege audit) and `integration_security_multitenancy` when you touched SQL, permissions or an API.
5. The same suites with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. (Not `integration_scalability_ha`: it probes owner-only operations on purpose.)

## When the change touches
- **An API or contract** (`openapi.yaml`, `workflow.schema.json`, proto, AsyncAPI): `contract_developer_experience` (bless docs only after reading the diff), `contract_rest_*`, the baselines and compatibility tests, `cd frontend/apps/studio && npm run api && npm run check:api`, and the ajv check CI runs on `workflow.schema.json` (`--strict=false`). New operations need a body in the cross-tenant battery (`body_of`) and a marker (`x-planned` / `x-implemented`).
- **A new error code or definition problem**: help in `apps/cli/src/diagnostics.rs` and a section in `docs/quickstarts/errors.md` with the same sentence; `cargo test -p sagat-cli`.
- **An SDK or a scenario**: every SDK check in `sdks/AGENTS.md`, then the conformance scenarios without bless.
- **The Studio**: `frontend/AGENTS.md`.
- **A migration**: `/migration`.
- **A decision** hard to reverse: `/adr`.

## Whole workspace
`RUST_TEST_THREADS=2 cargo test --workspace --locked --no-fail-fast` is what CI runs. On this machine run it with the Docker daemon of the project (`DOCKER_HOST=unix://$HOME/.colima/sagat-remaining/docker.sock`) or failures will be environmental; say so rather than trusting or ignoring them.

## Report
List: command, result (counts), anything skipped and why, anything flaky and its cause. Tick `tasks.md` only for what the output proves.
