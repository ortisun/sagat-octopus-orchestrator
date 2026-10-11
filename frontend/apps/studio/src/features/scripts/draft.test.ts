import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../api/errors';
import type { Script, UpdateScriptRequest } from './api';
import { DraftSaver } from './draft';
import { LIMITS } from './languages';
import { aScript } from './testing';

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const conflict = (currentRevision: number) => new ApiError({ code: 'CONFLICT', status: 409, message: 'changed', details: { reason: 'REVISION_CONFLICT', currentRevision } });
const refused = () => new ApiError({ code: 'VALIDATION_FAILED', status: 422, message: 'a schema is a JSON Schema object', details: { reason: 'SCHEMA' } });
const down = () => new ApiError({ code: 'TRANSPORT', status: 0, message: 'offline', retryable: true });

interface Sent {
  body: UpdateScriptRequest;
  key: string;
}

/** A saver over an in-memory platform: each update is recorded and answered by `answer` (by default, accepted). */
function harness(answer?: (sent: Sent, n: number) => Promise<Script> | Script, script: Script = aScript()) {
  const sent: Sent[] = [];
  let revision = script.revision;
  const update = vi.fn(async (body: UpdateScriptRequest, key: string): Promise<Script> => {
    sent.push({ body, key });
    if (answer) return answer({ body, key }, sent.length);
    revision += 1;
    return { ...script, ...(typeof body.source === 'string' ? { source: body.source } : {}), revision, updatedAt: new Date(Date.parse(script.updatedAt) + revision * 1000).toISOString() };
  });
  let n = 0;
  const saver = new DraftSaver({ script, update, debounceMs: 1000, retryDelaysMs: [2000, 5000], newKey: () => `key-${++n}` });
  const unsubscribe = saver.subscribe(() => undefined);
  return { saver, sent, update, unsubscribe, state: () => saver.getSnapshot().state.kind };
}

describe('DraftSaver: when it saves', () => {
  it('waits for a pause after the last keystroke, then sends one save with only what changed and the revision it read', async () => {
    const { saver, sent, state } = harness();
    saver.set('source', 'a');
    saver.set('source', 'ab');
    saver.set('source', 'abc');
    expect(state()).toBe('dirty');
    await vi.advanceTimersByTimeAsync(999);
    expect(sent).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.body).toEqual({ revision: 1, source: 'abc' });
    expect(state()).toBe('saved');
    expect(saver.getSnapshot().revision).toBe(2);
    expect(saver.getSnapshot().unsaved).toBe(false);
  });

  it('does nothing for text that is the same as the platform has', async () => {
    const { saver, sent, state } = harness();
    saver.set('source', aScript().source);
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toHaveLength(0);
    expect(state()).toBe('saved');
  });

  it('bases the next save on the revision the last one made', async () => {
    const { saver, sent } = harness();
    saver.set('source', 'one');
    await vi.advanceTimersByTimeAsync(1000);
    saver.set('source', 'two');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent.map((s) => s.body.revision)).toEqual([1, 2]);
    expect(sent.map((s) => s.body.source)).toEqual(['one', 'two']);
  });

  it('keeps one save in flight; what is typed meanwhile goes in the next, on the new revision', async () => {
    let finish: (script: Script) => void = () => undefined;
    const { saver, sent, state } = harness((_, n) => (n === 1 ? new Promise<Script>((resolve) => (finish = resolve)) : { ...aScript(), revision: 3 }));
    saver.set('source', 'first');
    await vi.advanceTimersByTimeAsync(1000);
    expect(state()).toBe('saving');
    saver.set('source', 'second');
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toHaveLength(1);
    finish({ ...aScript(), revision: 2 });
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toHaveLength(2);
    expect(sent[1]?.body).toEqual({ revision: 2, source: 'second' });
  });

  it('sends an emptied schema as null, which removes it, and a good one as the object', async () => {
    const { saver, sent } = harness(undefined, aScript({ inputSchema: { type: 'object' } as never }));
    saver.set('inputSchema', '');
    saver.set('outputSchema', '{"type":"object"}');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent[0]?.body).toEqual({ revision: 1, inputSchema: null, outputSchema: { type: 'object' } });
  });
});

