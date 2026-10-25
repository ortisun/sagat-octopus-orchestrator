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
| frontend: `npm run lint`, `typecheck` (app and journeys), `npm test`, `npm run build`, `npm run check:api`, `npm audit --audit-level=moderate` | clean; the original run had 849 tests in 62 files. The two low DOMPurify advisories recorded then are now fixed by the 3.4.16 override (G152); the current Studio gate passes 1,105 tests and `npm audit --audit-level=low` reports zero vulnerabilities. |
| the journeys, `cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys` | 16 passed, three consecutive runs (35.5, 34.8 and 35.3 s of Playwright); the 2 benchmark journeys are skipped unless `STUDIO_PERF=1` |
| the benchmark (`tests/performance/reports/workflow-studio.md`) | six hypotheses, all held; one machine, one browser |
| the reviews | two rounds of an independent security and an independent correctness review (read-only agents); round one found 3 major and 3 minor findings of security and 2 major and 5 minor of correctness, round two 2 major and 3 minor of security and 4 minor of correctness; each confirmed finding was reproduced by a test that failed first (or by a measurement: 6.8 s and 48 s of CPU for two hostile documents, 231 MB for an answer, 120 scans of a table for one quiet stream) and fixed; what was not fixed is a gap with a number (G151–G159) |

What was **not** run, or not proven: the `studio` job of CI on GitHub (its commands were run locally); a sign-in against a real identity provider (a stand-in played by the journeys: G149); a screen reader used by a person (G146); a browser other than Chromium; the benchmark on another machine; the visual direction (`design/README.md`, FR-007-009/010, T007-039..045), which came after these results and is open.

### What the reviews found and what was done

