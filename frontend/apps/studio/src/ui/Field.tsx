import { cloneElement, useId } from 'react';
import type { ReactElement } from 'react';

/** A label, a control, a hint and an error, tied together for assistive technology. */
export function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string | undefined; children: ReactElement<Record<string, unknown>> }) {
  const id = useId();
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  const describedBy = [hint ? hintId : '', error ? errorId : ''].filter(Boolean).join(' ');
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      {cloneElement(children, { id, 'aria-describedby': describedBy || undefined, 'aria-invalid': error ? true : undefined })}
      {hint ? (
        <span id={hintId} className="hint">
          {hint}
        </span>
      ) : null}
      {error ? (
        <span id={errorId} className="error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}
