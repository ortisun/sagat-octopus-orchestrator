# Runbook: the Studio (feature 007)

For the people who run the platform and for the people who use the Studio. Design: [ADR-0016](../adr/ADR-0016-the-studio-is-a-same-origin-app-over-the-rest-api-that-follows-runs-by-journal-sequence.md);
decisions and gaps: [`specs/007-workflow-studio/research.md`](../../specs/007-workflow-studio/research.md); the contract: the operations
marked `x-implemented: '007'` in `specs/001-platform-foundation/contracts/openapi.yaml`.

## 1. What it is

A single-page application (React, TypeScript) that the control plane serves under `/studio/` from a directory of files, on the same origin
as the API. It has three parts: **Workflows** (a canvas and a list to build a workflow without programming, a validator that is the
platform's own, a diff against the active version before publishing), **Runs** (the execution graph, a table, the journal as a timeline,
live; the operator's decisions) and **Scripts** (a code editor, the sandbox test, publishing a version). It keeps no data of its own: it
reads and writes what the API already offers.

## 2. Serving it

Build once (`npm ci --ignore-scripts && npm run build` in `frontend/`; the output is `frontend/apps/studio/dist`) and point the control plane at it:

| Variable | Meaning |
|---|---|
| `SAGAT_STUDIO_DIR` | The directory of the build (it must hold `index.html`). Unset: the Studio is not served and `/studio/` is a `404`. The process refuses to start if the directory has no `index.html`. |
| `SAGAT_STUDIO_OIDC_ISSUER`, `SAGAT_STUDIO_OIDC_CLIENT_ID`, `SAGAT_STUDIO_OIDC_SCOPE` | Optional. Sign-in with your identity provider (authorization code with PKCE, a *public* client: it has an id and no secret). The issuer must be an `https` URL (an `http` one only on a loopback address) without credentials; the client id and the issuer must both be set or neither. The default scope is `openid profile`. The redirect URI to register at the provider is `https://<studio host>/studio/auth/callback`. |

Without the OIDC variables the sign-in is a token the user pastes (development and automation). In both cases the token is held in the
page's memory only, ends at its expiry, and is gone when the page is reloaded.

What the server does for the files (`crates/api-http/src/studio.rs`, tested in `crates/api-http/tests/studio.rs`):
a path with no extension that matches no file gets the page (the app's routes); a missing *file* is a `404`; dot segments, backslashes, NUL, hidden
files and links that resolve outside the directory are not served; assets under `assets/` are `immutable`, the page is `no-store`; every answer carries
`Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'
[+ the issuer's origin]; worker-src 'self' blob:; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`, `nosniff`, `Referrer-Policy: no-referrer`,
`Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Resource-Policy: same-origin`. Behind a reverse proxy: pass these headers through, and do not
buffer `text/event-stream` (the stream sends `X-Accel-Buffering: no`). A Studio on another origin than the API is not supported by the API (it has no CORS);
put both behind one host.

## 3. Using it (short)

- **Sign in**: paste a token, or use your organisation's single sign-on. The header shows who you are and your role; what you cannot do is not hidden but explained.
- **Build a workflow**: add steps from the palette (or the *Add node* menu with the keyboard), connect them (drag, or choose in the inspector), set their fields. The checks of the
  platform run as you work and are listed with the node each is about; advice the platform does not give is listed apart. The draft is saved by itself; if someone else saved first you are
  shown the difference and nothing is overwritten without it. *Publish…* shows what changes against the active version and which changes can break callers.
- **Watch a run**: the graph shows each step's state, attempts and the way the run took; the *Table* tab has the same data as a list; the *Timeline* is the journal. A banner tells how the
  view is kept current (live, polling, reconnecting).
- **Act on a failure**: choose the step, *Retry*, *Retry compensation*, *Skip compensation* or *Resolve*; each asks for a reason, says what it will do, and is recorded with your name.
  Skipping or resolving needs the administrator permission `saga:override`.
- **Everything the canvas does, the list view does** (*Canvas | List* toggle), for keyboard and screen-reader use.
- **A reader** (`definition:read` without `definition:write`) can open every workflow but is not sent to the platform's checks, which answer only people who can change a workflow; the
  validation panel says so. The same reader on a run sees every action, disabled, with the permission it needs.

## 4. Operating it

| Concern | What to look at |
|---|---|
| Open streams | `run_streams_open` (gauge) and `run_streams_refused_total` (counter, a stream was refused because the subject or the tenant is at its cap: 20 per subject, 200 per tenant). A client that is refused polls instead (`GET /runs/{id}/graph/changes`), which costs the platform less than a stream per tab. |
| Cost of a stream | One small query per half second while a run moves, per second quietly, until the run ends, the token expires or 15 minutes pass (G143). Raising the caps raises database load roughly in proportion. |
| A stream that "never connects" | A proxy that buffers or times out `text/event-stream`; the Studio falls back to polling after three failures, so it works but is less prompt. |
| Sessions | A token's life is the platform's (at most one hour). When it ends the Studio says so and asks again; nothing is stored to refresh it. |
| Audit | Reads are not audited; every decision on a run is, with the reason; publishing and draft changes are, as before. |

## 5. Accessibility

WCAG 2.2 AA is the target (D148, G146): every action has a keyboard route, status is never only a colour, dialogs are native, changes are announced, motion respects
`prefers-reduced-motion`, and `forced-colors` is supported. It is checked by automated runs of axe-core over the main pages in the browser journeys and by a keyboard
walkthrough in them; it has **not** been tested by people using a screen reader. Report a barrier as a defect.

## 6. What is not built

Showing a step's input, output or error text (the journal carries none: G144); *retry run as new* and *mark manual review* (the API refuses them: G145); human approval and
for-each nodes can be drawn but not published (G147); manual positions of cards are kept in the browser only (G148); pushing changes from the engine instead of polling
the database (G143); a screen-reader test by a person (G146).

## 7. Checking it

| What | How |
|---|---|
| Lint, types, unit tests, build, generated API types | in `frontend/`: `npm run lint`, `npm run typecheck`, `npm run typecheck --workspace @sagat/e2e`, `npm test`, `npm run build`, `npm run check:api` |
| The journeys (US01..US04, SC-007-001, accessibility, security, sign-in) | `cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture`; needs Node, the frontend's dependencies (`npm ci` in `frontend/`), Chromium (`npx playwright install chromium` in `frontend/e2e/`) and Docker. `SAGAT_STUDIO_GREP="US04"` plays one; `SAGAT_STUDIO_REBUILD=0` reuses the build in `frontend/apps/studio/dist`. A failure leaves screenshots and traces in `frontend/e2e/test-results/`. |
| The benchmark | `STUDIO_PERF=1 STUDIO_PERF_OUT=$PWD/frontend/e2e/perf-results/perf.json SAGAT_STUDIO_GREP=PERF cargo test -p sagat-tests --test e2e_007 -- --ignored journeys --nocapture`, then `node frontend/e2e/scripts/perf-report.mjs frontend/e2e/perf-results/perf.json tests/performance/reports/007-workflow-studio.md`. The floors it asserts are in `frontend/e2e/tests/perf.spec.ts`; the report says what it does not measure. |
| Dependency advisories | `npm audit --audit-level=moderate` in `frontend/` (CI runs it; two low advisories are accepted: G152) |

