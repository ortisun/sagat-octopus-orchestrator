import { describe, expect, it } from 'vitest';
import { EditError, addNode } from '../../model/edit';
import { emptyDocument } from '../../model/ir';
import { EditorStore } from './useEditorStore';

describe('EditorStore', () => {
  it('lets two changes in one event see each other', () => {
    const store = new EditorStore(emptyDocument('x-flow'));
    store.edit((d) => addNode(d, 'activity', 'a'));
    store.edit((d) => addNode(d, 'activity', 'b'));
    expect(store.doc().nodes.map((n) => n.id)).toEqual(['a', 'b']);
  });

  it('changes nothing when a change is refused, and does not touch the history', () => {
    const store = new EditorStore(emptyDocument('x-flow'));
    store.edit((d) => addNode(d, 'activity', 'a'));
    const before = store.getSnapshot();
    expect(() => store.edit((d) => addNode(d, 'teleport'))).toThrow(EditError);
    expect(store.getSnapshot()).toBe(before);
  });

  it('can go back 100 steps and no further', () => {
    const store = new EditorStore(emptyDocument('x-flow'));
    for (let i = 0; i < 130; i++) store.edit((d) => addNode(d, 'end'));
    let undone = 0;
    while (store.undo()) undone++;
    expect(undone).toBe(100);
    expect(store.doc().nodes).toHaveLength(30);
    let redone = 0;
    while (store.redo()) redone++;
    expect(redone).toBe(100);
    expect(store.doc().nodes).toHaveLength(130);
  });

  it('selects together with a change, and a selection is not a change of the document', () => {
    const store = new EditorStore(emptyDocument('x-flow'));
    store.edit((d) => addNode(d, 'activity', 'a'), { select: 'a' });
    expect(store.getSnapshot().selectedId).toBe('a');
    store.select(undefined);
    expect(store.getSnapshot().history.past).toHaveLength(1);
    expect(store.edit((d) => d, { select: 'a' })).toBe(false);
  });

  it('starts over with a document of its own and no history', () => {
    const store = new EditorStore(emptyDocument('x-flow'));
    store.edit((d) => addNode(d, 'activity', 'a'), { select: 'a' });
    store.reset(emptyDocument('other-flow'));
    expect(store.doc().metadata.name).toBe('other-flow');
    expect(store.undo()).toBe(false);
    expect(store.getSnapshot().selectedId).toBeUndefined();
  });
});
