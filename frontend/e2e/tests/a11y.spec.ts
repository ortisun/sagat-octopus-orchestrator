import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { env } from '../support/env';
import { Api, paymentSaga } from '../support/platform';
import { failOnBrowserErrors, goTo, signIn } from '../support/studio';
import { ActivityFailure, startWorker } from '../support/worker';

test.use({ viewport: { width: 1470, height: 812 } });

/** WCAG 2.2 AA is the target (FR-007-008): the automated rules for A and AA of 2.0, 2.1 and 2.2. */
async function expectAccessible(page: Page, where: string): Promise<void> {
  const result = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
  const found = result.violations.map((v) => `${v.id} (${v.impact}): ${v.nodes.slice(0, 3).map((n) => n.target.join(' ')).join(' | ')}`);
  expect(found, `${where}: accessibility violations`).toEqual([]);
}

/** Reflow applies to the page; a diagram may still pan within its own viewport. */
async function expectReflow(page: Page, where: string): Promise<void> {
  await page.setViewportSize({ width: 320, height: 812 });
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth), {
    message: `${where}: the page must not require horizontal scrolling at 320px`,
  }).toBeLessThanOrEqual(321);
  await expectAccessible(page, `${where}, 320px`);
  await page.setViewportSize({ width: 1470, height: 812 });
}

