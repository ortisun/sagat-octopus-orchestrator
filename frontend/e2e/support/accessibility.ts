import AxeBuilder from '@axe-core/playwright';
import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';

/**
 * WCAG 2.2 AA is the target (FR-007-008): the automated rules for A and AA of 2.0, 2.1 and 2.2. The list lives here, once, for
 * `a11y.spec.ts` and the journeys of feature 018; there is no way to exclude a node, so what a journey scans is the whole page.
 */
export const WCAG_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

/** Zero violations on the page as it is now (an open modal dialog is the page's top layer: the rest is inert and is scanned all the same). */
export async function expectAccessible(page: Page, where: string): Promise<void> {
  const result = await new AxeBuilder({ page }).withTags(WCAG_TAGS).analyze();
  const found = result.violations.map((v) => `${v.id} (${v.impact}): ${v.nodes.slice(0, 3).map((n) => n.target.join(' ')).join(' | ')}`);
  expect(found, `${where}: accessibility violations`).toEqual([]);
}
