import '../../styles/code-editor.css';
import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Banner, Button, Checkbox, Hint, Row } from '../../ui';
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
  describedBy?: string | undefined;
  invalid?: boolean;
}

type Loading = { kind: 'loading' } | { kind: 'ready'; module: EditorModule } | { kind: 'failed' };

/**
 * A code editor that is never the only way in. Monaco loads on demand; while it loads, and if it
 * cannot (an old browser, a blocked worker, a failed download), a plain text box holds the same text
 * and is fully usable. A person can also choose the plain text box, and the choice is remembered.
 * Monaco traps the Tab key by design, so the editor says how to get out (Ctrl+M), has a visible
 * switch for the same thing and a "Skip editor" button.
 */
export function CodeEditor({ label, model, language, value, onChange, readOnly = false, onProblems, jsonSchema, controlsRef, hint, height = '22rem', describedBy, invalid = false }: CodeEditorProps) {
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
      <Row justify="end">
        {showMonaco ? (
          <>
            <Checkbox label="Tab moves focus out of the editor" checked={tabMovesFocus} onChange={setTabMovesFocus} />
            <Button size="sm" onClick={() => after.current?.focus()}>
              Skip editor
            </Button>
          </>
        ) : null}
        <Button size="sm" aria-pressed={plainChosen} onClick={() => choosePlain(!plainChosen)}>
          {plainChosen ? 'Use the code editor' : 'Use a plain text box'}
        </Button>
      </Row>
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
        // eslint-disable-next-line no-restricted-syntax -- the plain text box stands in for the Monaco editor in the same frame: it needs the editor's height, no wrapping and the editor's own accessible name and description, which the interface module's TextArea does not take
        <textarea
          className="code-textarea"
          aria-label={label}
          aria-invalid={invalid || undefined}
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
        <Hint id={helpId}>
          Inside the editor the Tab key inserts a tab. Press <kbd className="code-editor-key">Ctrl</kbd>+<kbd className="code-editor-key">M</kbd> (<kbd className="code-editor-key">Control</kbd>+<kbd className="code-editor-key">Shift</kbd>+<kbd className="code-editor-key">M</kbd> on a Mac) or tick the box above to let Tab leave the editor; <kbd className="code-editor-key">F1</kbd> opens the commands, <kbd className="code-editor-key">F8</kbd> goes to the next problem.
        </Hint>
      ) : null}
      {hint ? <Hint>{hint}</Hint> : null}
    </div>
  );
}
