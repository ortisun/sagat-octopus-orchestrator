// The conformance runner's awaitJournal reads the whole journal of a run: an event on a later page than the first is
// found, and each page is asked for with the cursor the page before it gave.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { journalHas, type JournalPageReader } from '../conformance/observe.js';

function twoPages(): { read: JournalPageReader; asked: (string | undefined)[] } {
  const asked: (string | undefined)[] = [];
  const read: JournalPageReader = async (cursor) => {
    asked.push(cursor);
    if (cursor === 'second') {
      return { items: [{ raw: { sequence: 2, eventType: 'step.started', stepId: 'held', attempt: 1 } }], nextCursor: null };
    }
    return { items: [{ raw: { sequence: 1, eventType: 'step.leased', stepId: 'held', attempt: 1 } }], nextCursor: 'second' };
  };
  return { read, asked };
}

test('awaitJournal finds an event on the second page of the journal, asked for with the cursor of the first', async () => {
  const { read, asked } = twoPages();
  assert.equal(await journalHas(read, 'step.started', 'held'), true);
  assert.deepEqual(asked, [undefined, 'second']);
});

test('awaitJournal reads every page before it says the journal has no such event', async () => {
  const { read, asked } = twoPages();
  assert.equal(await journalHas(read, 'step.completed', 'held'), false);
  assert.deepEqual(asked, [undefined, 'second']);
});