Round one, security: quadratic work on a hostile draft in the diff of triggers and in the naming of unknown script artifacts (fixed: sets and one pass); the Studio's file server read whole files into memory with none of the API's protections (fixed: streamed under a cap, a deadline, and a write-stall guard in the server loop for every response); every tick of a stream read the whole graph (fixed: the run's row, the graph only when it moved), no cap in the whole process, and a stream that kept its subject's role for fifteen minutes (fixed: re-asked every 30 s); a diff of unbounded size (fixed: caps and a count of what is left out); tests that could pass without the protection they name (fixed). Round one, correctness: a delta that missed the steps an event creates without naming (fixed at the source: `RunEvent::changed_nodes`, with the invariant in the chaos test); a rename that left `steps.<id>` reads behind (fixed); a decision repeated after a lost answer that carried a new version and so a new key (fixed: pinned); a retry of a step that may have happened without the id typed back (fixed); pause and cancel disabled on an assumption (fixed, G154); stale validation (fixed: "Check again" and on return to the window); the classifier's coverage (said, G155).

Round two, security: the paths of a diff of a deeply nested document (fixed: a stack of borrowed names, rendered only for what is listed); streams that could take every connection (fixed: configurable, at most half of them); a cut stream said "the run is over" (fixed: it says "come back"); a file server that opened a pipe before looking at it, and a test of a slow client that could pass without the cut (fixed). Round two, correctness: a rename to a word of the language wrote an expression that does not parse (fixed); coming back to a window asked twice (fixed); the list of steps an event changes existed twice (fixed: one list); an undecodable journal entry silently missed (fixed: the whole graph is sent).

The final gate also found, in code written before this feature, what a run on Linux shows and a macOS machine does not: a timestamp with nanoseconds where the database keeps microseconds, tests that assumed the order or the speed of the machine, and a sandbox out-of-memory kill the engine did not report; they were fixed on the release branch (`release/0.1.0`).

## Visual-direction validation — 2026-10-11

This record distinguishes local validation from the completed Studio CI job. The feature's REST contract and execution
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
suite and 1 033 local unit tests. The real-sandbox CI result below closes T007-044 together with owner approval. Human screen-reader testing remains outside this evidence.


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

## Real-sandbox CI evidence

The [Studio job](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/38156597169/job/114520746513) passed on commit `ace4060` (2026-10-11): **1 034 unit tests in 64 files**, lint, typecheck, generated API types and build; **19 browser journeys passed**, including US02 executing a script in the real sandbox. The two skipped cases are the opt-in benchmarks, which passed separately locally. This closes the sandbox validation gap of the partial local run and T007-044. Other jobs in that workflow were still running when this record was written.

The feature-012 native onboarding harness also passed against this final feature-007 frontend via `SAGAT_STUDIO_DIST`, using the real CLI, TypeScript SDK, engine and development identity provider.

### Workflow input/output schema editing

Open Workflow settings → Data contract. Both schemas offer JSON Schema autocomplete and local
validation through the bundled Monaco service. “Use a plain text box” keeps the current text,
including an unfinished edit. Invalid JSON is not written to the workflow; the last valid schema
remains until the text is corrected or cleared. Server-side validation remains authoritative.

Validation on 2026-10-12: 31 focused component tests and all three Chromium US01 journeys passed,
including schema completions, an invalid-type diagnostic and preservation when switching editor
modes. Typecheck, lint and production build passed. This focused browser run used the explicit
no-sandbox harness mode; it does not revalidate the separate US02 inline-script runner journey.

### Authorized structural payload inspection

An Admin or Operator can open a run, select the run input or a journal event, and choose **Inspect
redacted payload**. The response follows `structure-only-v1`: it shows container types and counts,
labels children only by ordinal position, and never returns object keys or scalar values. External
payloads show verified catalog metadata without downloading content. Missing, unsupported and
truncated payloads have explicit states. Changing run, node, tenant or session clears the result and
aborts an in-flight request.

The focused validation run on 2026-10-12 was:

```text
cargo test -p sagat-tests --test e2e_workflow_studio payload_inspection -- --nocapture
cargo test -p sagat-application --test payloads
cargo test -p sagat-infra-ir --test payload_projection
cargo test -p sagat-domain --test identity
cd frontend && npm test -- PayloadInspection RunDetailPage TimelineView NodePanel
cd frontend && npm run lint && npm run typecheck && npm run build
SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_GREP="operator inspects useful redacted input" \
  cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

Results: 2/2 real PostgreSQL/API tests, 6/6 application tests, 4/4 projector tests, 27/27
permission tests, 70/70 focused Studio tests, and one real Chromium journey passed. The browser
journey exercised the real API and engine; no sandbox script was involved. The suites cover exact
Admin/Operator permissions, all refused roles, foreign/missing opacity, audit failure and collapse,
historical selection after retry, forged blob-shaped JSON, verified external metadata, invalid and
oversized input, depth/node/child/response caps, UTF-8, and stale query/session cancellation. Canary
keys, strings, numbers, booleans and schema annotations are asserted absent from HTTP responses and
the DOM. Customer classification and raw-value disclosure remain pending under G42/G39.

### Visual defect correction — 2026-10-12 (T007-049)

Independent agent reviews used GPT-6 Astra for UX direction and GPT-5.6 Sol for frontend fixes,
geometry tests and code review. The root agent integrated the fixes and ran the final checks.
Graphs now open at 1:1 on a forward root/trigger, with Fit View as the explicit overview. Secondary
run facts are in **Run details**; the canvas uses the actual remaining layout height. Editor fields,
palettes, dialogs, forced-colour selection and populated mobile Scripts tables remain within
their containers. The API contract and tenant/audit/OTel behaviour are unchanged.

Final local results:

- `npm test`: **1 049 passed in 67 files**.
- `npm run lint`, app typecheck through `npm run build`, `npm run typecheck --workspace @sagat/e2e`,
  `npm run check:api` and `git diff --check`: passed.
- Production build: passed.
- Real Chromium with real PostgreSQL, API and engine: **23 passed, zero failed, two skipped** in
  49.8 seconds. Skips are the opt-in performance benchmarks. **US02 was excluded explicitly**;
  this run does not revalidate sandbox script execution.

Reproduce from the repository root after `cd frontend && npm run build`:

```sh
SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55432/postgres \
SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0 \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 \
cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

The `VISUAL` journeys assert readable initial zoom, visible graph controls, inspector bounds and
non-overlapping run headings, at 320/768/1024/1280/1470px as applicable. They cover real populated
Scripts tables, keyboard menus/disclosures and selected controls in forced colours. Existing
journeys retain their security/runtime assertions and explicitly open Run details or Fit View when
they need those presentations. Earlier iterations exposed a forward-vs-compensation root bug,
attribution-link contrast and an absolutely positioned table label that escaped the scroller;
all were corrected before the passing run. An intermediate run also missed the US04 fixture's
manual-review state; both US04 journeys passed in the final clean platform run without backend changes.

Eight [review captures](./design/README.md#visual-corrections-2026-10-12-t007-049) preserve before/after,
editable/read-only, dark/light/forced colours and mobile evidence. These are agent-reviewed images;
no new human acceptance, screen-reader study, non-Chromium result or performance benchmark is claimed.

## Asset replacement recovery — 2026-10-12 (T007-050)

The reported CSS preload failure was reproduced at `127.0.0.1:18080`: the server returned an HTML
entry for `index.CmpvQGsS.js` while disk already held the newer build, and the old
`workflows.CjFtM_i_.css` returned 404. `Site.index` retained startup HTML even though asset requests
read current files. This was a server snapshot error, not an incorrectly cached HTML response
(`Cache-Control` was already `no-store`). The earlier T007-049 browser harness started a fresh
server and did not cover replacement while it remained running.

The HTTP adapter now reads bounded, confined HTML on each request (root, deep link and explicit
index). Missing/oversized/symlink HTML returns 503 with Retry-After, without a stale fallback.
The eager route boundary replaces the developer error screen with a focused heading, explicit
reload and an unsaved-edit/session warning. The [recovery capture](design/review-2026-10-12/route-load-recovery.png)
shows the tested 320px layout. The UI follows the documented [React Router error boundary](https://reactrouter.com/how-to/error-boundary)
mechanism; [Vite's load-error guidance](https://vite.dev/guide/build#load-error-handling) describes
the missing-chunk case. No automatic reload or session persistence was added.

Validation:

- Before the fix, the new HTTP regression received startup HTML after replacing `index.html`;
  two component regressions rendered the router's developer error screen. Both failures were observed.
- `CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p sagat-api-http --test studio`:
  **13 passed**, including index replacement/HEAD, missing/unsafe/oversized index, path confinement,
  cache/CSP, transfer quotas and slow clients. Targeted `cargo clippy -p sagat-api-http --test studio
  -- -D warnings` and Rust formatting passed.
- Frontend: **1 051 tests in 68 files passed**; lint, app/E2E typechecks and production build passed.
  The final focused route-error tests also passed after correcting their TypeScript route shape.
- The real PostgreSQL/API/engine/Chromium command below passed **24 journeys, zero failures**;
  **two optional performance benchmarks skipped and US02 excluded**. The new journey injects a
  stylesheet 404, checks recovery focus/320px reflow/axe WCAG AA and no exception text, then restores
  the asset, reloads, signs in again and opens Workflows without browser errors.
- The control-plane binary built successfully. Its local replacement was first checked on separate
  ports with the existing configuration, then started on 18080. A fresh Chromium visit to
  `/studio/workflows` showed sign-in without failed asset responses or console/page errors.

```sh
SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55432/postgres \
SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0 \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 \
cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

This fixes the serving/UI contracts in the [runbook](../../docs/runbooks/studio.md); REST schemas,
tenant checks, operation audit and OTel contracts are unchanged. Browser telemetry remains G159.
No sandbox, performance, full-workspace Rust or non-Chromium validation is claimed for this fix.

### Create a namespace in Studio (T007-052, 2026-10-12)

Sign in with `namespace:create` (the local dev administrator has it). Open **Namespaces** in the sidebar menu, then click **New namespace**, choose a workspace, enter a unique slug and choose Development,
Staging or Production. Submit **Create namespace**: the new namespace becomes selected. A duplicate
name leaves the dialog open and asks for another name. If there are no workspaces, create one through
`POST /v1/workspaces` first. Refresh an already open Studio tab to load the new action; sign in again
if required (the token is held only in memory).

Validation: the new shell regression failed before implementation; 10 focused Vitest tests passed,
along with lint, TypeScript checking/production build and the real Chromium/API journey
`frontend/e2e/tests/namespaces.spec.ts` (creation, selection, persistence after login, duplicate and
375px dialog). Full-suite run: 1,059 passed and six failed in concurrent graph-layout work
(`tokens`, `RunDetailPage`, `operations`); these are not reported as passing namespace validation.

## Graph text and inspector correction — 2026-10-12 (T007-051)

The owner's Temporal/Windmill direction is recorded in the [design mapping and captures](design/README.md#graph-reading-and-selection-2026-10-12-t007-051).
Cards now separate identity, type, state, badges and ports; selection exposes complete details.
Duration amounts and units have independent space, and the initial camera includes the top of the
short trigger and taller entrypoint. Fixed variant dimensions preserve layout and live-update positions.

Observed regressions before the fixes: 14 editor glyph/overlap failures, a 22px Timeout amount,
clipped runtime selected-step identity, and a first-step title above the initial canvas boundary.
The added browser checks failed on those actual geometries before passing on the final build.

Validation of the shared working tree:

- **1 066 Vitest tests in 70 files passed**; ESLint, application/E2E TypeScript, production build
  and generated OpenAPI consistency passed. This includes the concurrent namespace increment.
- Real PostgreSQL/API/engine/Chromium suite: **27 passed, zero failed**, two opt-in performance
  tests skipped in the journey run. US02 was explicitly excluded with `SAGAT_STUDIO_NO_SANDBOX=1`.
- `graph-text-layout.spec.ts` measures DOM Range glyph bounds and collisions for triggers,
  Activity/Timer/Condition/Parallel, initial entrypoint framing, LR/TB, light/dark/forced colours,
  and blocked Geist fonts. The 320px inspector accepts twelve visible duration digits without
  colliding with the unit. A real compensated run exercises long identity/type/state, saga, failure,
  undo and keyboard-selected details. Browser IDs are 64 characters (66 for derived compensation);
  component tests additionally cover 80-character and arbitrary runtime values.
- The run journey verifies attempts in the selected panel while preserving live-update, graph-read
  count and journal assertions. Table assertions are scoped to the steps table, since selection
  also displays an attempts table.
- UX and frontend agents reviewed the code and actual captures. The remaining initial-camera
  defect was fixed and rechecked at 1:1. Vertical overview captures are labelled as overviews.
- The already-running server returned the current built HTML and all seven CSS assets with HTTP
  200 after replacement; no control-plane restart was needed.
- Both performance scenarios passed separately, three cold rounds each: median first-card time
  **978ms for 1 000 editor nodes** and **1 425ms for 9 991 runtime nodes**; median pan/zoom frames
  16.7ms, peak JavaScript heap 215MB. All six existing floors held; see the regenerated
  [benchmark report](../../tests/performance/reports/workflow-studio.md).

```sh
STUDIO_PERF=1 STUDIO_PERF_OUT="$PWD/frontend/e2e/perf-results/perf.json" \
SAGAT_STUDIO_GREP=PERF SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55432/postgres \
SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0 \
CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0 \
cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

Reproduce the integrated run using the T007-050 command above. The wire/API, transaction,
tenant/RBAC, audit and OTel contracts are unchanged; browser telemetry remains G159.
No sandbox execution, non-Chromium testing or new human acceptance is claimed here.

Owner navigation correction: namespace management now has its own **Namespaces** menu and
`/studio/namespaces` page. The page lists tenant namespaces and environments, marks the current
one and offers **Use namespace** and authorized **New namespace**. The sidebar selector remains.
Validation after this correction: all 1,066 Vitest tests passed (70 files), lint, app/E2E typechecks,
production build and the real Chromium/API namespace journey passed. This newer full-suite run
supersedes the six concurrent graph-layout failures recorded above.

Owner navigation correction (T007-058): the sidebar is a hierarchy. On a first visit it offers only **Namespaces** and the selector
(*Choose a namespace*); choosing a namespace, in the selector or with **Use namespace**, opens Workflows, Runs, Operations, Scripts
and Integrations under it. Nothing is chosen for you, even with a single namespace; a page of a namespace opened without one says
*Choose a namespace*. 1,145 Vitest tests passed (78 files), lint, typecheck, build and `check:api` too, and the built Studio was driven in Chromium
at 1470, 390 and 320 px. The Playwright journeys were not run (they need the Rust platform); `signIn` in `frontend/e2e/support/studio.ts`
now chooses the first namespace, and `smoke.spec.ts` carries the hierarchy journey.

### Configure a broker through Integrations (T007-053)

Open **Integrations** for the selected namespace. An administrator can expand **Network access**
and explicitly allow the broker hosts/networks and ports, preserving any rules other integrations
need. The policy applies namespace-wide. Private addresses require an explicit CIDR; platform
network restrictions still apply. Users without `egress:manage` ask an administrator to configure it.

Choose **New integration**, a name and Kafka, NATS JetStream or Amazon SQS. Enter servers and
specific settings (Kafka topic/initial offset; NATS stream/durable consumer/optional subject filter;
SQS queue URL/region). Choose no credentials, an existing namespace secret, or enter the supported
credentials with `secret:manage`. Kafka supports SASL PLAIN/SCRAM-SHA-256/SCRAM-SHA-512;
NATS supports a token, username/password or complete credentials-file text; SQS supports access
keys and an optional session token. Enter TLS addresses for credentials. Custom TLS CA/client
certificate configuration is not exposed by the existing adapters and is not implied by this form.

Save initially **Paused**, then **Edit** and choose **Active** when ready to consume. Active is a
configuration state, not a connectivity test. Review runtime errors/message receipt on the list.
Bind a workflow message trigger to the integration name using feature 004's existing trigger contract.
The current page manages inbound connectors; outbound event subscriptions remain their separate API.
The internal Sagat NATS uses deployment `SAGAT_NATS_URL`/`SAGAT_NATS_CREDENTIALS_FILE` settings.

Credentials are encrypted by the existing vault, write-only in Studio and referenced by name.
After a partial save the form explains that credentials were saved and retries only the connector.
No automatic deletion of unused secrets occurs. After replacing credentials, pause and resume an
active connector to ensure that its reader reloads them, per the events runbook.

Validation: 11 focused component tests passed (Kafka/NATS/SQS shapes, secret separation and failure
redaction, partial-save retry, permissions, namespace changes, updates/activation and explicit network
policy edits). A real Chromium/API journey passed: isolated namespace, explicit network policy,
Kafka synthetic credential save/edit, NATS save, new login/persistence, no credential redisplay or
browser persistence and a 375px dialog. Test connectors remain paused; no connectivity to a user's
broker or production credential was claimed or exercised.

Final T007-053 gates: 1,077 frontend tests in 71 files, lint, app/E2E typechecks, production build,
generated OpenAPI consistency and the real Chromium/API journey passed against the rebuilt Studio.

### Open a run trace in the deployment's tracing tool (T007-055)

Set an absolute URL template when starting the control plane, for example:

```sh
SAGAT_STUDIO_TRACE_URL='https://traces.example.test/explore?trace={traceId}'
```

Exactly one literal `{traceId}` is required. HTTPS is required except for loopback development
tools such as `http://localhost:16686/trace/{traceId}`. URL credentials, relative URLs, other
schemes and remote plain HTTP fail process configuration. The value is public runtime config; do
not put a token or password in it.

Open a run, expand **Run details**, and choose **Open trace**. The Studio substitutes only a
nonzero 32-hex trace id, opens a new separated tab and does not prefetch the tracing tool. Without
the setting, the trace id and **Copy trace id** remain and no link is shown. Changing the setting
requires a control-plane restart to reread its environment, but no frontend rebuild.

Validation on 2026-10-12: all 15 static Studio serving tests and all 34 control-plane config tests
passed with the shared target directory. Seventy-six focused run/config/link Vitest tests passed,
as did focused ESLint, Studio and E2E TypeScript checks, and the production build. Both scenarios
in `frontend/e2e/tests/trace-links.spec.ts` passed in real Chromium against the actual API/UI with
runtime `/studio/config.json` interception: exact safe link attributes without an automatic
external request, and the no-config copy fallback. Browser log:
`target/usability/trace-browser.log`.

## Completion follow-up — namespaces and signals (2026-10-12)

T007-054: `src/api/endpoints.test.ts` passed 3/3 including 201 namespaces, a later-page failure
and abort propagation. T007-056: `src/features/runs/SendSignal.test.tsx` passed 8/8 including
permission/terminal state, explicit confirmation, UTF-8 byte bound, stable retry keys and stale
responses. `frontend/e2e/tests/signals.spec.ts` passed 1/1 against the running real local API and
engine: publish a waitEvent workflow, start WAITING, signal through Studio, observe SUCCEEDED
and the persisted `wait.satisfied` journal event. The first browser assertion used an incorrect
invented event name and failed; it was corrected against the domain's canonical event kind and
rerun successfully. Logs: ignored `target/usability/signal-{tests,browser}.log`.

Integrated completion check (2026-10-12): all 1,104 Studio unit tests, full ESLint, TypeScript checking and the production build pass. The signal form also discards its payload after same-subject reauthentication, covered by a failing-first regression.

## Integration of committed UX work — 2026-10-25

The seven commits through `fad431fd` are integrated with features 016/017. Namespace-gated
navigation retains Connections, Secrets and Ad hoc database; run origin labels/private-version
handling remain intact beside address filters and Start run. Merge review found two defects:
unsafe numeric input in Start run and Origin not being cleared. New regressions were observed
failing before the fixes; exact-number authoring and all-filter clearing now pass.

Current gates, macOS arm64 with the repository's Testcontainers/Colima fixture and a private
Cargo target:

- `npm test -- --maxWorkers=2`: **1,193 passed in 87 files**, zero failed (30.42 s, final graph fixes).
- `npm run lint`, `npm run typecheck`, `npm run typecheck --workspace @sagat/e2e`,
  `npm run build`, `npm run check:api`: passed. TypeScript SDK preparation is required before
  E2E typechecking; the first attempt correctly failed because that SDK had not been built.
- `npm audit --audit-level=moderate`: **0 vulnerabilities**.
- `SAGAT_STUDIO_REBUILD=0 SAGAT_STUDIO_NO_SANDBOX=1 cargo test -p sagat-tests --test
  e2e_workflow_studio --locked -- --ignored journeys --nocapture`: **35 Chromium scenarios
  passed; zero failed; two opt-in performance skips** (101.20 s including harness). US02's
  script sandbox is explicitly excluded by this local mode; Linux CI must prove it.

The initial browser run had two failures (33 passed, 2 failed, 2 skips): the taller graph put
bottom controls below the viewport, and a selected panel requested a deliberately blocked font
weight after console observation began. Controls now occupy the visible top-right corner;
the font-failure fixture settles every declared weight before strict unfiltered console checks.
The original geometry, glyph, fallback-font and console assertions remain. Fresh independent
reviews closed with no remaining C1/C2 findings. The new Start run journey validates malformed
input, then verifies the real API's retained run, actor, workflow, navigation and successful
completion. No new human visual or screen-reader acceptance is claimed.
