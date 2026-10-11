// Turns what tests/perf.spec.ts measured into tests/performance/reports/workflow-studio.md.
// Usage: node scripts/perf-report.mjs <perf.json> <report.md>
import { readFileSync, writeFileSync } from 'node:fs';

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error('usage: node scripts/perf-report.mjs <perf.json> <report.md>');
  process.exit(2);
}
const data = JSON.parse(readFileSync(input, 'utf8'));
const { environment: env, canvas1k, run10k } = data;
if (!canvas1k || !run10k) {
  console.error('the measurements are incomplete: run both scenarios (STUDIO_PERF=1, grep PERF)');
  process.exit(1);
}
const floor = env.floor;

const median = (xs) => {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[Math.floor((sorted.length - 1) / 2)];
};
const range = (xs, unit = '', digits = 0) => {
  const f = (n) => n.toFixed(digits);
  return `${f(median(xs))}${unit} (${f(Math.min(...xs))}–${f(Math.max(...xs))})`;
};
const of = (rounds, pick) => rounds.map(pick);
const worst = (rounds, pick) => Math.max(...rounds.map(pick));

const c = canvas1k.rounds;
const r = run10k.rounds;
const verdict = (ok) => (ok ? 'held' : '**did not hold**');

const h1 = worst(c, (x) => x.openMs) <= floor.openMs;
const h2 = [...c.flatMap((x) => [x.panning, x.zooming])].every((f) => f.medianMs <= floor.medianFrameMs && f.p95Ms <= floor.p95FrameMs);
const h3 = worst(r, (x) => x.graphOpenMs) <= floor.graphOpenMs && worst(r, (x) => x.drawn) <= run10k.nodes * floor.cardsDrawnShare && r.every((x) => x.panning.medianMs <= floor.medianFrameMs && x.panning.p95Ms <= floor.p95FrameMs);
const h4 = worst(r, (x) => x.tableOpenMs) <= floor.tableOpenMs && worst(r, (x) => x.maxRowsInDom) <= floor.rowsInDom && r.every((x) => x.lastRowIndex === run10k.nodes - 1 && x.scrolling.medianMs <= floor.medianFrameMs && x.scrolling.p95Ms <= floor.p95FrameMs);
const h5 = worst(r, (x) => x.filterMs) <= floor.filterMs;
const h6 = Math.max(worst(c, (x) => x.heapMb ?? 0), worst(r, (x) => Math.max(x.graphHeap ?? 0, x.tableHeap ?? 0))) <= floor.heapMb;

const frames = (rounds, pick) => ({
  fps: range(of(rounds, (x) => pick(x).fps), '', 1),
  median: range(of(rounds, (x) => pick(x).medianMs), ' ms', 1),
  p95: range(of(rounds, (x) => pick(x).p95Ms), ' ms', 1),
  worst: range(of(rounds, (x) => pick(x).maxMs), ' ms', 1),
  slow: range(of(rounds, (x) => pick(x).over50Ms), '', 0),
});

const row = (name, f) => `| ${name} | ${f.fps} | ${f.median} | ${f.p95} | ${f.worst} | ${f.slow} |`;

