# Design direction — Sagat Octopus Studio

The visual direction the Studio follows (FR-007-009, D154). The design lives in a Claude Design project that its owner may
keep editing, so this folder is a **dated snapshot** and the Studio is built from it, not from the project: a change in the
project reaches the Studio by updating this record first.

| | |
|---|---|
| **Source** | Claude Design project `4dd415c2-a211-4bd1-803b-3dafcfa11303`, file `sagat-octopus.dc.html` (`https://claude.ai/design/p/4dd415c2-a211-4bd1-803b-3dafcfa11303?file=sagat-octopus.dc.html`) |
| **Snapshot** | the owner's export of 2026-10-10 (`Sagat-Octopus design system.zip`, sha-256 `7049b3b1…1d1447`): `sagat-octopus.dc.html`, `Logo sagat-octopus.dc.html`, `Analise Temporal x Windmill.dc.html`, `support.js` (the design tool's own runtime) and a thumbnail |
| **Kept here** | [`source/sagat-octopus.dc.html`](./source/sagat-octopus.dc.html) (sha-256 `10dd8b13…a0a87`), the Runs prototype: the **only** file the Studio's tokens are read from; and [`runs-prototype.jpg`](./runs-prototype.jpg), a capture of it. Neither is served or built: the page has sample data and loads a remote font and a script, which the Studio's CSP forbids |
| **Not used** | the logo page (the choice is not made, G162) and the Temporal × Windmill analysis (a market comparison, not a visual specification) |
| **History of this record** | first written from a capture of the running prototype (colours sampled from a JPEG); rewritten the same day from the export. **A correction:** the first version reported the list's secondary text at 3.98 : 1; the export shows its role is lightness 0.60, which is 4.98 : 1. The sample had hit an anti-aliased pixel |

## What is exact and what is not

**Exact** (read from the source, in `oklch`, converted to sRGB by the standard formulae and rounded): the dark theme's colours
and their roles, the fonts, the type scale, the radii, the spacing and sizes of the Runs screen, the graph, the three
tabs it adds to the Graph tab and the prototype's three tweaks.

**Not in the design** (so not written here, G160): the **light theme**; focus rings and disabled states; motion (the prototype
has none, only hover colours); loading and error states beyond one empty message (*Nenhuma execução encontrada.*); every screen other
than Runs (G161); how it behaves below 1180 px (the prototype sets `min-width: 1180px`).

## What the prototype draws

One screen, **Runs**, in dark, as three regions:

1. **Sidebar** (216 px): the mark and the name (*sagat-octopus*, with a tagline), a **namespace** selector (monospace), a
   navigation (Workflows, Runs, Schedules, Workers, Task queues, Secrets, Settings; a count beside most) and, at the bottom, a
   box with active workers (a bar) and the queue backlog.
2. **Top bar** (56 px): a breadcrumb (namespace / page), a 300 px search with a ⌘K hint, the primary button (*New workflow*).
3. **Master–detail**: a **list** (392 px) with a title, a live indicator (*Live* / *Paused*), five filter chips with counts and rows
   (status dot, workflow name in medium weight, duration, short run id, start time · trigger; the selected row is lighter and has a 2 px
   accent bar on its inside left edge); and the **detail**: name, a status badge, ids, the buttons (*Send signal* when it is waiting,
   *Cancel* while it runs or waits, *Re-run*), a strip of five facts, four tabs and the selected tab's content:
   - **Graph**: a dotted canvas with the DAG, and a strip for the **selected step** (name, status, kind, attempts `n / 3`, start offset,
     duration, and a red box with the failure text for a failed step);
   - **Timeline**: one row per step with a bar on a 0–100 % axis (quarters marked), the step's duration at the right;
   - **Event history**: a table (`#`, time, event, details) with a coloured dot per event type;
   - **Input / Output**: two panes with the run's JSON input and its output (or the error).

**Tweaks:** `graphLayout` (`horizontal` | `vertical`, default horizontal), `liveUpdates` (default on), `defaultTab` (graph, timeline, events, io).

## Tokens (dark theme) — exact

All neutrals are hue 260; the accent is hue 160. `oklch(L C H)`; the sRGB value is the clamped conversion.

### Surfaces and lines

