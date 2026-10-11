# Validation quickstart: the Workflow Studio (feature 007)

Commands below reproduce the feature's checks. The original baseline and the visual-direction validation are recorded separately
below; a baseline pass is not evidence that a later restyle passed. What was **not** run is stated with each result.

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
   cargo test -p sagat-tests --test e2e_workflow_studio --test contract_rest_workflow_studio --test fault_workflow_studio
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
   cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
   SAGAT_STUDIO_GREP="US04" …            # one journey      SAGAT_STUDIO_REBUILD=0 …   # reuse the build
   ```

   If the sandbox container runtime is unavailable, an explicit partial development run is possible:

   ```sh
   SAGAT_STUDIO_NO_SANDBOX=1 cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
   ```

   This starts the real API, engine and browser, but no sandbox runner; it prints `PARTIAL STUDIO RUN` and excludes US02.
   PostgreSQL remains required. Set `SAGAT_TEST_DATABASE_URL` to an existing administrative test connection if Docker cannot
   start PostgreSQL either. Default/CI execution still requires real containers and includes US02. Never report this partial
   command as proof of the full US01..US04 suite or sandbox execution.

4. **The benchmark** (a 1 000-node canvas and a run of 9 991 nodes, three cold rounds):

   ```text
   STUDIO_PERF=1 STUDIO_PERF_OUT=$PWD/frontend/e2e/perf-results/perf.json SAGAT_STUDIO_GREP=PERF \
     cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
   node frontend/e2e/scripts/perf-report.mjs frontend/e2e/perf-results/perf.json tests/performance/reports/workflow-studio.md
   ```

5. **The repository's gates**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
   `python3 scripts/check-architecture.py`, `cargo audit --deny warnings`, `cargo test --workspace --locked`, the OpenAPI lint.

## Gates

- E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces: `sc001-chain.spec.ts` (the workflow is built in the
  Studio, the failure and the undo are the platform's, the journal and the trace id are read from the page).
- Reproducible, annotated errors; no confirmed transition lost: every error carries code, trace id and a way on (`ErrorState`);
  the stream and the changes are proven equal to the whole graph (`us03`), and the decisions are made once (`fault_workflow_studio`).
- Update tasks `[x]` only when real tests have passed.

## Original baseline results (before the visual direction)

Run on macOS 26 (arm64), Docker Desktop, Node 26.8.1, Chromium 153, on the commit that carries this file (the working tree held, besides it, the visual-direction records, which no test reads).

| What | Result |
|---|---|
| `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `python3 scripts/check-architecture.py`, `cargo audit --deny warnings` | clean (19 crates, no cycle) |
| `cargo test --workspace --locked --no-fail-fast` | 1 426 passed, 0 failed, 24 ignored (the ignored ones need five toolchains or a browser and run in their own jobs) |
| of them, for this feature | `e2e_workflow_studio` 25, `fault_workflow_studio` 5, `contract_rest_workflow_studio` 1, `definition_service` 39, `infra-ir` diff 18, `api-http` studio 11, the server's faults of 001 37 (the guard against a client that stops reading), `control-plane` config 29, the chaos test of the domain with the invariant that every changed step is reported |
| frontend: `npm run lint`, `typecheck` (app and journeys), `npm test`, `npm run build`, `npm run check:api`, `npm audit --audit-level=moderate` | clean; 849 tests in 62 files; two **low** advisories remain in production dependencies (DOMPurify inside Monaco, G152) |
| the journeys, `cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys` | 16 passed, three consecutive runs (35.5, 34.8 and 35.3 s of Playwright); the 2 benchmark journeys are skipped unless `STUDIO_PERF=1` |
| the benchmark (`tests/performance/reports/workflow-studio.md`) | six hypotheses, all held; one machine, one browser |
| the reviews | two rounds of an independent security and an independent correctness review (read-only agents); round one found 3 major and 3 minor findings of security and 2 major and 5 minor of correctness, round two 2 major and 3 minor of security and 4 minor of correctness; each confirmed finding was reproduced by a test that failed first (or by a measurement: 6.8 s and 48 s of CPU for two hostile documents, 231 MB for an answer, 120 scans of a table for one quiet stream) and fixed; what was not fixed is a gap with a number (G151–G159) |

What was **not** run, or not proven: the `studio` job of CI on GitHub (its commands were run locally); a sign-in against a real identity provider (a stand-in played by the journeys: G149); a screen reader used by a person (G146); a browser other than Chromium; the benchmark on another machine; the visual direction (`design/README.md`, FR-007-009/010, T007-039..045), which came after these results and is open.

### What the reviews found and what was done

