# The Studio (feature 007)

A React and TypeScript single-page app that is served by the control plane under `/studio/` (same origin as the REST API, so
no CORS), with a strict content security policy. It builds and publishes workflows, edits and tests scripts, and follows runs and
acts on them. Why it is built this way: [ADR-0016](../docs/adr/ADR-0016-the-studio-is-a-same-origin-app-over-the-rest-api-that-follows-runs-by-journal-sequence.md);
what it decided and what it does not do: [`specs/007-workflow-studio/research.md`](../specs/007-workflow-studio/research.md);
how to run and operate it: [`docs/runbooks/studio.md`](../docs/runbooks/studio.md).

The server reads current entry HTML after a completed build replacement. Publish hashed assets
before replacing the HTML and retain old assets during rollout, as described in the runbook.
The eager route error page offers **Reload Studio** when a page cannot load, with an explicit
warning about unsaved edits and signing in again. It does not automatically reload or show stack
traces. `RouteErrorPage.test.tsx` and the missing-stylesheet journey in `smoke.spec.ts` cover recovery.

```text
apps/studio/        the app (Vite, React 19, React Flow, Monaco, TanStack Query)
  src/api/          the REST client, the types generated from the OpenAPI contract, the live follower (SSE with polling as fallback)
  src/auth/         sign-in with a pasted token or an OIDC provider (code flow with PKCE); the token lives in memory only
  src/model/        the workflow document and what is done to it (registry of node kinds, edits, layout, advice), the run graph
  src/features/     workflows (editor), runs (graph, table, timeline, decisions), scripts (code editor, test runs)
  src/ui/           the small set of accessible components everything is made of
e2e/                Playwright journeys against a real platform, and the benchmark
```

## Commands

```text
npm ci                                   # exact versions from the lockfile; no install scripts
npm run lint && npm run typecheck && npm test
npm run build                            # apps/studio/dist, what the control plane serves (SAGAT_STUDIO_DIR)
npm run api                              # regenerate src/api/schema.d.ts from specs/001-platform-foundation/contracts/openapi.yaml
npm run check:api                        # fails if schema.d.ts is not what the contract generates
npm run dev                              # Vite on :5173, proxying /v1 to the control plane (SAGAT_API_URL, default http://127.0.0.1:8080)
```

The journeys are played by the Rust harness, which starts PostgreSQL, the API (serving the build), the engine and a sandbox
runner, and then runs Playwright with the addresses and tokens it made:

```text
npx playwright install chromium         # once, in e2e/
cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

`docs/runbooks/studio.md` §7 has the rest (one journey, a build that is reused, the benchmark).

For development without a working sandbox container engine, the harness has an explicit **partial** mode. Use an existing
PostgreSQL through `SAGAT_TEST_DATABASE_URL` if Docker cannot start PostgreSQL either:

```sh
SAGAT_STUDIO_NO_SANDBOX=1 cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture
```

This runs the real API, engine and browser, excludes **US02**, and starts no sandbox runner. It cannot prove script build/test
execution. The default command and CI still require real Docker containers; this flag is not a substitute for their gate.

## Visual direction

The [recorded design](../specs/007-workflow-studio/design/README.md) supplies the dark palette, type, spacing and shapes. The
Studio follows the system's light/dark preference and supports forced colours. Geist and Geist Mono `5.3.0` are bundled on the
same origin with their SIL OFL licence in `apps/studio/public/licenses/geist-OFL.txt`; system fonts remain the fallback.

The sidebar follows the hierarchy: **Namespaces** (the tenant's own page), the namespace selector and, under it and only while a
namespace is chosen, Workflows, Runs, Operations, Scripts and Integrations. Nothing is chosen for you; a page of a namespace opened without
one says *Choose a namespace*. On wide screens, `/runs/:runId` keeps a filtered,
virtualized Runs list beside the selected run; choosing another run retains the list's state. Narrow screens keep the separate
list and detail routes. The Runs filters are query parameters (`status`, `workflow`) and the Workflows search is `q`, so links carry
them. Facts come from the API: missing terminal duration is unknown, and queue, worker, trigger-kind and
global counts are not invented. Graph, Table and Timeline remain available; Timeline means the journal, not per-step bars.

Graphs open at a readable 1:1 scale on their starting node; **Fit View** shows the whole graph on
request. Run **details** keeps technical identifiers available without crowding the graph. The
editor reflows its palette before squeezing the canvas, and populated Scripts tables scroll inside
an accessible region on small screens. The `VISUAL` browser journeys measure these layouts at
320, 768, 1024, 1280 and 1470px, in addition to the accessibility suite.

The workflow editor, Scripts, sign-in and light theme are derived proposals. The owner approved the visual proposal on 2026-10-11. Current test results and remaining gaps are in
[the validation record](../specs/007-workflow-studio/quickstart.md#visual-direction-validation--2026-10-11). The passing browser suite's
[ten review captures](../specs/007-workflow-studio/design/README.md#owner-review-captures--2026-10-11) are available for the owner.

## Rules this code keeps

- The platform decides; the page asks it and shows what it said (validation, the diff of two versions, what an operator may do).
- A token is held in memory and sent to the page's own origin only. Text that comes from outside (a name, a description, a
  correlation id, a failure code) is rendered as text and clipped; there is no `dangerouslySetInnerHTML`.
- What a person cannot do is shown, disabled, with the reason and the permission it needs.
- Everything the canvas does the list view does, from the keyboard.