| Role | `oklch` | sRGB | Used for |
|---|---|---|---|
| sidebar | `0.13 0.006 260` | `#06070a` | the sidebar |
| page | `0.15 0.006 260` | `#0a0b0e` | `body`, top bar, list, detail |
| canvas | `0.14 0.006 260` | `#08090c` | the graph's canvas |
| field | `0.17 0.007 260` | `#0e0f13` | search, namespace select, table headers (events, I/O) |
| raised | `0.19 0.007 260` / `0.19 0.008 260` | `#121417` | node card, selected list row |
| hover / active | row `0.18 0.007`, nav hover `0.19 0.007`, nav active `0.20 0.008`, ghost button `0.20 0.007` → hover `0.23 0.008`, card hover `0.21 0.008` | | |
| line, panel | `0.24 0.008 260` | `#1d1f23` | panel borders, dividers, fact strip |
| line, control | `0.26 0.008 260` (field, chip), `0.27` (chip, idle card), `0.28` (⌘K hint, scrollbar thumb), `0.30` (ghost button) | `#222428` … `#2b2e32` | borders of controls — **see the deviations** |
| line, inner | `0.20`–`0.22 0.007 260` | | list rows, timeline rows and quarters |
| dot grid | `0.26 0.008 260`, 1 px every 18 px | | the canvas |

### Text

| Role | `oklch` | sRGB | Used for |
|---|---|---|---|
| primary | `0.95 0.005 260` | `#eceff2` | |
| strong secondary | `0.85` (JSON, input pane) · `0.80` (durations) · `0.78` (event detail) · `0.70` (event time) | `#babec4` for 0.80 | |
| secondary | `0.68` (breadcrumb, sidebar labels) · `0.66` (inactive nav, tab, chip) | `#8f9298` for 0.66 | |
| tertiary | `0.62` (ids) · `0.60` (list meta, node kind) · `0.58` (**micro-labels**, uppercase) | `#7d8086` for 0.60 | |
| dim | `0.55` (counts, tagline, placeholder, ⌘K, event numbers) · `0.40` (breadcrumb `/`) | `#6e7278` for 0.55 | — **see the deviations** |

### Accent and status

| Role | `oklch` | sRGB |
|---|---|---|
| accent | `0.80 0.14 160` (hover `0.86 0.13 160`) | `#5ed99e` |
| on accent (button) | `0.18 0.03 160` (the mark `0.15 0.02 160`) | `#05160d` |
| accent text on its tint · link | `0.86 0.12 160` · `0.80 0.14 160` (hover `0.88 0.12 160`) | `#85e9b6` |
| accent tint · border · ring | the accent at **12 %** · **45 %** · **18 %** (a 3 px ring on the selected card) | |
| progress | `0.76 0.15 155` on a `0.24 0.008 260` track, 4 px | `#52cd86` |
| status: completed | `0.76 0.15 155` | `#52cd86` |
| status: running | `0.72 0.13 245` | `#56acf0` |
| status: failed | `0.68 0.19 25` | `#f75d59` |
| status: waiting for a signal | `0.80 0.14 80` | `#edb345` |
| status: pending, cancelled | `0.58 0.01 260` | `#777a80` |
| status tint (badge) · card border · timeline bar fill | the status colour at **12 %** · **45 %** · **25 %** (bar border: solid) | |
| failure box | background red at 10 %, border red at 35 %, text `0.82 0.10 25` | `#ffaba3` |
| signal button | border amber at 50 %, fill amber at 12 %, text `0.86 0.12 80` | `#fac871` |

### Type, shape and size

- **Faces:** *Geist* (400, 500, 600) and *Geist Mono* (400, 500). The prototype loads them from Google Fonts; the Studio's policy
  (`font-src 'self'`, D153) cannot, so they are bundled (D159, G166). Fallbacks: the platform's system stacks.
- **Scale (px):** 20/600, −0.02 em (page title) · 15/600, −0.01 em (list title) · 14/600, −0.01 em (brand) · **13** (base) · 12 and 12.5
  (mono values) · 11.5 (list meta) · 11 (micro-labels: uppercase, +0.05–0.06 em) · 10.5 (tagline). Line height 1.6 in the JSON panes.
  In the Studio the sizes are in `rem`, so the browser's text-size setting scales them.
- **Radii (px):** 4 (⌘K hint) · 7 (nav item, mark) · 8 (fields, buttons) · 10 (node cards, fact strips, the sidebar box) · 12 (panels) · 999 (chips, badges) · dots 7 (list, card) and 6 (badge, live).
- **Sizes (px):** sidebar 216 · top bar 56 · list 392 · search 300 · detail padding 18 / 24 · node card 184 × 62 · node step 236 across, 80 down (horizontal); 104 across, 110 down (vertical).
- **Graph:** edges are cubic curves, 1.5 px; a completed edge is solid, the success colour at 60 %; an edge not yet taken is `0.34 0.01 260`, dashed `4 4`. The selected card has the accent border and ring; a card at rest has the status colour at 45 % (idle: the control line).

