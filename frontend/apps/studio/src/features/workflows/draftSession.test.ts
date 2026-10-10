import { describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../api/errors';
import type { Workflow } from '../../api/types';
import { DraftSession } from './draftSession';

const doc = (activity: string): Record<string, unknown> => ({ metadata: { name: 'x-flow' }, nodes: [{ id: 'a', type: 'activity', activity }] });
const workflow = (revision: number, ir: Record<string, unknown>): Workflow => ({ id: 'w', name: 'x-flow', draftRevision: revision, ir });
const conflict = () => new ApiError({ code: 'CONFLICT', status: 409, message: 'The draft changed.' });
const lost = () => new ApiError({ code: 'TRANSPORT', status: 0, message: 'The platform could not be reached.', retryable: true });
const failing = () => new ApiError({ code: 'INTERNAL', status: 500, message: 'boom', traceId: 'c'.repeat(32) });

function setup(start = doc('v1')) {
  const updateDraft = vi.fn<(id: string, revision: number, ir: Record<string, unknown>, key?: string) => Promise<Workflow>>();
  const getWorkflow = vi.fn<(id: string) => Promise<Workflow>>();
  let keys = 0;
  const session = new DraftSession({ ep: { updateDraft, getWorkflow }, id: 'w', revision: 3, wire: start, newKey: () => `key-${++keys}` });
  return { session, updateDraft, getWorkflow };
}

describe('DraftSession', () => {
  it('starts clean at the revision it was given, and is dirty once the document differs', () => {
    const { session } = setup();
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 3, dirty: false });
    session.update(doc('v2'));
    expect(session.getSnapshot().dirty).toBe(true);
    session.update(doc('v1'));
    expect(session.getSnapshot().dirty).toBe(false);
  });

  it('saves on the revision it holds, with an idempotency key, and takes the new revision from the answer', async () => {
    const { session, updateDraft } = setup();
    updateDraft.mockResolvedValueOnce(workflow(4, doc('v2')));
    session.update(doc('v2'));
    await expect(session.save()).resolves.toBe('saved');
    expect(updateDraft).toHaveBeenCalledWith('w', 3, doc('v2'), 'key-1');
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 4, dirty: false });
  });

  it('sends nothing when nothing changed', async () => {
    const { session, updateDraft } = setup();
    await expect(session.save()).resolves.toBe('saved');
    expect(updateDraft).not.toHaveBeenCalled();
  });

  it('keeps the idempotency key when the very same save is tried again, and uses a new one for another document', async () => {
    const { session, updateDraft } = setup();
    updateDraft
      .mockRejectedValueOnce(lost())
      .mockRejectedValueOnce(failing())
      .mockResolvedValueOnce(workflow(4, doc('v3')));
    session.update(doc('v2'));
    expect(await session.save()).toBe('failed');
    expect(await session.save()).toBe('failed');
    session.update(doc('v3'));
    expect(await session.save()).toBe('saved');
    const keys = updateDraft.mock.calls.map((c) => c[3]);
    expect(keys).toEqual(['key-1', 'key-1', 'key-2']);
    // Every try was on the revision the editor knew.
    expect(updateDraft.mock.calls.map((c) => c[1])).toEqual([3, 3, 3]);
  });

  it('reports a failure with its error, and does not mark the document saved', async () => {
    const { session, updateDraft } = setup();
    updateDraft.mockRejectedValueOnce(failing());
    session.update(doc('v2'));
    expect(await session.save()).toBe('failed');
    const snapshot = session.getSnapshot();
    expect(snapshot.phase).toBe('failed');
    expect(snapshot.dirty).toBe(true);
    expect((snapshot.error as ApiError).traceId).toBe('c'.repeat(32));
    expect(snapshot.failedKey).toBe(snapshot.currentKey);
  });

  it('on a 409 keeps the author copy, fetches the other, and overwrites nothing', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(4, doc('theirs')));
    session.update(doc('mine'));
    expect(await session.save()).toBe('conflict');
    const snapshot = session.getSnapshot();
    expect(snapshot.phase).toBe('conflict');
    expect(snapshot.theirs?.draftRevision).toBe(4);
    expect(snapshot.dirty).toBe(true);
    // It stops there: no further save goes out until the author decides.
    expect(await session.save()).toBe('conflict');
    expect(updateDraft).toHaveBeenCalledTimes(1);
  });

  it('is not fooled into a conflict by a draft that already says what is being saved', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(7, doc('mine')));
    session.update(doc('mine'));
    expect(await session.save()).toBe('saved');
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 7, dirty: false });
  });

  it('recognises its own earlier save whose answer was lost, and goes on from the revision it reached', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    // The first save reached the platform (revision 4) but the answer never came back.
    updateDraft.mockRejectedValueOnce(lost());
    session.update(doc('v2'));
    expect(await session.save()).toBe('failed');
    // The author types on; the next save is on the old revision and is refused...
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(4, doc('v2')));
    // ... the platform's draft is the document that was sent, so this is no conflict: save on revision 4.
    updateDraft.mockResolvedValueOnce(workflow(5, doc('v3')));
    session.update(doc('v3'));
    expect(await session.save()).toBe('saved');
    expect(updateDraft.mock.calls.map((c) => c[1])).toEqual([3, 3, 4]);
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 5, dirty: false });
  });

  it('keeps the conflict if the platform holds something it never sent', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(lost());
    session.update(doc('v2'));
    await session.save();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(4, doc('somebody else')));
    session.update(doc('v3'));
    expect(await session.save()).toBe('conflict');
  });

  it('can save over the other copy on top of their revision, with a new key', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(4, doc('theirs')));
    session.update(doc('mine'));
    await session.save();
    const theirs = session.getSnapshot().theirs as Workflow;
    updateDraft.mockResolvedValueOnce(workflow(5, doc('mine')));
    expect(await session.overwrite(theirs)).toBe('saved');
    expect(updateDraft.mock.calls[1]).toEqual(['w', 4, doc('mine'), 'key-2']);
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 5, dirty: false });
  });

  it('can take the other copy: clean, at their revision, with nothing sent', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockResolvedValueOnce(workflow(4, doc('theirs')));
    session.update(doc('mine'));
    await session.save();
    session.adopt(session.getSnapshot().theirs as Workflow, doc('theirs'));
    expect(session.getSnapshot()).toMatchObject({ phase: 'idle', revision: 4, dirty: false, theirs: undefined });
    expect(updateDraft).toHaveBeenCalledTimes(1);
  });

  it('asks again after a conflict if the other copy cannot be read, and says why', async () => {
    const { session, updateDraft, getWorkflow } = setup();
    updateDraft.mockRejectedValueOnce(conflict());
    getWorkflow.mockRejectedValueOnce(lost());
    session.update(doc('mine'));
    expect(await session.save()).toBe('failed');
    expect(session.getSnapshot().phase).toBe('failed');
  });

  it('never has two saves in flight: a second call waits for the first, then saves what changed meanwhile', async () => {
    const { session, updateDraft } = setup();
    let release: (w: Workflow) => void = () => undefined;
    updateDraft.mockImplementationOnce(() => new Promise<Workflow>((resolve) => (release = resolve)));
    session.update(doc('v2'));
    const first = session.save();
    session.update(doc('v3'));
    const second = session.save();
    expect(updateDraft).toHaveBeenCalledTimes(1);
    updateDraft.mockResolvedValueOnce(workflow(5, doc('v3')));
    release(workflow(4, doc('v2')));
    await Promise.all([first, second]);
    expect(updateDraft.mock.calls.map((c) => [c[1], (c[2] as { nodes: { activity: string }[] }).nodes[0]?.activity])).toEqual([
      [3, 'v2'],
      [4, 'v3'],
    ]);
    expect(session.getSnapshot()).toMatchObject({ revision: 5, dirty: false });
  });

  it('says it is saving while a save is in flight', async () => {
    const { session, updateDraft } = setup();
    let release: (w: Workflow) => void = () => undefined;
    updateDraft.mockImplementationOnce(() => new Promise<Workflow>((resolve) => (release = resolve)));
    session.update(doc('v2'));
    const pending = session.save();
    expect(session.getSnapshot().phase).toBe('saving');
    release(workflow(4, doc('v2')));
    await pending;
    expect(session.getSnapshot().phase).toBe('idle');
  });

  it('says so when the document keeps changing under the saves, instead of saying everything is saved', async () => {
    const { session, updateDraft } = setup();
    let n = 0;
    updateDraft.mockImplementation(async () => {
      n += 1;
      session.update(doc(`typed-${n}`)); // the author types while the save is under way
      return workflow(3 + n, doc(`saved-${n}`));
    });
    session.update(doc('v2'));
    await expect(session.save()).resolves.toBe('changing');
    expect(session.getSnapshot().dirty).toBe(true);
    expect(updateDraft).toHaveBeenCalledTimes(4);
  });
});