const lines = `# Benchmark: the Studio in a real browser (feature 007)

Produced by \`frontend/e2e/tests/perf.spec.ts\` (T007-034, NFR-007-PERF) and rendered by \`frontend/e2e/scripts/perf-report.mjs\`. The hypotheses are written down as floors before the measured rounds; each says whether it held.

## Hypotheses

- **H1** A workflow of 1 000 nodes opens in the editor, with its first card drawn, within ${floor.openMs / 1000} s of the navigation, in a cold page.
- **H2** Panning and zooming that canvas keeps the median frame at or under ${floor.medianFrameMs} ms (30 frames per second) and the 95th percentile at or under ${floor.p95FrameMs} ms.
- **H3** A run of about 10 000 nodes draws its first card within ${floor.graphOpenMs / 1000} s; no more than ${floor.cardsDrawnShare * 100} % of its cards are in the page at a time (the canvas draws what is in view); panning it keeps the frame figures of H2.
- **H4** The table of those 10 000 rows opens within ${floor.tableOpenMs / 1000} s of the click, never holds more than ${floor.rowsInDom} rows in the page at any scroll position, reaches the last row, and scrolling through all of it keeps the frame figures of H2.
- **H5** Typing a step's id into the table's filter shows the one match among the 10 000 within ${floor.filterMs / 1000} s.
- **H6** The JavaScript heap of the page stays at or under ${floor.heapMb} MB in all of that.

The floors were fixed before the three measured rounds, after one exploratory run on the same machine that had already shown the hardware was far above them. They are therefore floors that say "usable", not a fit to the results; a machine that is a tenth as fast would still be the machine this report cannot speak for (see the limits).

## Environment

- CPU: ${env.cpu} (${env.cores} cores), ${env.memoryGb} GB
- OS: ${env.os}; Node ${env.node}
- Browser: ${env.browser}
- Studio: ${env.studio}
- Load average when the run began: ${env.loadAverageAtStart.join(' ')}
- Commit: ${env.commit}${env.dirty ? ' (with uncommitted changes at the time of the run)' : ''}
- Rounds per scenario: ${env.rounds}; each round is a new browser context (a cold page); every cell is the median round, with the lowest and highest in brackets
- The platform, the engine and the browser on the same machine, over loopback; real PostgreSQL (a local server via SAGAT_TEST_DATABASE_URL or the harness container)

## The editor with a workflow of 1 000 nodes (a chain of 333 diamonds: a condition, two steps, the next condition)

| Measure | Result |
|---|---|
| Time until the first card is drawn | ${range(of(c, (x) => x.openMs), ' ms')} |
| Cards in the page (the editor draws all of them up to 1 000) | ${range(of(c, (x) => x.drawn))} |
| Time for the platform's check of the draft to answer | ${range(of(c, (x) => x.validationMs), ' ms')} |
| Time from Enter on a card to its fields in the inspector | ${range(of(c, (x) => x.selectMs), ' ms')} |
| JavaScript heap | ${range(of(c, (x) => x.heapMb ?? 0), ' MB')} |

| Interaction (about 4 s each) | Frames per second | Median frame | 95th percentile | Slowest frame | Frames over 50 ms |
|---|---|---|---|---|---|
${row('Pan (drag the background)', frames(c, (x) => x.panning))}
${row('Zoom (Ctrl and wheel)', frames(c, (x) => x.zooming))}

## A run of ${run10k.nodes} nodes (a chain of 3 330 diamonds), none of them started

| Measure | Result |
|---|---|
| Time until the first card of the graph is drawn | ${range(of(r, (x) => x.graphOpenMs), ' ms')} |
| Cards in the page of ${run10k.nodes} (simplified cards, only those in view) | ${range(of(r, (x) => x.drawn))} |
| Time from the click on the Table tab to its first row | ${range(of(r, (x) => x.tableOpenMs), ' ms')} |
| Most rows in the page at any point of a scroll through all of them | ${range(of(r, (x) => x.maxRowsInDom))} |
| Index of the last row reached | ${r.map((x) => x.lastRowIndex).join(', ')} |
| Time to find one step by typing its id | ${range(of(r, (x) => x.filterMs), ' ms')} |
| JavaScript heap with the graph open | ${range(of(r, (x) => x.graphHeap ?? 0), ' MB')} |
| JavaScript heap after the table was scrolled through | ${range(of(r, (x) => x.tableHeap ?? 0), ' MB')} |

| Interaction | Frames per second | Median frame | 95th percentile | Slowest frame | Frames over 50 ms |
|---|---|---|---|---|---|
${row('Pan the graph (about 4 s)', frames(r, (x) => x.panning))}
${row('Scroll the table from the first row to the last (one screenful per frame)', frames(r, (x) => x.scrolling))}

## Results against the hypotheses

| | Hypothesis | Result |
|---|---|---|
| H1 | the editor opens 1 000 nodes in ≤ ${floor.openMs / 1000} s | ${verdict(h1)} (slowest round ${worst(c, (x) => x.openMs)} ms) |
| H2 | pan and zoom of 1 000 nodes: median ≤ ${floor.medianFrameMs} ms, p95 ≤ ${floor.p95FrameMs} ms | ${verdict(h2)} |
| H3 | a run of ${run10k.nodes} nodes: first card ≤ ${floor.graphOpenMs / 1000} s, ≤ ${floor.cardsDrawnShare * 100} % of the cards drawn, smooth pan | ${verdict(h3)} (slowest round ${worst(r, (x) => x.graphOpenMs)} ms; most cards drawn ${worst(r, (x) => x.drawn)}) |
| H4 | the table: opens in ≤ ${floor.tableOpenMs / 1000} s, ≤ ${floor.rowsInDom} rows in the page, last row reached, smooth scroll | ${verdict(h4)} (slowest ${worst(r, (x) => x.tableOpenMs)} ms; most rows ${worst(r, (x) => x.maxRowsInDom)}) |
| H5 | the filter finds one step in ≤ ${floor.filterMs / 1000} s | ${verdict(h5)} (slowest ${worst(r, (x) => x.filterMs)} ms) |
| H6 | heap ≤ ${floor.heapMb} MB | ${verdict(h6)} (highest ${Math.max(worst(c, (x) => x.heapMb ?? 0), worst(r, (x) => Math.max(x.graphHeap ?? 0, x.tableHeap ?? 0)))} MB) |

## What this report does not say

- **One machine, one browser.** A fast laptop (see the environment) with headless Chromium. Nothing here is a statement about a low-end machine, about Firefox or Safari, or about a browser with a graphics driver that composites slowly. The reference browser the specification names is not defined in it (a gap, G151): this report names the one it measured.
- **Frames are measured with \`requestAnimationFrame\`,** which on a 60 Hz display tops out at 60 per second: a figure of 60 says "no frame was missed", not "it could do more". The scripted drag and wheel are not a person: a person pauses.
- **The shape of the graphs is one shape:** a chain of diamonds, two wide. A wide graph (hundreds of branches side by side) or one with many edges per node draws differently; it was not measured.
- **Dragging a card** in the editor and **editing while 1 000 nodes are drawn** were not measured beyond the keyboard selection above.
- **The run is not moving:** its nodes are not started, so no live updates arrived while it was measured. The size of a live update is bounded by the design (a change carries the steps its events name or create, and the table redraws the rows in view) and the model is unit-tested, but this benchmark did not put a time on applying updates to a large run.
- **The heap is Chromium's \`performance.memory\`,** which is coarse (it is rounded and updated lazily) and says nothing about the memory of the browser outside the page.
- **Loopback network.** The platform and the browser are on the same machine; the time to fetch ${run10k.nodes} nodes of graph over a real network is not in these figures.
`;
writeFileSync(output, lines);
console.log(`wrote ${output}`);