## Where the prototype and the Studio's floor disagree

FR-007-008 (WCAG 2.2 AA as a target, G146) is the floor (axe-core in the journeys, a visible focus, forced colours, the table
alternative to the graph). Contrast ratios are computed from the exact values (WCAG relative luminance; text 4.5 : 1, the
boundary of a control and a graphic that carries meaning 3 : 1). **Passes** with margin: primary and secondary text (5.4–17 : 1),
tertiary 0.60 (4.7–5.0 : 1), micro-labels 0.58 (4.59 : 1, thin), every link, accent and status colour as text or dot (4.6–11 : 1),
the button label on the accent (10.6 : 1), the failure and signal texts on their tints (9.9–10.6 : 1), the completed edge (4.1 : 1)
and the active chip's border (3.01 : 1, thin). **Fails** and what the Studio does (D155):

| Where | Prototype | Ratio | Studio |
|---|---|---|---|
| dim text: sidebar counts and tagline, search placeholder and ⌘K, event numbers | lightness 0.55 | 3.9–4.15 : 1 (4.5 needed) | lightness **0.60** (4.7–5.1 : 1) |
| border of a field, the namespace select, an idle chip, a ghost button | 0.26–0.30 | 1.2–1.4 : 1 (3 needed) | neutral lightness **0.50** (`#616368`, 3.2–3.4 : 1) |
| the edge not yet taken | 0.34 | 1.7 : 1 (3 needed; the dash is not enough, a graphic that carries the structure) | lightness **0.50**, dashed as in the prototype |
| omitted edge and its legend sample in the Studio | decorative divider colour at 80 % opacity | **1.15 : 1 dark**, **1.24 : 1 light** against the canvas | control border at full opacity: `#686b70` dark (**3.72 : 1**), `#78818d` light (**3.62 : 1**); keeps a dotted `2 5` pattern, distinct from pending `4 4` |
| the card's border at rest and its status tint | 0.27 / status at 45 % | 1.3 / 2.1–2.7 : 1 | **kept**: the card is identified by its text, and the state by the dot (≥ 4.6 : 1) **and a word**, in the selected-step strip and in the Table tab. T007-042 checks that no state is carried by colour alone and adds a pattern or a label if one is |
| panel dividers, the dotted grid, the selected row's fill, the ⌘K hint's border | 0.20–0.28 | ≈ 1.1–1.3 : 1 | **kept**: decorative (the row's accent bar, 10.5 : 1, and `aria-current` mark the selection) |
| `min-width: 1180px` | fixed | no reflow | **not adopted**: content reflows to 320 px (WCAG 1.4.10), D158 |
| the fonts' origin | Google Fonts | a policy violation | bundled locally (D159) |

Each deviation found later is added to this table with its ratio, and the Vitest test of T007-039 enforces every threshold from the
tokens, so a value cannot drift back.

## Mapping the labels (the prototype is in Portuguese; the Studio is in English, D156)

| Prototype | Studio |
|---|---|
| Execuções · Workflows · Novo workflow | Runs · Workflows · New workflow |
| Buscar por ID ou workflow · Ao vivo / Pausado | Search by ID or workflow · Live / Paused |
| Todas / Em execução / Aguardando / Falhou / Concluído | All / Running / Waiting / Failed / Succeeded |
| Aguardando sinal · Pendente · Cancelado | Waiting for a signal · Pending · Cancelled (the platform's own state names win where they differ) |
| Cancelar / Reexecutar / Enviar sinal | Cancel / Re-run (disabled with the reason, G145) / Send signal (G167) |
| Início · Duração · Gatilho · Fila · Worker | Started · Duration · Started by (G164) · — · — |
| Grafo · Linha do tempo · Histórico de eventos · Entrada / Saída | Graph · *(no counterpart: the bars per step, G168)* · **Timeline** (the Studio's tab of that name is the journal, the prototype's *Event history*) · **Payload structure inspection** (ADR-0028; raw values hidden) |
| Etapa selecionada · Tipo · Tentativas · Início · Duração | Selected step · Kind · Attempts · Started · Duration |
| Atividade · Workflow filho · Sinal · Timer · Loop | the Studio's node names (Step, Child, Wait, Timer, …) |
| *(not in the prototype)* | **Table**: the accessible alternative to the graph (FR-007-008); it stays, as a tab of its own |

## What the prototype shows that the platform cannot give (FR-007-010)

| Prototype element | Why it is not drawn as shown | What the Studio does |
|---|---|---|
| Queue and worker of a run | `Run` carries neither (OpenAPI `Run`: `startedBy`, `createdAt`, `finishedAt`, `deadline`, `traceId`, `correlationId`, `depth`, `parent`) | omits them (G164) |
| Trigger kind (API, Cron, Webhook, Manual) | the run records who started it (`startedBy`), not the kind of trigger | shows *Started by* (G164) |
| Counts in the navigation and the filters, over all runs | the API pages by cursor and has no count | no count that needs the whole set; a count of what is loaded may be shown if it says so (G163) |
| Active workers, queue backlog (sidebar) | no read API (features 004 and 010 own it) | omits the box (G163) |
| Schedules, Workers, Task queues, Secrets, Settings | pages of other features | no link to a page that does not exist (G163) |
| Input / Output tab and the failure text in the step strip | the journal and graph retain digest/code references; raw values need a separately approved disclosure policy | explicit Admin/Operator inspection reads input or the selected immutable journal record through `payload-inspections`, showing only structure or verified external metadata; the strip still shows the typed failure code (ADR-0028, T007-048) |
| Event history with names such as `ActivityTaskScheduled` | those are another engine's event types | the platform's own journal events, in the Studio's *Timeline* tab, restyled |
| Timeline: a bar per step over elapsed time | the Studio has no such view (its *Timeline* is the journal); the bars would be a new view over the graph's step times | not built by this restyle (G168) |
| Re-run | `422` (G145) | disabled, with the reason |
| Send signal | the API has it (`signalRun`); **the Studio has no such action today**, and a signal takes a name and a payload and has consequences (feature 003) | not built by this restyle; an open question (G167) |
| Runs as a list beside a detail pane | today a run is a page (`/runs/:id`) opened from a table | **composition, not data** (D158): side by side on a wide viewport, the URL of a run unchanged, the two pages it is today below the reflow width |
| Tweaks `graphLayout`, `liveUpdates`, `defaultTab` | not settings of the Studio (G165) | none is built as a setting |
| Scripts | **not in the prototype's navigation**, but a page of the Studio | keeps its item; its place is an open question (G163) |

## Open items for the owner

1. Design the screens the prototype does not draw: the **light theme**, the workflow editor (canvas, palette, inspector, validation,
   publish and diff), the Scripts pages, the sign-in, focus and disabled states, the empty, loading and error states (G160, G161).
2. Choose the logo (the project has three options and a provisional "S" mark; G162).
3. Say where Scripts sits in the navigation, and whether *Send signal* and the per-step *Timeline* bars are part of the Studio (G163, G167, G168).

Settled: light and dark, following the system by default (D157, the owner, 2026-10-10). The light theme is the Studio's derivation, for the owner to review.

## How this is checked

- the tokens above go into `frontend/apps/studio/src/styles/base.css` (dark, light, forced colours), and a Vitest test computes the contrast
  of every text, control-border and focus pair against the thresholds, and fails on a literal colour in any other stylesheet (T007-039);
- axe-core runs in the journeys in light, dark and forced colours on every page (T007-044);
- the Runs screen is compared with `runs-prototype.jpg` at 1470 × 812 by a person (T007-044): **a review, not a test**, and the report says so.

## First token increment — 2026-10-11

This historical first increment adopted the recorded page/field/raised surfaces, text,
accent and semantic status colours. T007-039 was then **in progress**: the complete
spacing/type/radius scale, forced-colour coverage and all component pairs are not
claimed by this increment. Fonts and layout are separate tasks.

Additional contrast deviations: shared dark control border is `#686b70` instead
of `#616368` to maintain 3:1 on the raised surface as well as the page; idle/skip
text uses `#83868c` to maintain 4.5:1 on that surface. The light derivation uses
`#12643f` for the accent/focus, `#78818d` for controls and `#626975` for skipped
status. The previous light border (2.81:1) and skipped text (4.43:1) failed against
the subtle surface. Colour literals for the minimap mask and backdrop now live
in the shared tokens.

`src/styles/tokens.test.ts` computes WCAG sRGB contrast for shared text/status,
control/focus, primary action and banner pairs, prevents literal hex/function
colours in sibling stylesheets, and checks system/explicit dark parity. It is
not a claim that forced system colours have fixed RGB ratios or that every page
has passed axe. The full visual review and authenticated journeys remain open.

Validation: Studio typecheck, ESLint, Vitest and production build passed. The
initial full run had 957 tests (94 new); the added dark-parity assertion is checked
in the focused run. Chrome rendered the dark sign-in screen at localhost:5177;
no authenticated API/Studio journey or human prototype approval was performed.

## Implemented restyle — 2026-10-11

The shared scale now includes the recorded type sizes, weights, spacing, radii and layout dimensions. Dark follows the export;
light is the Studio's derivation; forced colours map to system roles. `tokens.test.ts` checks light/dark text, control and focus
contrasts, system/explicit dark parity, forced-colour mappings, local font imports and licences, and that CSS custom-property
references resolve. A system colour is not assigned an invented fixed RGB contrast ratio.

Geist and Geist Mono are bundled from pinned `@fontsource` packages `5.3.0`, published 2026-07-19, with their SIL OFL 1.1 licence
served at `licenses/geist-OFL.txt`. There is no remote font request. System stacks remain; the browser suite includes a check
that blocks font files, verifies Geist did not load, and exercises reflow with the fallback; it passed in the final Chromium run.

The shell uses the 216px sidebar, namespace selector, real navigation destinations and a top bar with current location/session.
Runs uses a 392px master list beside `/runs/:runId` at 74rem and above. Below that threshold it keeps the separate pages and does
not mount or request the hidden master list. The master list retains filters, loaded rows and scroll across sibling-run
navigation; run-specific stores, selections and dialogs reset. Run facts and the selected-step strip use the API's values.
Terminal rows without a finish time show `—`. The list displays the last eight characters of the run ID to distinguish nearby
UUIDv7 runs; the full ID remains available in the detail and the tooltip.

Full graph cards are 184 × 62px, edges are curved, and the canvas is dotted. Taken, pending and omitted paths stay solid, dashed
and dotted respectively, including in forced colours; words and the Table view carry the same states. Review fixed missing
graph-token definitions, insufficient contrast of omitted paths, the selected-step mobile cascade and master-list remounting.
The workflow editor, Scripts and sign-in consume the same tokens. The owner approved their layouts and the light theme on 2026-10-11 (T007-043); the plain `S` monogram is provisional, not a final logo selection (G162).

Final local validation records **1 033 passing Vitest tests in 64 files**, typecheck, lint, build, generated-API check and the
dependency advisory gate. The expanded browser suite finished with **18 passed, zero failed and two skipped opt-in benchmarks**;
light/dark/forced-colour checks, 320px reflow and system-font fallback passed. Both benchmarks passed separately in three cold
rounds per scenario, with all six existing floors met and the report regenerated. The earlier dark Monaco contrast failure was
corrected before this final run. US02 was explicitly excluded because Docker's sandbox runtime was unavailable; no sandbox
execution is claimed for that local run; the subsequent Studio CI job passed all 19 browser journeys including US02. See [the validation record](../quickstart.md#visual-direction-validation--2026-10-11).
The owner explicitly approved the visual proposal after receiving the live Studio, reference and review captures.

## Owner review captures — 2026-10-11

These ten PNGs were copied from the passing browser suite, using a 1470 × 812 viewport (full-page captures may be taller).
They contain test fixtures and capture the validated wide-header and canvas-background adjustments. They are review material,
not evidence of owner approval or a visual match; compare Runs with
[`runs-prototype.jpg`](./runs-prototype.jpg) and review the derived light/editor/Script screens separately.

| Screen | Light | Dark | Forced colours |
|---|---|---|---|
| Runs | [Capture](./review/runs-light-1470x812.png) | [Capture](./review/runs-dark-1470x812.png) | [Capture](./review/runs-forced-1470x812.png) |
| Workflow editor | [Capture](./review/editor-light-1470x812.png) | [Capture](./review/editor-dark-1470x812.png) | [Capture](./review/editor-forced-1470x812.png) |
| Scripts, plain-text source | [Capture](./review/scripts-light-1470x812.png) | [Capture](./review/scripts-dark-1470x812.png) | [Capture](./review/scripts-forced-1470x812.png) |

[Runs with system fonts, Geist blocked](./review/runs-system-fonts-1470x812.png).

Owner review recorded on 2026-10-11: Marcelo explicitly approved the visual proposal ("Aprovo a proposta visual")
after opening the local Studio. This approval covers Runs, the light theme, editor, Scripts and sign-in;
automated CI evidence for this branch is recorded separately.