describe('DraftSaver: what it will not send', () => {
  it('keeps back a schema that is not JSON, says so, and still saves the rest', async () => {
    const { saver, sent } = harness();
    saver.set('source', 'new source');
    saver.set('inputSchema', '{"type": ');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent[0]?.body).toEqual({ revision: 1, source: 'new source' });
    const snapshot = saver.getSnapshot();
    expect(snapshot.unsaved).toBe(true);
    expect(snapshot.problems).toHaveLength(1);
    expect(snapshot.problems[0]).toMatchObject({ field: 'inputSchema' });
    expect(snapshot.problems[0]?.message).toMatch(/Not valid JSON/);
    // The author's text is still there.
    expect(snapshot.fields.inputSchema).toBe('{"type": ');
  });

  it('sends nothing at all when the only change is one it cannot send', async () => {
    const { saver, sent } = harness();
    saver.set('outputSchema', '[1,2]');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(sent).toHaveLength(0);
    expect(saver.getSnapshot().problems[0]?.message).toMatch(/JSON object/);
  });

  it('does not send a source over the limit, an empty one or one with a NUL character', async () => {
    const { saver, sent } = harness();
    saver.set('source', 'x'.repeat(LIMITS.sourceBytes + 1));
    await vi.advanceTimersByTimeAsync(2000);
    expect(saver.getSnapshot().problems[0]?.message).toMatch(/limit is 256 KiB/);
    saver.set('source', '   ');
    expect(saver.getSnapshot().problems[0]?.message).toMatch(/empty/);
    saver.set('source', `ok${String.fromCharCode(0)}`);
    expect(saver.getSnapshot().problems[0]?.message).toMatch(/NUL/);
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toHaveLength(0);
  });

  it('counts the limit in bytes, as the platform does, not in characters', async () => {
    const { saver, sent } = harness();
    // 100 000 characters of 3 bytes each is 300 000 bytes: over the limit though under it in characters.
    saver.set('source', '你'.repeat(100_000));
    await vi.advanceTimersByTimeAsync(2000);
    expect(sent).toHaveLength(0);
    expect(saver.getSnapshot().problems).toHaveLength(1);
  });
});

describe('DraftSaver: two people editing', () => {
  it('stops at a conflict, keeps the author\'s text and sends nothing more until they decide', async () => {
    const { saver, sent, state } = harness(() => {
      throw conflict(5);
    });
    saver.set('source', 'mine');
    await vi.advanceTimersByTimeAsync(1000);
    expect(state()).toBe('conflict');
    expect(saver.getSnapshot().state).toMatchObject({ kind: 'conflict', currentRevision: 5 });
    saver.set('source', 'mine, edited further');
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sent).toHaveLength(1);
    expect(saver.getSnapshot().fields.source).toBe('mine, edited further');
    expect(await saver.save()).toBe(false);
    expect(sent).toHaveLength(1);
  });

  it('can take the platform\'s copy, which drops the author\'s changes', async () => {
    const { saver, state } = harness(() => {
      throw conflict(5);
    });
    saver.set('source', 'mine');
    await vi.advanceTimersByTimeAsync(1000);
    saver.adoptTheirs(aScript({ source: 'theirs', revision: 5, updatedAt: '2026-10-10T12:00:00Z' }));
    expect(saver.getSnapshot()).toMatchObject({ revision: 5, unsaved: false, fields: { source: 'theirs' } });
    expect(state()).toBe('saved');
  });

  it('can keep the author\'s changes over the platform\'s copy: what they changed stays, the rest is the platform\'s, and the save is based on the new revision', async () => {
    let calls = 0;
    const { saver, sent, state } = harness((sent) => {
      calls += 1;
      if (calls === 1) throw conflict(5);
      return { ...aScript(), revision: 6, source: String(sent.body.source) };
    });
    saver.set('source', 'my source');
    await vi.advanceTimersByTimeAsync(1000);
    expect(state()).toBe('conflict');
    // Meanwhile the other person changed the output schema, which the author never touched.
    saver.keepMine(aScript({ source: 'their source', outputSchema: { type: 'string' } as never, revision: 5 }));
    expect(saver.getSnapshot().fields.source).toBe('my source');
    expect(saver.getSnapshot().fields.outputSchema).toContain('"string"');
    expect(await saver.save()).toBe(true);
    expect(sent[1]?.body).toEqual({ revision: 5, source: 'my source' });
    expect(saver.getSnapshot().revision).toBe(6);
  });

  it('can be told by the page that the platform\'s draft has moved on', () => {
    const { saver, state } = harness();
    saver.reportConflict(9);
    expect(state()).toBe('conflict');
    expect(saver.getSnapshot().state).toMatchObject({ currentRevision: 9 });
  });
});

