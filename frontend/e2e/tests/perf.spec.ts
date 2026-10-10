import { execSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import { dirname } from 'node:path';
import { expect, test } from '@playwright/test';
import type { Browser, Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, IR_BASE } from '../support/platform';
import { goTo, signIn } from '../support/studio';

/**
 * The benchmark of the Studio (NFR-007-PERF, T007-034): a canvas of 1 000 nodes, a run of about 10 000 nodes and its table.
 * It is not part of the journeys: it runs when STUDIO_PERF=1 and writes what it measured to STUDIO_PERF_OUT, which
 * `scripts/perf-report.mjs` turns into tests/performance/reports/007-workflow-studio.md (the runbook has the commands).
 */
test.skip(process.env.STUDIO_PERF !== '1', 'the benchmark runs with STUDIO_PERF=1');
test.use({ viewport: { width: 1440, height: 900 } });

/** Rounds of every scenario; each round is a new browser context (a cold page), and every round must meet the floors. */
const ROUNDS = 3;

/**
 * The floors the hypotheses of the report state. The specification asks for a 1 000-node canvas that is "usable" and
 * lists of 10 000 rows that are windowed, with no number of frames: these are the numbers chosen for "usable", at a
 * smooth-enough 30 frames per second, and they were fixed before the three measured rounds (an exploratory run on the same
 * machine had shown the hardware is far above them: they are floors, not a fit to the results).
 */
const FLOOR = {
  openMs: 3000,
  medianFrameMs: 33.4,
  p95FrameMs: 50,
  rowsInDom: 100,
  cardsDrawnShare: 0.1,
  graphOpenMs: 10_000,
  tableOpenMs: 3000,
  filterMs: 2000,
  heapMb: 512,
};

const measured: Record<string, unknown> = {};

/** A chain of diamonds: a condition, two steps, and the next condition. `diamonds` of them make 3 × diamonds + 1 nodes. */
function diamonds(name: string, count: number): Record<string, unknown> {
  const nodes: Array<Record<string, unknown>> = [];
  for (let i = 0; i < count; i += 1) {
    const next = i + 1 === count ? 'end' : `c${i + 1}`;
    nodes.push({ id: `c${i}`, type: 'condition', expression: 'input.amount > 1000', ifTrue: `a${i}`, ifFalse: `b${i}` });
    nodes.push({ id: `a${i}`, type: 'activity', activity: 'bench.a', timeoutMs: 1000, next });
    nodes.push({ id: `b${i}`, type: 'activity', activity: 'bench.b', timeoutMs: 1000, next });
  }
  nodes.push({ id: 'end', type: 'end' });
  return { ...IR_BASE, metadata: { name }, entrypoint: 'c0', nodes };
}

interface Frames {
  count: number;
  seconds: number;
  fps: number;
  medianMs: number;
  p95Ms: number;
  maxMs: number;
  over50Ms: number;
}

function summarize(deltas: number[]): Frames {
  const sorted = [...deltas].sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0;
  const total = deltas.reduce((a, b) => a + b, 0);
  return {
    count: deltas.length,
    seconds: Math.round(total) / 1000,
    fps: total === 0 ? 0 : Math.round((deltas.length / total) * 10_000) / 10,
    medianMs: Math.round(at(0.5) * 10) / 10,
    p95Ms: Math.round(at(0.95) * 10) / 10,
    maxMs: Math.round(Math.max(0, ...deltas) * 10) / 10,
    over50Ms: deltas.filter((d) => d > 50).length,
  };
}

/** The time between animation frames while `action` runs: what a person sees as smoothness. */
async function frameTimes(page: Page, action: () => Promise<void>): Promise<Frames> {
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __raf: number };
    w.__frames = [];
    let last = performance.now();
    const tick = (now: number) => {
      w.__frames.push(now - last);
      last = now;
      w.__raf = requestAnimationFrame(tick);
    };
    w.__raf = requestAnimationFrame(tick);
  });
  await action();
  const deltas = await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __raf: number };
    cancelAnimationFrame(w.__raf);
    return w.__frames.slice(1);
  });
  return summarize(deltas);
}

async function heapMb(page: Page): Promise<number | undefined> {
  return page.evaluate(() => {
    const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    return memory === undefined ? undefined : Math.round(memory.usedJSHeapSize / 1_048_576);
  });
}

/**
 * Drags the background of the canvas in circles. It starts where nothing but the background is (a drag that starts on a
 * card of the editor moves the card, not the canvas); the cards of a run cannot be moved, so there any point will do.
 */
async function pan(page: Page, seconds: number): Promise<void> {
  const pane = page.locator('.react-flow__pane').first();
  const box = await pane.boundingBox();
  if (!box) throw new Error('no canvas');
  const y = box.y + box.height / 2;
  let x = box.x + 240;
  for (let candidate = box.x + 20; candidate < box.x + 400; candidate += 20) {
    const onBackground = await page.evaluate(([px, py]) => (document.elementFromPoint(px as number, py as number) as HTMLElement | null)?.classList.contains('react-flow__pane') ?? false, [candidate, y]);
    if (onBackground) {
      x = candidate;
      break;
    }
  }
  await page.mouse.move(x, y);
  await page.mouse.down();
  const end = Date.now() + seconds * 1000;
  let i = 0;
  while (Date.now() < end) {
    i += 1;
    await page.mouse.move(x + Math.sin(i / 8) * 150, y + Math.cos(i / 8) * 120, { steps: 2 });
  }
  await page.mouse.up();
}

