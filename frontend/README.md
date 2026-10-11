# The Studio (feature 007)

A React and TypeScript single-page app that is served by the control plane under `/studio/` (same origin as the REST API, so
no CORS), with a strict content security policy. It builds and publishes workflows, edits and tests scripts, and follows runs and
acts on them. Why it is built this way: [ADR-0016](../docs/adr/ADR-0016-the-studio-is-a-same-origin-app-over-the-rest-api-that-follows-runs-by-journal-sequence.md);
what it decided and what it does not do: [`specs/007-workflow-studio/research.md`](../specs/007-workflow-studio/research.md);
how to run and operate it: [`docs/runbooks/studio.md`](../docs/runbooks/studio.md).

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
cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture
```

`docs/runbooks/studio.md` §7 has the rest (one journey, a build that is reused, the benchmark).

## Rules this code keeps

- The platform decides; the page asks it and shows what it said (validation, the diff of two versions, what an operator may do).
- A token is held in memory and sent to the page's own origin only. Text that comes from outside (a name, a description, a
  correlation id, a failure code) is rendered as text and clipped; there is no `dangerouslySetInnerHTML`.
- What a person cannot do is shown, disabled, with the reason and the permission it needs.
- Everything the canvas does the list view does, from the keyboard.
