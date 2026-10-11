import { beforeEach, describe, expect, it } from 'vitest';
import { installStorage } from '../../../test/storage';
import type { MemoryStorage } from '../../../test/storage';
import { readDirection, readPositions, readView, writeDirection, writePositions, writeView } from './viewPrefs';

let storage: MemoryStorage;
beforeEach(() => {
  storage = installStorage();
});

describe('view preferences', () => {
  it('remembers the view and the direction, with canvas and left-to-right as the defaults', () => {
    expect(readView()).toBe('canvas');
    expect(readDirection()).toBe('LR');
    writeView('list');
    writeDirection('TB');
    expect(readView()).toBe('list');
    expect(readDirection()).toBe('TB');
    storage.setItem('studio.workflows.view', 'something else');
    expect(readView()).toBe('canvas');
  });

  it('keeps the positions of a workflow under its own key, and clears them when there are none', () => {
    writePositions('wf-1', new Map([['a', { x: 10, y: 20 }]]));
    writePositions('wf-2', new Map([['a', { x: 99, y: 99 }]]));
    expect(readPositions('wf-1').get('a')).toEqual({ x: 10, y: 20 });
    expect(readPositions('wf-2').get('a')).toEqual({ x: 99, y: 99 });
    writePositions('wf-1', new Map());
    expect(readPositions('wf-1').size).toBe(0);
    expect(readPositions('wf-2').size).toBe(1);
  });

  it('keeps no document, token or secret there: only positions', () => {
    writePositions('wf-1', new Map([['a', { x: 1.4, y: 2.6 }]]));
    const stored = [...Array(storage.length).keys()].map((i) => storage.getItem(storage.key(i) ?? '') ?? '').join('\n');
    expect(JSON.parse(stored)).toEqual({ v: 1, positions: { a: { x: 1, y: 3 } } });
  });

  it('ignores what is in storage and is not a position', () => {
    const key = 'studio.workflow.layout.wf-1';
    storage.setItem(key, 'not json');
    expect(readPositions('wf-1').size).toBe(0);
    storage.setItem(key, JSON.stringify({ v: 2, positions: { a: { x: 1, y: 1 } } }));
    expect(readPositions('wf-1').size).toBe(0);
    storage.setItem(key, JSON.stringify({ v: 1, positions: { ok: { x: 1, y: 2 }, nan: { x: 'a', y: 1 }, far: { x: 1e9, y: 0 }, nul: null, long: { x: 1, y: 1 } } }));
    expect([...readPositions('wf-1').keys()].sort()).toEqual(['long', 'ok']);
    storage.setItem(key, JSON.stringify({ v: 1, positions: [1, 2] }));
    expect(readPositions('wf-1').size).toBe(0);
  });

  it('carries on without storage: nothing throws when it is blocked', () => {
    storage.broken();
    expect(readView()).toBe('canvas');
    expect(readDirection()).toBe('LR');
    expect(readPositions('wf-1').size).toBe(0);
    expect(() => {
      writeView('list');
      writeDirection('TB');
      writePositions('wf-1', new Map([['a', { x: 1, y: 1 }]]));
    }).not.toThrow();
  });

  it('carries on when there is no storage object at all', () => {
    Object.defineProperty(globalThis, 'localStorage', { value: undefined, configurable: true, writable: true });
    expect(readView()).toBe('canvas');
    expect(() => writeView('list')).not.toThrow();
  });
});
