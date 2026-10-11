import { useEffect, useEffectEvent, useRef } from 'react';
import type { RefObject } from 'react';
import { createEditor } from './monaco';
import type { EditorHandle, JsonSchemaFor } from './monaco';
import type { ModelRole, Problem } from './monacoSetup';

export interface EditorControls {
  focus: () => void;
  reveal: (line: number, column: number) => void;
}

export interface MonacoEditorProps {
  model: ModelRole;
  language: string;
  value: string;
  readOnly: boolean;
  ariaLabel: string;
  onChange: ((value: string) => void) | undefined;
  onProblems: ((problems: Problem[]) => void) | undefined;
  jsonSchema: JsonSchemaFor;
  /** Whether Tab moves focus out of the editor instead of inserting a tab. */
  tabMovesFocus: boolean;
  controlsRef: RefObject<EditorControls | null> | undefined;
}

/**
 * Monaco in a box. The text belongs to the parent (`value` in, `onChange` out); Monaco keeps its own
 * model, undo stack and cursor, and an outside change of `value` is applied as an edit so undo still
 * works. A different `model` or `language` is a different model, so the editor is made again.
 */
export function MonacoEditor(props: MonacoEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<EditorHandle | null>(null);
  const { model, language, value, readOnly, jsonSchema, tabMovesFocus, controlsRef, onChange, onProblems } = props;
  // The editor outlives renders: what it calls back must be the page's latest handlers, not the ones
  // it was made with.
  const handlers = useRef({ onChange, onProblems });
  useEffect(() => {
    handlers.current = { onChange, onProblems };
  });

  const make = useEffectEvent((element: HTMLElement): EditorHandle => {
    return createEditor(element, {
      role: model,
      language,
      value,
      readOnly,
      ariaLabel: props.ariaLabel,
      jsonSchema,
      onChange: (next) => handlers.current.onChange?.(next),
      onProblems: (problems) => handlers.current.onProblems?.(problems),
    });
  });

  useEffect(() => {
    const element = host.current;
    if (element === null) return undefined;
    const created = make(element);
    handle.current = created;
    if (controlsRef) controlsRef.current = { focus: created.focus, reveal: created.reveal };
    return () => {
      if (controlsRef) controlsRef.current = null;
      handle.current = null;
      created.dispose();
    };
  }, [model, language, controlsRef]);

  useEffect(() => {
    handle.current?.setValue(value);
  }, [value]);
  useEffect(() => {
    handle.current?.setReadOnly(readOnly);
  }, [readOnly]);
  useEffect(() => {
    if (language === 'json') handle.current?.setJsonSchema(jsonSchema);
  }, [jsonSchema, language]);
  useEffect(() => {
    handle.current?.setTabMovesFocus(tabMovesFocus);
  }, [tabMovesFocus]);

  return <div ref={host} className="monaco-host" data-testid="monaco-host" />;
}