async function zoom(page: Page, seconds: number): Promise<void> {
  const pane = page.locator('.react-flow__pane').first();
  const box = await pane.boundingBox();
  if (!box) throw new Error('no canvas');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const end = Date.now() + seconds * 1000;
  let i = 0;
  while (Date.now() < end) {
    i += 1;
    await page.keyboard.down('Control');
    await page.mouse.wheel(0, i % 20 < 10 ? -80 : 80);
    await page.keyboard.up('Control');
    await page.waitForTimeout(16);
  }
}

test.afterAll(async ({ browser }) => {
  // Not under test-results: Playwright empties that directory when a run starts.
  const out = process.env.STUDIO_PERF_OUT ?? 'perf-results/perf.json';
  mkdirSync(dirname(out), { recursive: true });
  const git = (args: string) => {
    try {
      return execSync(`git ${args}`, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    } catch {
      return 'unknown';
    }
  };
  const environment = {
    cpu: os.cpus()[0]?.model ?? 'unknown',
    cores: os.cpus().length,
    memoryGb: Math.round(os.totalmem() / 2 ** 30),
    os: `${os.type()} ${os.release()} ${os.arch()}`,
    node: process.version,
    browser: `Chromium ${browser.version()} (headless, the one Playwright installs)`,
    studio: 'the production build (vite build, minified), served by the platform',
    loadAverageAtStart: os.loadavg().map((n) => Math.round(n * 100) / 100),
    commit: git('rev-parse --short HEAD'),
    dirty: git("status --porcelain -- . ':!tests/performance/reports'").split('\n').filter((l) => l !== '').length > 0,
    rounds: ROUNDS,
    floor: FLOOR,
  };
  writeFileSync(out, `${JSON.stringify({ environment, ...measured }, null, 2)}\n`);
});

async function freshPage(browser: Browser, token: string): Promise<{ page: Page; close: () => Promise<void> }> {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await signIn(page, token);
  return { page, close: () => context.close() };
}

test('PERF a workflow of 1 000 nodes in the editor: how long it takes to open, what is drawn, and how smooth it is to pan and zoom', async ({ browser }) => {
  test.setTimeout(600_000);
  const admin = new Api('admin');
  const created = await admin.call<{ id: string }>('POST', '/workflows', { name: 'bench-1k', ir: diamonds('bench-1k', 333) }, 201);
  const rounds: Array<Record<string, unknown>> = [];

  for (let round = 0; round < ROUNDS; round += 1) {
    const { page, close } = await freshPage(browser, env.tokens.developer);
    const started = Date.now();
    await goTo(page, `/workflows/${created.body.id}`);
    await expect(page.getByRole('heading', { name: 'bench-1k', level: 1 })).toBeVisible({ timeout: 60_000 });
    await page.locator('.react-flow__node').first().waitFor({ timeout: 60_000 });
    const openMs = Date.now() - started;
    const drawn = await page.locator('.react-flow__node').count();
    const validated = Date.now();
    await expect(page.getByTestId('validation-summary')).toHaveText(/problems/, { timeout: 60_000 });
    const validationMs = Date.now() - validated;

    // The frames count for something only if the canvas moved: the viewport's transform is read before and after.
    const viewport = () => page.locator('.react-flow__viewport').first().evaluate((el) => (el as HTMLElement).style.transform);
    const before = await viewport();
    const panning = await frameTimes(page, () => pan(page, 4));
    const panned = await viewport();
    expect(panned).not.toBe(before);
    const zooming = await frameTimes(page, () => zoom(page, 4));
    expect(await viewport()).not.toBe(panned);

    // Keyboard: focus a card, open it, and see the inspector show its fields.
    await page.locator('.react-flow__node').first().focus();
    const select = Date.now();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('complementary', { name: 'Inspector' }).getByRole('heading').first()).not.toHaveText('Workflow settings', { timeout: 10_000 });
    const selectMs = Date.now() - select;

    const heap = await heapMb(page);
    rounds.push({ openMs, validationMs, selectMs, drawn, panning, zooming, heapMb: heap });
    await close();
  }
  measured['canvas1k'] = { nodes: 1000, rounds };
  console.log('canvas1k', JSON.stringify(measured['canvas1k']));

  for (const r of rounds as Array<{ openMs: number; panning: Frames; zooming: Frames; heapMb?: number }>) {
    expect(r.openMs).toBeLessThanOrEqual(FLOOR.openMs);
    for (const frames of [r.panning, r.zooming]) {
      expect(frames.medianMs).toBeLessThanOrEqual(FLOOR.medianFrameMs);
      expect(frames.p95Ms).toBeLessThanOrEqual(FLOOR.p95FrameMs);
    }
    expect(r.heapMb ?? 0).toBeLessThanOrEqual(FLOOR.heapMb);
  }
});

