import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Banner } from '../../ui/Banner';
import { loadEditor } from './editorModule';
import type { EditorModule } from './editorModule';
import type { EditorControls } from './MonacoEditor';
import type { JsonSchemaFor } from './monaco';
import type { ModelRole, Problem } from './monacoSetup';
import { readPlainPreference, writePlainPreference } from './preference';

export type { EditorControls } from './MonacoEditor';

export interface CodeEditorProps {
  /** The accessible name of the editor. */
  label: string;
  /** Which model of the page this is (its address, and so its schema and language service). */
  model: ModelRole;
  /** The Monaco language of the text. */
  language: string;
  value: string;
  onChange?: (value: string) => void;
  readOnly?: boolean;
  onProblems?: (problems: Problem[]) => void;
  jsonSchema?: JsonSchemaFor;
  controlsRef?: RefObject<EditorControls | null>;
  /** What the browser can check in this language, shown under the editor. */
  hint?: ReactNode;
  /** CSS height of the editor. */
  height?: string;
  /** The id of text that describes the editor (the limit, the problems...). */
  describedBy?: string;
}

type Loading = { kind: 'loading' } | { kind: 'ready'; module: EditorModule } | { kind: 'failed' };

/**
 * A code editor that is never the only way in. Monaco loads on demand; while it loads, and if it
 * cannot (an old browser, a blocked worker, a failed download), a plain text box holds the same text
 * and is fully usable. A person can also choose the plain text box, and the choice is remembered.
 * Monaco traps the Tab key by design, so the editor says how to get out (Ctrl+M), has a visible
 * switch for the same thing and a "Skip editor" button.
 */
export function CodeEditor({ label, model, language, value, onChange, readOnly = false, onProblems, jsonSchema, controlsRef, hint, height = '22rem', describedBy }: CodeEditorProps) {
  const [plainChosen, setPlainChosen] = useState(readPlainPreference);
  const [loading, setLoading] = useState<Loading>({ kind: 'loading' });
  const [tabMovesFocus, setTabMovesFocus] = useState(false);
  const after = useRef<HTMLSpanElement>(null);
  const helpId = useId();

  useEffect(() => {
    if (plainChosen) return undefined;
    let current = true;
    loadEditor().then(
      (module) => {
        if (current) setLoading({ kind: 'ready', module });
      },
      () => {
        if (current) setLoading({ kind: 'failed' });
      },
    );
    return () => {
      current = false;
    };
  }, [plainChosen]);

  const choosePlain = (plain: boolean) => {
    setPlainChosen(plain);
    writePlainPreference(plain);
    if (!plain) setLoading({ kind: 'loading' });
  };

  const showMonaco = !plainChosen && loading.kind === 'ready';
  const MonacoEditor = loading.kind === 'ready' ? loading.module.MonacoEditor : undefined;
  const describedByIds = [describedBy, showMonaco ? helpId : undefined].filter(Boolean).join(' ') || undefined;

  return (
    <div className="code-editor" data-mode={showMonaco ? 'monaco' : 'plain'}>
      {!plainChosen && loading.kind === 'failed' ? (
        <Banner tone="warn" title="The code editor could not be loaded.">
          You can keep working in a plain text box: it holds the same text and is saved the same way. It has no highlighting or autocomplete.
        </Banner>
      ) : null}
      <div className="code-editor-bar">
        {showMonaco ? (
          <>
            <label className="inline-check">
              <input type="checkbox" checked={tabMovesFocus} onChange={(event) => setTabMovesFocus(event.target.checked)} />
              <span>Tab moves focus out of the editor</span>
            </label>
            <button type="button" className="btn" onClick={() => after.current?.focus()}>
              Skip editor
            </button>
          </>
        ) : null}
        <button type="button" className="btn" aria-pressed={plainChosen} onClick={() => choosePlain(!plainChosen)}>
          {plainChosen ? 'Use the code editor' : 'Use a plain text box'}
        </button>
      </div>
      {showMonaco && MonacoEditor ? (
        <div className="code-editor-frame" style={{ height }}>
          <MonacoEditor
            model={model}
            language={language}
            value={value}
            readOnly={readOnly}
            ariaLabel={label}
            onChange={onChange}
            onProblems={onProblems}
            jsonSchema={jsonSchema}
            tabMovesFocus={tabMovesFocus}
            controlsRef={controlsRef}
          />
        </div>
      ) : !plainChosen && loading.kind === 'loading' ? (
        <div className="code-editor-frame code-editor-loading" style={{ height }} role="status">
          Loading the code editor…
        </div>
      ) : (
        <textarea
          className="code-textarea"
          aria-label={label}
          aria-describedby={describedByIds}
          value={value}
          readOnly={readOnly}
          spellCheck={false}
          autoCapitalize="off"
          autoComplete="off"
          autoCorrect="off"
          wrap="off"
          style={{ height }}
          onChange={(event) => onChange?.(event.target.value)}
        />
      )}
      <span ref={after} tabIndex={-1} className="visually-hidden">
        End of the code editor
      </span>
      {showMonaco ? (
        <p id={helpId} className="hint">
          Inside the editor the Tab key inserts a tab. Press <kbd>Ctrl</kbd>+<kbd>M</kbd> (<kbd>Control</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> on a Mac) or tick the box above to let Tab leave the editor; <kbd>F1</kbd> opens the commands, <kbd>F8</kbd> goes to the next problem.
        </p>
      ) : null}
      {hint ? <p className="hint">{hint}</p> : null}
    </div>
  );
}
