import { describe, expect, it } from 'vitest';
import type { RuntimeEdge, RuntimeNode } from '../../api/types';
import { edgeKindWords, kindView, shortId, waitingWords } from './display';

const node = (over: Partial<RuntimeNode>): RuntimeNode => ({ id: 'n', type: 'activity', status: 'NOT_STARTED', ...over });

describe('what a run graph says about the routes and waits of the structured primitives', () => {
  it('names the three routes of an approval', () => {
    expect(edgeKindWords('approve')).toBe('If approved');
    expect(edgeKindWords('reject')).toBe('If rejected');
    expect(edgeKindWords('expire')).toBe('If expired');
  });

  it('does not fail on an edge kind from a newer server', () => {
    expect(edgeKindWords('teleport' as RuntimeEdge['kind'])).toBe('teleport');
    expect(edgeKindWords('x'.repeat(100) as RuntimeEdge['kind']).length).toBeLessThan(100);
  });

  it('says what an owner waits for, and degrades for a wait it does not know', () => {
    expect(waitingWords(node({ waitingOn: 'approval' }))).toBe('Waiting for an approval decision');
    expect(waitingWords(node({ waitingOn: 'loop' }))).toBe('Waiting for the loop iterations');
    expect(waitingWords(node({ waitingOn: 'collection' }))).toBe('Waiting for the collection items');
    expect(waitingWords(node({ waitingOn: 'future-wait' as RuntimeNode['waitingOn'] }))).toBe('Waiting');
    expect(waitingWords(node({ waitingOn: null }))).toBeUndefined();
  });

  it('shows a node of an unknown kind as unsupported instead of failing', () => {
    const view = kindView(node({ type: 'quantumGate' }));
    expect(view.unsupported).toBe(true);
    expect(view.label).toBe('Unsupported node (quantumGate)');
  });
});

describe('shortId', () => {
  it('is the end of an identifier: they are time-ordered, so the start is the same for everything made within a minute', () => {
    const first = '01a1c079-b050-7696-afaa-f4a5466f8152';
    const second = '01a1c079-b0aa-7696-afaa-f4a5466f8153';
    expect(shortId(first)).toBe('466f8152');
    expect(first.slice(0, 8)).toBe(second.slice(0, 8));
    expect(shortId(first)).not.toBe(shortId(second));
    expect(shortId('abc')).toBe('abc');
  });
});