Round one, security: quadratic work on a hostile draft in the diff of triggers and in the naming of unknown script artifacts (fixed: sets and one pass); the Studio's file server read whole files into memory with none of the API's protections (fixed: streamed under a cap, a deadline, and a write-stall guard in the server loop for every response); every tick of a stream read the whole graph (fixed: the run's row, the graph only when it moved), no cap in the whole process, and a stream that kept its subject's role for fifteen minutes (fixed: re-asked every 30 s); a diff of unbounded size (fixed: caps and a count of what is left out); tests that could pass without the protection they name (fixed). Round one, correctness: a delta that missed the steps an event creates without naming (fixed at the source: `RunEvent::changed_nodes`, with the invariant in the chaos test); a rename that left `steps.<id>` reads behind (fixed); a decision repeated after a lost answer that carried a new version and so a new key (fixed: pinned); a retry of a step that may have happened without the id typed back (fixed); pause and cancel disabled on an assumption (fixed, G154); stale validation (fixed: "Check again" and on return to the window); the classifier's coverage (said, G155).

Round two, security: the paths of a diff of a deeply nested document (fixed: a stack of borrowed names, rendered only for what is listed); streams that could take every connection (fixed: configurable, at most half of them); a cut stream said "the run is over" (fixed: it says "come back"); a file server that opened a pipe before looking at it, and a test of a slow client that could pass without the cut (fixed). Round two, correctness: a rename to a word of the language wrote an expression that does not parse (fixed); coming back to a window asked twice (fixed); the list of steps an event changes existed twice (fixed: one list); an undecodable journal entry silently missed (fixed: the whole graph is sent).

The final gate also found, in code written before this feature, what a run on Linux shows and a macOS machine does not: a timestamp with nanoseconds where the database keeps microseconds, tests that assumed the order or the speed of the machine, and a sandbox out-of-memory kill the engine did not report; they were fixed on the release branch (`release/0.1.0`).

## Visual-direction validation — 2026-10-11

This is the local worktree validation of the restyled Studio, not a GitHub CI result. The feature's REST contract and execution
semantics are unchanged. The implementation and accessibility deviations are in [`design/README.md`](./design/README.md).

| Check | Latest observed result |
|---|---|
| Studio Vitest | **1 033 passed in 64 files**. Four workers bound jsdom contention; test deadlines and assertions were not weakened. |
| Frontend lint, typecheck, production build, generated API types, `npm audit --audit-level=moderate` | Passed locally; the accepted low-severity Monaco dependency advisories remain G152. Final post-Monaco-fix checks passed. |
| Rust format, workspace/all-targets clippy, architecture check | Passed locally after the harness partial-mode change. This row does not claim a new complete workspace test or GitHub CI run. |
| Expanded Chromium journeys in partial mode | **18 passed, 0 failed, 2 skipped** after the dark Monaco contrast correction. All executed journeys passed, including light/dark/forced-colour accessibility, 320px reflow and system-font fallback. The two skips are opt-in benchmarks, passed separately. **US02 was excluded**, not passed, because Docker's sandbox runtime was unavailable. |
| Restyled benchmark | **Both browser benchmarks passed**, three cold rounds each; H1–H6 held with unchanged floors. Median first-card time: 687 ms for 1 000 editor nodes, 1 470 ms for 9 991 run nodes; 245 run cards and at most 29 table rows mounted; peak recorded heap 307 MB. Report regenerated at `tests/performance/reports/workflow-studio.md`. |
| Font fallback | Passed: blocked font files, asserted Geist did not load and checked 320px reflow. Capture: [`runs-system-fonts-1470x812.png`](./design/review/runs-system-fonts-1470x812.png). |
| Owner acceptance | **Approved by Marcelo on 2026-10-11**: "Aprovo a proposta visual", covering Runs, light theme, editor, Scripts and sign-in after opening the local Studio and receiving the reference/captures. |

The browser suite checks axe in light, dark and forced colours, including the Script editor/plain-text alternative, and selected
pages at 320px. Status locators in US03, US04 and SC-007-001 are scoped to the named **Run detail** region because the new master
list contains the same words. The expected states, deadlines, graph-update and audited-operation assertions retain their
original intent. The benchmark's run-table selector scopes `.rn-scroll` to `.rn-run` so that the new master list cannot be measured in its place.
Scenario sizes and performance thresholds are unchanged.

The independent restyle review found missing graph CSS tokens, low-contrast omitted edges, master-list state lost on run
navigation and invented elapsed time for terminal runs without a finish timestamp. These were fixed; regression coverage checks
token resolution, edge styling, master navigation and unknown duration. The selected-step mobile layout was corrected as well.

Ten named captures from the passing suite are preserved in [`design/review/`](./design/README.md#owner-review-captures--2026-10-11).
The initial expanded run had 16 passes and one dark Monaco contrast failure; the correction is covered by the final passing
suite and 1 033 unit tests. Before closing T007-044, verify US02 with the real Docker sandbox in CI; owner visual approval is recorded below. No GitHub CI execution, real identity-provider
sign-in, human screen-reader testing or additional browser/machine result is claimed for this restyle.


The final visual inspection also found that the initial wide header pushed the graph below the first screen and that React
Flow's dark background covered the dotted canvas. The wide composition now keeps all eight facts in two rows, places actions
beside the title and makes the library background transparent. Browser checks require a complete graph step in the initial
1470 × 812 viewport and verify the transparent overlay; the normal, forced-colour and 320px checks remain enabled. The graph
legend says "Dotted: omitted", matching the accessible full-opacity edge. Post-adjustment focused tests: 233 passed.

Implementation was split across GPT-5.6 Sol agents; independent correctness/security reviews used GPT-6 Astra. These agent
reviews are technical evidence only and do not replace owner visual acceptance or a human accessibility study.

Owner review recorded on 2026-10-11: Marcelo explicitly approved the visual proposal ("Aprovo a proposta visual")
after opening the local Studio. This approval covers Runs, the light theme, editor, Scripts and sign-in;
automated CI evidence for this branch is recorded separately.