for (const theme of ['light', 'dark', 'forced'] as const) {
  test.describe(`A11Y ${theme}`, () => {
    test.use({ colorScheme: theme === 'dark' ? 'dark' : 'light', forcedColors: theme === 'forced' ? 'active' : 'none' });

    test('A11Y the pages of the Studio have no automated WCAG 2.2 AA violation, with the canvas and with the table', async ({ page }) => {
      test.setTimeout(240_000);
      const problems = failOnBrowserErrors(page);
      const api = new Api('admin');
      const name = `payment-a11y-${theme}`;
      const wf = await api.publish(name, paymentSaga(name));
      const run = await api.start(wf.id, { paymentId: 'p-3', amount: 25 });
      const host = await startWorker('a11y-worker', {
        'payment.validate': () => ({ valid: true }),
        'limit.reserve': () => ({ reservation: 'r-1' }),
        'payment.submit': () => { throw ActivityFailure.permanent('PAYMENT_DECLINED', 'declined by the issuer'); },
      }, { 'limit.release': () => ({ released: true }) });
      try {
        await api.until(run, 'COMPENSATED');
      } finally {
        await host.stop();
      }

      await page.goto('./');
      await expectAccessible(page, 'the sign-in page');
      await signIn(page, env.tokens.operator);

      await page.getByRole('link', { name: 'Workflows' }).click();
      await expect(page.getByRole('heading', { name: 'Workflows', level: 1 })).toBeVisible();
      await expectAccessible(page, 'the workflows list');
      await expectReflow(page, 'the workflows list');

      await page.getByRole('link', { name }).click();
      await expect(page.getByRole('heading', { name, level: 1 })).toBeVisible();
      await expectAccessible(page, 'the workflow editor, canvas');
      await page.screenshot({ path: test.info().outputPath(`editor-${theme}-1470x812.png`), fullPage: true });
      await page.getByRole('button', { name: 'List' }).click();
      await expectAccessible(page, 'the workflow editor, list');

      await page.getByRole('link', { name: 'Runs' }).click();
      await expect(page.getByRole('heading', { name: 'Runs', level: 1 })).toBeVisible();
      await expectAccessible(page, 'the runs list');
      await expectReflow(page, 'the runs list');

      await goTo(page, `/runs/${run}`);
      await expect(page.getByRole('region', { name: 'Run detail' }).getByText('Compensated', { exact: true }).first()).toBeVisible({ timeout: 20_000 });
      await expectAccessible(page, 'the run, graph');
      if (theme !== 'forced') {
        expect(await page.locator('.rn-canvas .react-flow').evaluate((node) => getComputedStyle(node).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
      }
      // At the reference viewport the graph must show a step without scrolling past its header.
      let lastGraphGeometry = '';
      await expect.poll(async () => {
        const geometry = await page.evaluate(() => {
          const rect = (selector: string) => {
            const element = document.querySelector(selector);
            if (element === null) return null;
            const bounds = element.getBoundingClientRect();
            return { top: bounds.top, bottom: bounds.bottom, left: bounds.left, right: bounds.right, width: bounds.width, height: bounds.height };
          };
          const nodes = Array.from(document.querySelectorAll('.rn-canvas .react-flow__node'));
          const nodeRects = nodes.map((node) => {
            const bounds = node.getBoundingClientRect();
            return {
              id: node.getAttribute('data-id'),
              top: bounds.top,
              bottom: bounds.bottom,
              left: bounds.left,
              right: bounds.right,
              width: bounds.width,
              height: bounds.height,
            };
          });
          const viewport = document.querySelector('.rn-canvas .react-flow__viewport');
          return {
            complete: nodes.some((node) => {
              const bounds = node.getBoundingClientRect();
              return bounds.width > 0 && bounds.height > 0 && bounds.top >= 0 && bounds.bottom <= window.innerHeight;
            }),
            window: { innerWidth: window.innerWidth, innerHeight: window.innerHeight, scrollX: window.scrollX, scrollY: window.scrollY, devicePixelRatio: window.devicePixelRatio },
            document: { scrollWidth: document.documentElement.scrollWidth, scrollHeight: document.documentElement.scrollHeight },
            forcedColors: window.matchMedia('(forced-colors: active)').matches,
            fonts: document.fonts.status,
            run: rect('.rn-run'),
            header: rect('.rn-header'),
            body: rect('.rn-body'),
            graph: rect('.rn-graph'),
            toolbar: rect('.rn-graph > .toolbar'),
            legend: rect('.rn-legend'),
            canvas: rect('.rn-canvas'),
            flow: rect('.rn-canvas .react-flow'),
            viewport: {
              rect: rect('.rn-canvas .react-flow__viewport'),
              inlineTransform: viewport instanceof HTMLElement || viewport instanceof SVGElement ? viewport.style.transform : null,
              computedTransform: viewport === null ? null : getComputedStyle(viewport).transform,
            },
            nodes: nodeRects,
          };
        });
        if (!geometry.complete) {
          const diagnostic = JSON.stringify(geometry);
          if (diagnostic !== lastGraphGeometry) {
            console.log(`RUN_GRAPH_GEOMETRY ${diagnostic}`);
            lastGraphGeometry = diagnostic;
          }
        }
        return geometry.complete;
      }, { message: 'the initial run viewport shows at least one complete graph step' }).toBe(true);
      await page.screenshot({ path: test.info().outputPath(`runs-${theme}-1470x812.png`), fullPage: true });
      await page.getByRole('tab', { name: 'Table' }).click();
      await expectAccessible(page, 'the run, table');
      await expectReflow(page, 'the run, table');
      await page.getByRole('tab', { name: 'Timeline' }).click();
      await expect(page.getByText(/journal events are read/)).toBeVisible({ timeout: 20_000 });
      await expectAccessible(page, 'the run, timeline');

      await page.getByRole('link', { name: 'Scripts' }).click();
      await expect(page.getByRole('heading', { name: 'Scripts', level: 1 })).toBeVisible();
      await expectAccessible(page, 'the scripts list');
      await signIn(page, env.tokens.admin);
      await page.getByRole('link', { name: 'Scripts' }).click();
      await page.getByRole('button', { name: 'New script' }).click();
      await page.getByRole('dialog').getByRole('textbox', { name: 'Name' }).fill(`a11y-script-${theme}`);
      await expectAccessible(page, 'the new script dialog');
      await page.getByRole('dialog').getByRole('button', { name: 'Create script' }).click();
      await expect(page.getByRole('heading', { name: `a11y-script-${theme}`, level: 1 })).toBeVisible();
      await expect(page.getByRole('textbox', { name: new RegExp(`^Source of a11y-script-${theme}`) })).toBeAttached({ timeout: 30_000 });
      await expectAccessible(page, 'the script editor');
      // Source editing remains available when a canvas/code editor is not usable.
      await page.getByRole('region', { name: 'Source' }).getByRole('button', { name: 'Use a plain text box' }).click();
      await expectAccessible(page, 'the script editor, plain text');
      await expectReflow(page, 'the script editor, plain text');
      await page.screenshot({ path: test.info().outputPath(`scripts-${theme}-1470x812.png`), fullPage: true });
      expect(problems).toEqual([]);
    });
  });
}


test('A11Y a dialog and a menu are operated with the keyboard alone: focus goes in, stays in, and comes back', async ({ page }) => {
  test.setTimeout(240_000);
  const problems = failOnBrowserErrors(page);
  const api = new Api('admin');
  const wf = await api.publish('payment-a11y-keys', paymentSaga('payment-a11y-keys'));
  // No worker: the first step waits, so the run is live and can be paused.
  const run = await api.start(wf.id, { paymentId: 'p-4', amount: 25 });
  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);

  // The disclosure opens with Enter, shows its list, and Escape closes it with focus back on its button.
  const trigger = page.getByRole('button', { name: 'Run actions' });
  await trigger.focus();
  await page.keyboard.press('Enter');
  await expect(trigger).toHaveAttribute('aria-expanded', 'true');
  await expectAccessible(page, 'the run actions menu, open');
  await page.keyboard.press('Escape');
  await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  await expect(trigger).toBeFocused();

  // A decision dialog: opened from the keyboard, focus moves into it and cannot leave it, Escape closes it, focus returns.
  await page.keyboard.press('Enter');
  await page.getByRole('button', { name: 'Pause run' }).focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('textbox', { name: 'Reason' })).toBeFocused();
  await expectAccessible(page, 'the decision dialog');
  // The page behind a modal dialog is inert: Tab cycles inside the dialog (and may pass through the browser's own
  // controls, where the page has no focused element), but never lands on something of the page behind it.
  const behind = () => dialog.evaluate((el) => document.activeElement !== null && document.activeElement !== document.body && !el.contains(document.activeElement));
  for (let i = 0; i < 10; i += 1) {
    await page.keyboard.press('Tab');
    expect(await behind(), `Tab ${i + 1} moved focus to the page behind the dialog`).toBe(false);
  }
  for (let i = 0; i < 10; i += 1) {
    await page.keyboard.press('Shift+Tab');
    expect(await behind(), `Shift+Tab ${i + 1} moved focus to the page behind the dialog`).toBe(false);
  }
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(trigger).toBeFocused();
  expect(problems).toEqual([]);
});


test('A11Y system font fallback keeps the run readable and reflows at 320px', async ({ page }) => {
  const api = new Api('admin');
  const wf = await api.publish('payment-font-fallback', paymentSaga('payment-font-fallback'));
  const run = await api.start(wf.id, { paymentId: 'font-fallback', amount: 25 });
  let blockedFonts = 0;
  await page.route(/\.woff2?(?:\?|$)/, async (route) => {
    blockedFonts += 1;
    await route.abort();
  });
  await signIn(page, env.tokens.operator);
  await goTo(page, `/runs/${run}`);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(run);
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
  expect(blockedFonts).toBeGreaterThan(0);
  expect(await page.evaluate(() => {
    let loaded = 0;
    document.fonts.forEach((font) => { if (font.status === 'loaded') loaded += 1; });
    return loaded;
  })).toBe(0);
  await expectAccessible(page, 'the run with system fonts');
  await page.screenshot({ path: test.info().outputPath('runs-system-fonts-1470x812.png'), fullPage: true });
  await page.getByRole('tab', { name: 'Table' }).click();
  await expectReflow(page, 'the run with system fonts');
});
