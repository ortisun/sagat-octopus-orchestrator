import { useEffect, useId, useRef, useState } from 'react';
import { canonical } from '../../model/ir';
import { CodeEditor } from '../scripts/CodeEditor';

function pretty(value: Record<string, unknown> | undefined): string {
  return value === undefined ? '' : JSON.stringify(value, null, 2);
}

/**
 * A JSON object edited as text. What is typed is kept as typed; the document changes only when the
 * text is a JSON object (or empty, which removes it), and the field says why when it is not.
 */
export function JsonField({
  label,
  model,
  hint,
  value,
  disabled,
  onCommit,
}: {
  label: string;
  model: 'workflowInputSchema' | 'workflowOutputSchema';
  hint?: string;
  value: Record<string, unknown> | undefined;
  disabled: boolean;
  onCommit: (value: Record<string, unknown> | undefined) => void;
}) {
  const errorId = useId();
  const base = value === undefined ? '' : canonical(value);
  const committedBase = useRef(base);
  useEffect(() => { committedBase.current = base; }, [base]);
  const [pending, setPending] = useState<{ text: string; base: string; error: string | undefined } | undefined>();
  const typing = pending?.base === base ? pending : undefined;
  const text = typing ? typing.text : pretty(value);
  return (
    <div className="field" role="group" aria-label={label} aria-describedby={typing?.error ? errorId : undefined}>
      <h4>{label}</h4>
      <CodeEditor
        label={label}
        model={model}
        language="json"
        jsonSchema={{ kind: 'meta' }}
        value={text}
        height="16rem"
        readOnly={disabled}
        invalid={!!typing?.error}
        describedBy={typing?.error ? errorId : undefined}
        hint={hint}
        onChange={(next) => {
          if (next.trim() === '') {
            committedBase.current = '';
            setPending({ text: next, base: '', error: undefined });
            onCommit(undefined);
            return;
          }
          let parsed: unknown;
          try {
            parsed = JSON.parse(next);
          } catch {
            setPending({ text: next, base: committedBase.current, error: 'That is not valid JSON. The workflow keeps the last valid schema; your unfinished text stays here.' });
            return;
          }
          if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
            setPending({ text: next, base: committedBase.current, error: 'A schema is a JSON object, such as {"type": "object"}. The last valid text is kept.' });
            return;
          }
          committedBase.current = canonical(parsed);
          setPending({ text: next, base: committedBase.current, error: undefined });
          onCommit(parsed as Record<string, unknown>);
        }}
      />
      {typing?.error ? <span id={errorId} className="error" role="alert">{typing.error}</span> : null}
    </div>
  );
}
