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
  src/ui/           the Studio's interface module: its own components, built on the Astryx design system (ADR-0101); the only directory that imports Astryx
  src/ui/theme/     the `sagat` Astryx theme: its colours (`roles.ts`, the only file with a literal colour) and how they map to the design system (`sagatTheme.ts`)
  src/styles/       stylesheets for the surfaces Astryx has no component for (canvas, graph, timeline, code editor), on the theme's tokens
e2e/                Playwright journeys against a real platform, and the benchmark
```

## Screens

The interface is built on the Astryx design system with the Studio's own theme, `sagat`
([ADR-0101](../docs/adr/ADR-0101-the-studio-builds-its-interface-on-the-astryx-design-system.md)). It follows the
colour scheme of the operating system: dark and light are the same theme.

These captures were taken on 2026-11-21 from the development server at 1440 by 900 pixels, over a mock of the REST
API with sample data (a payments tenant), because no platform was running on the machine that took them. They show
the screens, not a real deployment: no run, worker or secret in them exists.

| | |
|---|---|
| **Sign in**: a pasted token, or single sign-on when it is configured<br>![Sign in](docs/screens/dark-sign-in.png) | **Namespaces**: the tenant's namespaces; one is chosen to open its menu<br>![Namespaces](docs/screens/dark-namespaces.png) |
| **Workflows**: the workflows of the namespace, with their active version<br>![Workflows](docs/screens/dark-workflows.png) | **Workflow editor**: the palette, the canvas, the inspector, validation and versions<br>![Workflow editor](docs/screens/dark-workflow-editor.png) |
| **Scripts**: the scripts of the namespace and their languages<br>![Scripts](docs/screens/dark-scripts.png) | **Script editor**: the source, the sandbox test, schemas, versions and limits<br>![Script editor](docs/screens/dark-script-editor.png) |
| **Runs**: the list, with filters by status, origin and workflow<br>![Runs](docs/screens/dark-runs.png) | **A failed run**: the list beside the detail, with the graph, the table and the timeline<br>![A failed run](docs/screens/dark-run-failed.png) |
| **A running run**: followed live<br>![A running run](docs/screens/dark-run-running.png) | **Operations**: alerts and worker identities, with drain and resume<br>![Operations](docs/screens/dark-operations.png) |
| **Ad hoc database**: one statement against a managed connection<br>![Ad hoc database](docs/screens/dark-database.png) | **Connections**: managed database connections and their probe<br>![Connections](docs/screens/dark-connections.png) |
| **Secrets**: metadata only, never a value<br>![Secrets](docs/screens/dark-secrets.png) | **Integrations**: connectors and network access<br>![Integrations](docs/screens/dark-integrations.png) |

The light theme, on three of the same screens:

| | | |
|---|---|---|
| ![Runs, light](docs/screens/light-runs.png) | ![A failed run, light](docs/screens/light-run-failed.png) | ![Workflow editor, light](docs/screens/light-workflow-editor.png) |

## Configuration

The Studio has no configuration file of its own: it is a directory of files that the control plane serves, and
what it needs to know at page load it reads from `/studio/config.json`, which the control plane answers from its
environment. Everything is set on the control plane.

| Variable (control plane) | Meaning | Default |
|---|---|---|
| `SAGAT_STUDIO_DIR` | The directory of the build (`frontend/apps/studio/dist`); it must hold `index.html`. | Unset: the Studio is not served and `/studio/` answers `404`. |
| `SAGAT_STUDIO_OIDC_ISSUER`, `SAGAT_STUDIO_OIDC_CLIENT_ID` | Sign-in with an identity provider (authorization code with proof key, a public client: an identifier and no secret). Both or neither. The issuer is an `https` address (plain `http` only on a loopback address). | Unset: the sign-in is a token the person pastes. |
| `SAGAT_STUDIO_OIDC_SCOPE` | The scope asked of the identity provider. | `openid profile` |
| `SAGAT_STUDIO_TRACE_URL` | A public address template with exactly one `{traceId}` (for example `http://127.0.0.1:16686/trace/{traceId}` for a local Jaeger). Run details then offer *Open trace*. `https` outside a loopback address; no credentials in it. Restart the control plane after changing it; the Studio is not rebuilt. | Unset: only *Copy trace id* is offered. |
| `SAGAT_STREAMS_MAX`, `SAGAT_STREAMS_PER_TENANT`, `SAGAT_STREAMS_PER_SUBJECT` | How many live run streams may be open in the process, per tenant and per person. A person over the limit is served by polling instead. | the lesser of 2,000 and half of `SAGAT_MAX_CONNECTIONS`; 200; 20 |
| `SAGAT_WRITE_STALL_TIMEOUT_SECS` | How long the server waits for a client that stopped reading before it closes the connection. | 30 |

For development only, the Vite server reads one variable of its own:

| Variable (development server) | Meaning | Default |
|---|---|---|
| `SAGAT_API_URL` | Where `npm run dev` forwards `/v1`. | `http://127.0.0.1:8080` |

What a person chooses is kept in their browser and nowhere else: the chosen namespace (its slug, under the key
`studio.namespace`) and nothing about the session. The access token lives in the page's memory only: it ends at
its expiry and is gone when the page is reloaded.

The look is configured in source, not at run time: the colours, typefaces and radii are the `sagat` theme
(`apps/studio/src/ui/theme/roles.ts` for the colours, `sagatTheme.ts` for how they map to the design system). After a
change nothing is rebuilt: the theme is applied from its source when the application starts. `npm test` fails when
a colour pair falls under its contrast floor or a role is left unmapped. To force a colour scheme for a
page whatever the system says, set `data-theme="light"` or `data-theme="dark"` on the `html` element.

## Operation

1. **Build.** In `frontend/`: `npm ci` (exact versions, no install scripts), then `npm run build`. The result is
   `apps/studio/dist`.
2. **Serve.** Start the control plane with `SAGAT_STUDIO_DIR` pointing at that directory. The Studio is then at
   `/studio/` on the API's own host; it is not supported on another origin (the API has no cross-origin access).
   Behind a reverse proxy, pass the security headers through and do not buffer `text/event-stream`.
3. **Sign in.** Paste a token issued for you, or use *Sign in with your organisation* when single sign-on is
   configured. The top bar shows who you are and your role.
4. **Choose a namespace.** On a first visit nothing is chosen and the Studio opens the namespace list: pick one in
   the selector of the sidebar or with *Use namespace*. Workflows, Scripts, Runs, Operations, Ad hoc database,
   Connections, Secrets and Integrations then appear under it.
5. **Work.** Build and publish a workflow in the editor (canvas or list); edit, test and publish a script; follow a
   run in its graph, table or timeline and, with the permission, retry, compensate, cancel or resolve a step, each
   with a reason that is recorded; drain or resume a worker identity; manage connections, secret references and
   connectors. An action you may not take is shown unavailable with the permission it needs, not hidden.
6. **Deploy a new build.** Build in a separate directory, publish the new hashed files under `assets/` first, then
   replace `index.html` atomically, and keep the previous hashed files while tabs opened before the change are
   still in use. New page requests see the new build without restarting the control plane.

Three things work differently from a plain web form, because the design system draws the controls: a selector opens
a list of options instead of the operating system's picker (type to move, Enter to choose, Escape to close); in a
strip of tabs the arrow keys move the focus and Enter or Space selects; and below 768 pixels of width the sidebar
becomes a menu behind the *Open navigation* button.

What to watch, the limits of live streams, the audit of decisions and the accessibility statement are in
[`docs/runbooks/studio.md`](../docs/runbooks/studio.md), sections 4 and 5.

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
