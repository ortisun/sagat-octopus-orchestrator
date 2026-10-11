# Benchmark: the Studio in a real browser (feature 007)

Produced by `frontend/e2e/tests/perf.spec.ts` (T007-034, NFR-007-PERF) and rendered by `frontend/e2e/scripts/perf-report.mjs`. The hypotheses are written down as floors before the measured rounds; each says whether it held.

## Hypotheses

- **H1** A workflow of 1 000 nodes opens in the editor, with its first card drawn, within 3 s of the navigation, in a cold page.
- **H2** Panning and zooming that canvas keeps the median frame at or under 33.4 ms (30 frames per second) and the 95th percentile at or under 50 ms.
- **H3** A run of about 10 000 nodes draws its first card within 10 s; no more than 10 % of its cards are in the page at a time (the canvas draws what is in view); panning it keeps the frame figures of H2.
- **H4** The table of those 10 000 rows opens within 3 s of the click, never holds more than 100 rows in the page at any scroll position, reaches the last row, and scrolling through all of it keeps the frame figures of H2.
- **H5** Typing a step's id into the table's filter shows the one match among the 10 000 within 2 s.
- **H6** The JavaScript heap of the page stays at or under 512 MB in all of that.

The floors were fixed before the three measured rounds, after one exploratory run on the same machine that had already shown the hardware was far above them. They are therefore floors that say "usable", not a fit to the results; a machine that is a tenth as fast would still be the machine this report cannot speak for (see the limits).

## Environment

- CPU: Apple M3 Max (16 cores), 48 GB
- OS: Darwin 25.3.0 arm64; Node v26.8.1
- Browser: Chromium 153.0.8010.12 (headless, the one Playwright installs)
- Studio: the production build (vite build, minified), served by the platform
- Load average when the run began: 5.24 5.09 5.39
- Commit: 969a748 (with uncommitted changes at the time of the run)
- Rounds per scenario: 3; each round is a new browser context (a cold page); every cell is the median round, with the lowest and highest in brackets
- The platform, the engine and the browser on the same machine, over loopback; PostgreSQL in a container

## The editor with a workflow of 1 000 nodes (a chain of 333 diamonds: a condition, two steps, the next condition)

| Measure | Result |
|---|---|
| Time until the first card is drawn | 702 ms (698–715) |
| Cards in the page (the editor draws all of them up to 1 000) | 1000 (1000–1000) |
| Time for the platform's check of the draft to answer | 38 ms (38–41) |
| Time from Enter on a card to its fields in the inspector | 144 ms (142–159) |
| JavaScript heap | 82 MB (65–87) |

| Interaction (about 4 s each) | Frames per second | Median frame | 95th percentile | Slowest frame | Frames over 50 ms |
|---|---|---|---|---|---|
| Pan (drag the background) | 59.2 (59.1–59.4) | 16.7 ms (16.7–16.7) | 16.8 ms (16.7–16.8) | 33.4 ms (33.3–33.4) | 0 (0–0) |
| Zoom (Ctrl and wheel) | 60.0 (59.8–60.0) | 16.7 ms (16.7–16.7) | 16.7 ms (16.7–16.7) | 16.8 ms (16.8–33.4) | 0 (0–0) |

## A run of 9991 nodes (a chain of 3 330 diamonds), none of them started

| Measure | Result |
|---|---|
| Time until the first card of the graph is drawn | 1198 ms (1183–1317) |
| Cards in the page of 9991 (simplified cards, only those in view) | 377 (377–377) |
| Time from the click on the Table tab to its first row | 72 ms (60–72) |
| Most rows in the page at any point of a scroll through all of them | 29 (29–29) |
| Index of the last row reached | 9990, 9990, 9990 |
| Time to find one step by typing its id | 8 ms (8–8) |
| JavaScript heap with the graph open | 273 MB (273–290) |
| JavaScript heap after the table was scrolled through | 273 MB (273–290) |

| Interaction | Frames per second | Median frame | 95th percentile | Slowest frame | Frames over 50 ms |
|---|---|---|---|---|---|
| Pan the graph (about 4 s) | 60.0 (60.0–60.0) | 16.7 ms (16.7–16.7) | 16.7 ms (16.7–16.7) | 16.8 ms (16.8–16.8) | 0 (0–0) |
| Scroll the table from the first row to the last (one screenful per frame) | 60.0 (60.0–60.0) | 16.7 ms (16.7–16.7) | 16.7 ms (16.7–16.8) | 16.8 ms (16.8–16.8) | 0 (0–0) |

## Results against the hypotheses

| | Hypothesis | Result |
|---|---|---|
| H1 | the editor opens 1 000 nodes in ≤ 3 s | held (slowest round 715 ms) |
| H2 | pan and zoom of 1 000 nodes: median ≤ 33.4 ms, p95 ≤ 50 ms | held |
| H3 | a run of 9991 nodes: first card ≤ 10 s, ≤ 10 % of the cards drawn, smooth pan | held (slowest round 1317 ms; most cards drawn 377) |
| H4 | the table: opens in ≤ 3 s, ≤ 100 rows in the page, last row reached, smooth scroll | held (slowest 72 ms; most rows 29) |
| H5 | the filter finds one step in ≤ 2 s | held (slowest 8 ms) |
| H6 | heap ≤ 512 MB | held (highest 290 MB) |

## What this report does not say

- **One machine, one browser.** A fast laptop (see the environment) with headless Chromium. Nothing here is a statement about a low-end machine, about Firefox or Safari, or about a browser with a graphics driver that composites slowly. The reference browser the specification names is not defined in it (a gap, G151): this report names the one it measured.
- **Frames are measured with `requestAnimationFrame`,** which on a 60 Hz display tops out at 60 per second: a figure of 60 says "no frame was missed", not "it could do more". The scripted drag and wheel are not a person: a person pauses.
- **The shape of the graphs is one shape:** a chain of diamonds, two wide. A wide graph (hundreds of branches side by side) or one with many edges per node draws differently; it was not measured.
- **Dragging a card** in the editor and **editing while 1 000 nodes are drawn** were not measured beyond the keyboard selection above.
- **The run is not moving:** its nodes are not started, so no live updates arrived while it was measured. The size of a live update is bounded by the design (a change carries the steps its events name or create, and the table redraws the rows in view) and the model is unit-tested, but this benchmark did not put a time on applying updates to a large run.
- **The heap is Chromium's `performance.memory`,** which is coarse (it is rounded and updated lazily) and says nothing about the memory of the browser outside the page.
- **Loopback network.** The platform and the browser are on the same machine; the time to fetch 9991 nodes of graph over a real network is not in these figures.
