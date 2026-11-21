import { Field as AstryxField } from '@astryxdesign/core/Field';
import { cloneElement, useId } from 'react';
import type { ReactElement } from 'react';

/**
 * A label, a hint and an error around a control that is not one of the Studio's own inputs (the code
 * editor, a composite of two controls). `TextInput`, `TextArea`, `Select` and `Checkbox` carry their
 * own label: use those for an ordinary field.
 */
export function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string | undefined; children: ReactElement<Record<string, unknown>> }) {
  const id = useId();
  const hintId = `${id}-hint`;
  const errorId = `${id}-error`;
  const labelId = `${id}-label`;
  const describedBy = [hint ? hintId : '', error ? errorId : ''].filter(Boolean).join(' ');
  return (
    <AstryxField label={label} inputID={id} labelID={labelId} {...(hint ? { description: hint, descriptionID: hintId } : {})} {...(error ? { status: { type: 'error' as const, message: error, messageID: errorId } } : {})} width="100%">
      {cloneElement(children, { id, 'aria-labelledby': labelId, 'aria-describedby': describedBy || undefined, 'aria-invalid': error ? true : undefined })}
    </AstryxField>
  );
}
