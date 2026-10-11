import { useState } from 'react';
import { canonical } from '../../model/ir';
import { Field } from '../../ui/Field';

function pretty(value: Record<string, unknown> | undefined): string {
  return value === undefined ? '' : JSON.stringify(value, null, 2);
}

/**
 * A JSON object edited as text. What is typed is kept as typed; the document changes only when the
 * text is a JSON object (or empty, which removes it), and the field says why when it is not.
 */
export function JsonField({
  label,
  hint,
  value,
  disabled,
  onCommit,
}: {
  label: string;
  hint?: string;
  value: Record<string, unknown> | undefined;
  disabled: boolean;
  onCommit: (value: Record<string, unknown> | undefined) => void;
}) {
  const base = value === undefined ? '' : canonical(value);
  const [pending, setPending] = useState<{ text: string; base: string; error: string | undefined } | undefined>();
  const typing = pending?.base === base ? pending : undefined;
  const text = typing ? typing.text : pretty(value);
  return (
    <Field label={label} {...(hint ? { hint } : {})} error={typing?.error}>
      <textarea
        value={text}
        rows={8}
        spellCheck={false}
        disabled={disabled}
        onBlur={() => setPending(undefined)}
        onChange={(event) => {
          const next = event.target.value;
          if (next.trim() === '') {
            setPending({ text: next, base: '', error: undefined });
            onCommit(undefined);
            return;
          }
          let parsed: unknown;
          try {
            parsed = JSON.parse(next);
          } catch (error) {
            setPending({ text: next, base, error: `That is not valid JSON${error instanceof SyntaxError ? `: ${error.message}` : '.'} The last valid text is kept.` });
            return;
          }
          if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
            setPending({ text: next, base, error: 'A schema is a JSON object, such as {"type": "object"}. The last valid text is kept.' });
            return;
          }
          setPending({ text: next, base: canonical(parsed), error: undefined });
          onCommit(parsed as Record<string, unknown>);
        }}
      />
    </Field>
  );
}
