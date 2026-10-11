import { render } from '@testing-library/react';
import { createRef } from 'react';
import type { RefObject } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { EditorControls, MonacoEditorProps } from './MonacoEditor';
import { MonacoEditor } from './MonacoEditor';
import type { EditorInit } from './monaco';

/** Monaco cannot run in jsdom: its facade is replaced by one that records how the component drives it. */
const facade = vi.hoisted(() => ({ created: [] as { init: unknown; handle: Record<string, ReturnType<typeof vi.fn>> }[] }));
vi.mock('./monaco', () => ({
  createEditor: (_host: HTMLElement, init: unknown) => {
    const handle = { setValue: vi.fn(), setReadOnly: vi.fn(), setJsonSchema: vi.fn(), setTabMovesFocus: vi.fn(), focus: vi.fn(), reveal: vi.fn(), dispose: vi.fn() };
    facade.created.push({ init, handle });
    return handle;
  },
}));

const base: MonacoEditorProps = {
  model: 'source',
  language: 'typescript',
  value: 'let a = 1;',
  readOnly: false,
  ariaLabel: 'Source',
  onChange: undefined,
  onProblems: undefined,
  jsonSchema: undefined,
  tabMovesFocus: false,
  controlsRef: undefined,
};

const initOf = (index: number) => facade.created[index]?.init as EditorInit;
const handleOf = (index: number) => facade.created[index]?.handle as Record<string, ReturnType<typeof vi.fn>>;

beforeEach(() => {
  facade.created.length = 0;
});

describe('MonacoEditor', () => {
  it('makes one editor with the text, the language, the name and the read-only flag it was given', () => {
    render(<MonacoEditor {...base} readOnly ariaLabel="Source of x" />);
    expect(facade.created).toHaveLength(1);
    expect(initOf(0)).toMatchObject({ role: 'source', language: 'typescript', value: 'let a = 1;', readOnly: true, ariaLabel: 'Source of x' });
  });

  it('hands the editor\'s changes and problems to the page', () => {
    const onChange = vi.fn();
    const onProblems = vi.fn();
    render(<MonacoEditor {...base} onChange={onChange} onProblems={onProblems} />);
    initOf(0).onChange('let a = 2;');
    initOf(0).onProblems([{ severity: 'error', line: 1, column: 1, message: 'm' }]);
    expect(onChange).toHaveBeenCalledWith('let a = 2;');
    expect(onProblems).toHaveBeenCalledWith([{ severity: 'error', line: 1, column: 1, message: 'm' }]);
  });

  it('calls the latest handler, without making the editor again when the page re-renders', () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = render(<MonacoEditor {...base} onChange={first} />);
    rerender(<MonacoEditor {...base} onChange={second} />);
    expect(facade.created).toHaveLength(1);
    initOf(0).onChange('x');
    expect(second).toHaveBeenCalledWith('x');
    expect(first).not.toHaveBeenCalled();
  });

  it('applies a new text, read-only flag, schema and tab mode to the same editor', () => {
    const { rerender } = render(<MonacoEditor {...base} language="json" />);
    const schema = { kind: 'schema', schema: { type: 'object' } } as const;
    rerender(<MonacoEditor {...base} language="json" value="{}" readOnly tabMovesFocus jsonSchema={schema} />);
    expect(facade.created).toHaveLength(1);
    const handle = handleOf(0);
    expect(handle['setValue']).toHaveBeenLastCalledWith('{}');
    expect(handle['setReadOnly']).toHaveBeenLastCalledWith(true);
    expect(handle['setTabMovesFocus']).toHaveBeenLastCalledWith(true);
    expect(handle['setJsonSchema']).toHaveBeenLastCalledWith(schema);
  });

  it('makes a new editor, and disposes the old one, for another model or language', () => {
    const { rerender } = render(<MonacoEditor {...base} />);
    rerender(<MonacoEditor {...base} model="version" />);
    expect(facade.created).toHaveLength(2);
    expect(handleOf(0)['dispose']).toHaveBeenCalledTimes(1);
    expect(initOf(1)).toMatchObject({ role: 'version' });
    rerender(<MonacoEditor {...base} model="version" language="python" />);
    expect(facade.created).toHaveLength(3);
    expect(handleOf(1)['dispose']).toHaveBeenCalledTimes(1);
  });

  it('disposes the editor when it leaves, and its controls go with it', () => {
    const controlsRef: RefObject<EditorControls | null> = createRef<EditorControls | null>();
    const { unmount } = render(<MonacoEditor {...base} controlsRef={controlsRef} />);
    expect(controlsRef.current).not.toBeNull();
    controlsRef.current?.reveal(3, 4);
    controlsRef.current?.focus();
    expect(handleOf(0)['reveal']).toHaveBeenCalledWith(3, 4);
    expect(handleOf(0)['focus']).toHaveBeenCalled();
    unmount();
    expect(handleOf(0)['dispose']).toHaveBeenCalledTimes(1);
    expect(controlsRef.current).toBeNull();
  });
});