describe('DraftSaver: failures', () => {
  it('tries again by itself with the same key (a save whose answer was lost is not applied twice), then gives up and says so', async () => {
    const { saver, sent, state } = harness(() => {
      throw down();
    });
    saver.set('source', 'x');
    await vi.advanceTimersByTimeAsync(1000);
    expect(state()).toBe('error');
    expect(saver.getSnapshot().state).toMatchObject({ retryInMs: 2000 });
    await vi.advanceTimersByTimeAsync(2000);
    expect(sent).toHaveLength(2);
    expect(saver.getSnapshot().state).toMatchObject({ retryInMs: 5000 });
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toHaveLength(3);
    expect(saver.getSnapshot().state).toMatchObject({ kind: 'error', retryInMs: undefined });
    await vi.advanceTimersByTimeAsync(120_000);
    expect(sent).toHaveLength(3);
    expect(new Set(sent.map((s) => s.key)).size).toBe(1);
    expect(sent.every((s) => s.body.revision === 1 && s.body.source === 'x')).toBe(true);
  });

  it('recovers when a retry works', async () => {
    const { saver, sent, state } = harness((_sent, n) => {
      if (n === 1) throw down();
      return { ...aScript(), revision: 2 };
    });
    saver.set('source', 'x');
    await vi.advanceTimersByTimeAsync(3000);
    expect(sent).toHaveLength(2);
    expect(state()).toBe('saved');
    expect(saver.getSnapshot().revision).toBe(2);
  });

  it('uses a new key when what is sent has changed', async () => {
    const { saver, sent } = harness((_sent, n) => {
      if (n === 1) throw down();
      return { ...aScript(), revision: 2 };
    });
    saver.set('source', 'x');
    await vi.advanceTimersByTimeAsync(1000);
    saver.set('source', 'xy');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toHaveLength(2);
    expect(sent[0]?.key).not.toBe(sent[1]?.key);
  });

  it('does not retry a refusal: the same text would be refused again; a new edit tries again', async () => {
    let refuse = true;
    const { saver, sent, state } = harness(() => {
      if (refuse) throw refused();
      return { ...aScript(), revision: 2 };
    });
    saver.set('outputSchema', '{"type":"bogus"}');
    await vi.advanceTimersByTimeAsync(30_000);
    expect(state()).toBe('rejected');
    expect(sent).toHaveLength(1);
    refuse = false;
    saver.set('outputSchema', '{"type":"string"}');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toHaveLength(2);
    expect(state()).toBe('saved');
  });

  it('does not retry what is not worth it (forbidden)', async () => {
    const { saver, sent, state } = harness(() => {
      throw new ApiError({ code: 'FORBIDDEN', status: 403, message: 'no' });
    });
    saver.set('source', 'x');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sent).toHaveLength(1);
    expect(state()).toBe('error');
  });
});

describe('DraftSaver.save', () => {
  it('sends what is pending now and says the platform has it all', async () => {
    const { saver, sent } = harness();
    saver.set('source', 'x');
    expect(await saver.save()).toBe(true);
    expect(sent).toHaveLength(1);
    // The pause that was running is not left behind to send it again.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(sent).toHaveLength(1);
  });

  it('waits for the save in flight and then sends what was typed after it', async () => {
    let finish: (script: Script) => void = () => undefined;
    const { saver, sent } = harness((_, n) => (n === 1 ? new Promise<Script>((resolve) => (finish = resolve)) : { ...aScript(), revision: 3 }));
    saver.set('source', 'a');
    await vi.advanceTimersByTimeAsync(1000);
    saver.set('source', 'ab');
    const done = saver.save();
    finish({ ...aScript(), revision: 2 });
    expect(await done).toBe(true);
    expect(sent.map((s) => s.body.source)).toEqual(['a', 'ab']);
  });

  it('says no, after one try, when the save fails; it does not hammer the platform', async () => {
    const { saver, sent } = harness(() => {
      throw down();
    });
    saver.set('source', 'x');
    expect(await saver.save()).toBe(false);
    expect(sent).toHaveLength(1);
  });

  it('says no while a part of the draft cannot be sent', async () => {
    const { saver } = harness();
    saver.set('inputSchema', 'nope');
    expect(await saver.save()).toBe(false);
  });

  it('says yes when there is nothing to save', async () => {
    const { saver, sent } = harness();
    expect(await saver.save()).toBe(true);
    expect(sent).toHaveLength(0);
  });
});

describe('DraftSaver: leaving the page', () => {
  it('sends the edits that were waiting for the pause when the last listener goes', async () => {
    const { saver, sent, unsubscribe } = harness();
    saver.set('source', 'unsaved work');
    unsubscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(sent).toHaveLength(1);
    expect(sent[0]?.body.source).toBe('unsaved work');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('leaves no timer behind', async () => {
    const { saver, sent, unsubscribe } = harness();
    saver.set('source', 'x');
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toHaveLength(1);
    unsubscribe();
    expect(vi.getTimerCount()).toBe(0);
  });
});