test('PERF a run of about 10 000 nodes: the graph of simplified cards, the table of rows that are windowed, and how smooth they are', async ({ browser }) => {
  test.setTimeout(900_000);
  const admin = new Api('admin');
  const nodes = 3330 * 3 + 1;
  const wf = await admin.publish('bench-10k', diamonds('bench-10k', 3330));
  const run = await admin.start(wf.id, { amount: 5 });
  const rounds: Array<Record<string, unknown>> = [];

  for (let round = 0; round < ROUNDS; round += 1) {
    const { page, close } = await freshPage(browser, env.tokens.operator);
    const started = Date.now();
    await goTo(page, `/runs/${run}`);
    await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
    await page.locator('.react-flow__node').first().waitFor({ timeout: 120_000 });
    const graphOpenMs = Date.now() - started;
    const drawn = await page.locator('.react-flow__node').count();
    const viewport = () => page.locator('.react-flow__viewport').first().evaluate((el) => (el as HTMLElement).style.transform);
    const before = await viewport();
    const panning = await frameTimes(page, () => pan(page, 4));
    expect(await viewport()).not.toBe(before);
    const graphHeap = await heapMb(page);

    const table = Date.now();
    await page.getByRole('tab', { name: 'Table' }).click();
    await page.getByRole('table').locator('tbody tr[data-index]').first().waitFor({ timeout: 120_000 });
    const tableOpenMs = Date.now() - table;
    await expect(page.getByText(`${nodes} steps`)).toBeVisible();
    const rowsInDom = [await page.getByRole('table').locator('tbody tr[data-index]').count()];

    // Scroll the whole of it, from the first row to the last, one screenful per frame, counting the rows in the page as it goes.
    const scrolling = await frameTimes(page, () =>
      page.evaluate(
        () =>
          new Promise<void>((done) => {
            const box = document.querySelector('.rn-scroll') as HTMLElement;
            const step = box.clientHeight;
            const tick = () => {
              (window as unknown as { __rows: number[] }).__rows = [...((window as unknown as { __rows?: number[] }).__rows ?? []), document.querySelectorAll('.rn-steps tbody tr[data-index]').length];
              if (box.scrollTop + box.clientHeight >= box.scrollHeight - 2) {
                done();
                return;
              }
              box.scrollTop += step;
              requestAnimationFrame(tick);
            };
            requestAnimationFrame(tick);
          }),
      ),
    );
    rowsInDom.push(...(await page.evaluate(() => (window as unknown as { __rows: number[] }).__rows)));
    const lastRow = Number(await page.getByRole('table').locator('tbody tr[data-index]').last().getAttribute('data-index'));
    const tableHeap = await heapMb(page);

    // Find one step among all of them: the text filter over 10 000 rows.
    const find = Date.now();
    await page.getByRole('searchbox', { name: 'Find a step' }).fill('b3329');
    await expect(page.getByText('1 of')).toBeVisible({ timeout: 30_000 });
    const filterMs = Date.now() - find;

    rounds.push({ graphOpenMs, drawn, panning, graphHeap, tableOpenMs, maxRowsInDom: Math.max(...rowsInDom), lastRowIndex: lastRow, scrolling, tableHeap, filterMs });
    await close();
  }
  measured['run10k'] = { nodes, rounds };
  console.log('run10k', JSON.stringify(measured['run10k']));

  for (const r of rounds as Array<{ graphOpenMs: number; drawn: number; panning: Frames; graphHeap?: number; tableOpenMs: number; maxRowsInDom: number; lastRowIndex: number; scrolling: Frames; tableHeap?: number; filterMs: number }>) {
    expect(r.graphOpenMs).toBeLessThanOrEqual(FLOOR.graphOpenMs);
    expect(r.drawn).toBeLessThanOrEqual(nodes * FLOOR.cardsDrawnShare);
    expect(r.panning.medianMs).toBeLessThanOrEqual(FLOOR.medianFrameMs);
    expect(r.panning.p95Ms).toBeLessThanOrEqual(FLOOR.p95FrameMs);
    expect(r.tableOpenMs).toBeLessThanOrEqual(FLOOR.tableOpenMs);
    expect(r.maxRowsInDom).toBeLessThanOrEqual(FLOOR.rowsInDom);
    expect(r.lastRowIndex).toBe(nodes - 1);
    expect(r.scrolling.medianMs).toBeLessThanOrEqual(FLOOR.medianFrameMs);
    expect(r.scrolling.p95Ms).toBeLessThanOrEqual(FLOOR.p95FrameMs);
    expect(r.filterMs).toBeLessThanOrEqual(FLOOR.filterMs);
    expect(Math.max(r.graphHeap ?? 0, r.tableHeap ?? 0)).toBeLessThanOrEqual(FLOOR.heapMb);
  }
});
