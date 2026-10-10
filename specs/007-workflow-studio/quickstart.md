# Validation quickstart: the Workflow Studio (feature 007)

Every command below was run on macOS 26 (arm64, Darwin 25.3) against this repository. Results are in the last section; what was
**not** run is stated there too.

## Prerequisites

- The Rust toolchain pinned by the repository, and Docker (the tests start their own PostgreSQL; the API, the execution engine and
  a sandbox runner run in the test process — no compose file is needed).
- For the frontend: Node ≥ 20.19 (run: 26.8.1). `cd frontend && npm ci` (exact versions from the lockfile, no install scripts).
- For the journeys: Chromium for Playwright, `cd frontend/e2e && npx playwright install chromium` (run: Chromium 153).
- No token, tenant or fixture is prepared by hand: the harness creates a tenant, a namespace, an administrator and the operator,
  viewer, developer and worker identities the journeys sign in as, and a second tenant for the isolation checks.

## Validation flow

1. **The server side, by the public interfaces** (real PostgreSQL; validate, diff, the graph and its changes, the stream, the static
   serving, the contract on the normal and error paths, and the faults around decisions):

   ```text
   cargo test -p sagat-tests --test e2e_007 --test contract_rest_007 --test fault_007
   cargo test -p sagat-api-http --test studio
   cargo test -p sagat-infra-ir -p sagat-application
   ```

2. **The Studio's checks** (lint, types, unit tests, the production build, the generated API types against the contract):

   ```text
   cd frontend && npm run lint && npm run typecheck && npm run typecheck --workspace @sagat/e2e \
     && npm test && npm run build && npm run check:api && npm audit --audit-level=moderate
   ```

3. **The journeys in a real browser** (`spec.md` US01..US04, SC-007-001, accessibility, security, sign-in), against the real
   platform the harness starts, with workers of the TypeScript SDK:

   ```text
   cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture
   SAGAT_STUDIO_GREP="US04" …            # one journey      SAGAT_STUDIO_REBUILD=0 …   # reuse the build
   ```

4. **The benchmark** (a 1 000-node canvas and a run of 9 991 nodes, three cold rounds):

   ```text
   STUDIO_PERF=1 STUDIO_PERF_OUT=$PWD/frontend/e2e/perf-results/perf.json SAGAT_STUDIO_GREP=PERF \
     cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture
   node frontend/e2e/scripts/perf-report.mjs frontend/e2e/perf-results/perf.json tests/performance/reports/007-workflow-studio.md
   ```

5. **The repository's gates**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
   `python3 scripts/check-architecture.py`, `cargo audit --deny warnings`, `cargo test --workspace --locked`, the OpenAPI lint.

## Gates

- E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces: `sc001-chain.spec.ts` (the workflow is built in the
  Studio, the failure and the undo are the platform's, the journal and the trace id are read from the page).
- Reproducible, annotated errors; no confirmed transition lost: every error carries code, trace id and a way on (`ErrorState`);
  the stream and the changes are proven equal to the whole graph (`us03`), and the decisions are made once (`fault_007`).
- Update tasks `[x]` only when real tests have passed.
