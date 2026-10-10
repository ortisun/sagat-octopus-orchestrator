import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { setField } from '../../model/edit';
import type { IrDocument, IrNode } from '../../model/ir';
import type { FieldSpec } from '../../model/registry';
import { Button } from '../../ui/Button';
import { Field } from '../../ui/Field';
import { DURATION_UNITS, splitDuration, toMs } from './duration';
import type { DurationUnit } from './duration';
import { setRetryField } from './operations';

/** How a field reports a change: a function of the document, and the key that folds typing into one undo step. */
export type Change = (change: (doc: IrDocument) => IrDocument, key: string) => void;

interface FieldProps {
  spec: FieldSpec;
  node: IrNode;
  disabled: boolean;
  change: Change;
}

const stringOf = (value: unknown): string => (typeof value === 'string' ? value : '');

/**
 * Text being typed that is not (yet) a value: a number field that holds "12x" shows it and says why,
 * and the document keeps the last good value. The text is tied to the value it was typed over, so an
 * undo (which changes the value) shows the value, not the old typing.
 */
function useTyping(committed: string) {
  const [pending, setPending] = useState<{ text: string; base: string } | undefined>();
  return {
    shown: pending !== undefined && pending.base === committed ? pending.text : committed,
    type: (text: string, base: string) => setPending({ text, base }),
    stop: () => setPending(undefined),
  };
}

export function TextField({ spec, node, disabled, change }: FieldProps) {
  return (
    <Field label={spec.label} {...(spec.hint ? { hint: spec.hint } : {})}>
      <input
        type="text"
        value={stringOf(node[spec.name])}
        disabled={disabled}
        aria-required={spec.required === true ? true : undefined}
        autoComplete="off"
        onChange={(event) => change((doc) => setField(doc, node.id, spec.name, event.target.value), `field:${node.id}:${spec.name}`)}
      />
    </Field>
  );
}

export function ExpressionField({ spec, node, disabled, change }: FieldProps) {
  return (
    <Field label={spec.label} {...(spec.hint ? { hint: spec.hint } : {})}>
      <textarea
        value={stringOf(node[spec.name])}
        rows={3}
        spellCheck={false}
        disabled={disabled}
        aria-required={spec.required === true ? true : undefined}
        onChange={(event) => change((doc) => setField(doc, node.id, spec.name, event.target.value), `field:${node.id}:${spec.name}`)}
      />
    </Field>
  );
}

function parseWhole(text: string, min: number, max: number): { value: number } | { error: string } {
  if (!/^\d{1,10}$/.test(text)) return { error: 'Enter a whole number.' };
  const value = Number(text);
  if (value < min || value > max) return { error: `Enter a number from ${min} to ${max}.` };
  return { value };
}

/** A whole number, also used by the retry sub-form. */
export function NumberInput({
  label,
  hint,
  value,
  min,
  max,
  disabled,
  required,
  onCommit,
}: {
  label: string;
  hint?: string | undefined;
  value: number | undefined;
  min: number;
  max: number;
  disabled: boolean;
  required?: boolean | undefined;
  onCommit: (value: number | undefined) => void;
}) {
  const committed = value === undefined ? '' : String(value);
  const typing = useTyping(committed);
  const parsed = typing.shown === '' ? undefined : parseWhole(typing.shown, min, max);
  const error = parsed !== undefined && 'error' in parsed ? parsed.error : undefined;
  return (
    <Field label={label} {...(hint ? { hint } : {})} error={error}>
      <input
        type="text"
        inputMode="numeric"
        autoComplete="off"
        value={typing.shown}
        disabled={disabled}
        aria-required={required === true ? true : undefined}
        onBlur={typing.stop}
        onChange={(event) => {
          const text = event.target.value.trim();
          const result = text === '' ? undefined : parseWhole(text, min, max);
          if (result === undefined) {
            typing.type('', committed);
            onCommit(undefined);
          } else if ('value' in result) {
            typing.type(text, String(result.value));
            onCommit(result.value);
          } else {
            typing.type(text, committed);
          }
        }}
      />
    </Field>
  );
}

export function IntegerField({ spec, node, disabled, change }: FieldProps) {
  const raw = node[spec.name];
  return (
    <NumberInput
      label={spec.label}
      hint={spec.hint}
      value={typeof raw === 'number' ? raw : undefined}
      min={spec.min ?? 0}
      max={spec.max ?? 2_000_000_000}
      disabled={disabled}
      required={spec.required}
      onCommit={(value) => change((doc) => setField(doc, node.id, spec.name, value), `field:${node.id}:${spec.name}`)}
    />
  );
}

/** An amount and a unit; what is stored is milliseconds. */
export function DurationInput({
  label,
  hint,
  value,
  disabled,
  required,
  onCommit,
}: {
  label: string;
  hint?: string | undefined;
  value: number | undefined;
  disabled: boolean;
  required?: boolean | undefined;
  onCommit: (ms: number | undefined) => void;
}) {
  const id = useId();
  const split = value === undefined ? { amount: undefined, unit: 'ms' as DurationUnit } : splitDuration(value);
  const committed = `${split.amount ?? ''}${split.unit}`;
  const [pending, setPending] = useState<{ text: string; unit: DurationUnit; base: string } | undefined>();
  const text = pending?.base === committed ? pending.text : String(split.amount ?? '');
  const unit = pending?.base === committed ? pending.unit : split.unit;
  const bad = text !== '' && (!/^\d{1,12}$/.test(text) || toMs(Number(text), unit) === undefined);

  const commit = (nextText: string, nextUnit: DurationUnit) => {
    if (nextText === '') {
      // An empty field stores nothing, which reads back as "ms"; the unit chosen stays.
      setPending({ text: '', unit: nextUnit, base: 'ms' });
      onCommit(undefined);
      return;
    }
    const ms = /^\d{1,12}$/.test(nextText) ? toMs(Number(nextText), nextUnit) : undefined;
    if (ms === undefined) {
      setPending({ text: nextText, unit: nextUnit, base: committed });
      return;
    }
    const after = splitDuration(ms);
    // The base is what the document will say after this commit, so the typed text stays as typed.
    setPending({ text: nextText, unit: nextUnit, base: `${after.amount}${after.unit}` });
    onCommit(ms);
  };

  return (
    <fieldset className="field wf-duration" aria-describedby={hint ? `${id}-hint` : undefined}>
      <legend>{label}</legend>
      <div className="wf-duration-row">
        <input
          type="text"
          inputMode="numeric"
          autoComplete="off"
          aria-label={`${label} amount`}
          aria-invalid={bad ? true : undefined}
          aria-required={required === true ? true : undefined}
          value={text}
          disabled={disabled}
          onBlur={() => setPending(undefined)}
          onChange={(event) => commit(event.target.value.trim(), unit)}
        />
        <select aria-label={`${label} unit`} value={unit} disabled={disabled} onChange={(event) => commit(text, event.target.value as DurationUnit)}>
          {DURATION_UNITS.map((u) => (
            <option key={u.unit} value={u.unit}>
              {u.unit}
            </option>
          ))}
        </select>
      </div>
      {hint ? (
        <span id={`${id}-hint`} className="hint">
          {hint}
        </span>
      ) : null}
      {bad ? (
        <span className="error" role="alert">
          Enter a whole number of {unit}.
        </span>
      ) : null}
    </fieldset>
  );
}

export function DurationField({ spec, node, disabled, change }: FieldProps) {
  const raw = node[spec.name];
  return (
    <DurationInput
      label={spec.label}
      hint={spec.hint}
      value={typeof raw === 'number' ? raw : undefined}
      disabled={disabled}
      required={spec.required}
      onCommit={(ms) => change((doc) => setField(doc, node.id, spec.name, ms), `field:${node.id}:${spec.name}`)}
    />
  );
}

export function SelectField({ spec, node, disabled, change }: FieldProps) {
  return (
    <Field label={spec.label} {...(spec.hint ? { hint: spec.hint } : {})}>
      <select
        value={stringOf(node[spec.name])}
        disabled={disabled}
        aria-required={spec.required === true ? true : undefined}
        onChange={(event) => change((doc) => setField(doc, node.id, spec.name, event.target.value), `field:${node.id}:${spec.name}`)}
      >
        {spec.required === true ? null : <option value="">(not set)</option>}
        {(spec.options ?? []).map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    </Field>
  );
}

export function CheckField({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  hint?: string | undefined;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="field wf-check">
      <label>
        <input type="checkbox" checked={checked} disabled={disabled} aria-describedby={hint ? `${id}-hint` : undefined} onChange={(event) => onChange(event.target.checked)} />{' '}
        {label}
      </label>
      {hint ? (
        <span id={`${id}-hint`} className="hint">
          {hint}
        </span>
      ) : null}
    </div>
  );
}

export function BooleanField({ spec, node, disabled, change }: FieldProps) {
  return (
    <CheckField
      label={spec.label}
      hint={spec.hint}
      checked={node[spec.name] === true}
      disabled={disabled}
      onChange={(checked) => change((doc) => setField(doc, node.id, spec.name, checked ? true : undefined), `field:${node.id}:${spec.name}`)}
    />
  );
}

const SECRET_NAME = /^[a-z][a-z0-9._-]{0,63}$/;
const MAX_TAGS = 16;

export function TagsField({ spec, node, disabled, change }: FieldProps) {
  const raw = node[spec.name];
  const tags = Array.isArray(raw) ? raw.filter((t): t is string => typeof t === 'string') : [];
  const [text, setText] = useState('');
  const [error, setError] = useState<string | undefined>();
  const id = useId();
  const set = (next: string[]) => change((doc) => setField(doc, node.id, spec.name, next.length === 0 ? undefined : next), `tags:${node.id}:${spec.name}`);
  const add = () => {
    const value = text.trim();
    if (value === '') return;
    if (!SECRET_NAME.test(value)) return setError('A name starts with a lowercase letter and holds lowercase letters, digits, dots, dashes or underscores (up to 64).');
    if (tags.includes(value)) return setError(`"${value}" is already in the list.`);
    if (tags.length >= MAX_TAGS) return setError(`At most ${MAX_TAGS}.`);
    setError(undefined);
    setText('');
    set([...tags, value]);
  };
  return (
    <fieldset className="field wf-tags" aria-describedby={spec.hint ? `${id}-hint` : undefined}>
      <legend>{spec.label}</legend>
      {tags.length > 0 ? (
        <ul className="wf-chips" aria-label={`${spec.label} in use`}>
          {tags.map((tag) => (
            <li key={tag}>
              <code>{tag}</code>
              {disabled ? null : (
                <button type="button" className="wf-chip-remove" aria-label={`Remove ${spec.label.toLowerCase()} ${tag}`} onClick={() => set(tags.filter((t) => t !== tag))}>
                  <span aria-hidden="true">×</span>
                </button>
              )}
            </li>
          ))}
        </ul>
      ) : null}
      {disabled ? null : (
        <div className="wf-duration-row">
          <input
            type="text"
            autoComplete="off"
            aria-label={`Add to ${spec.label.toLowerCase()}`}
            aria-invalid={error ? true : undefined}
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              setError(undefined);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                add();
              }
            }}
          />
          <Button onClick={add}>Add</Button>
        </div>
      )}
      {spec.hint ? (
        <span id={`${id}-hint`} className="hint">
          {spec.hint}
        </span>
      ) : null}
      {error ? (
        <span className="error" role="alert">
          {error}
        </span>
      ) : null}
    </fieldset>
  );
}

export function RetryFields({ spec, node, disabled, change }: FieldProps) {
  const raw = node[spec.name];
  const retry = typeof raw === 'object' && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const set = (field: string, value: unknown) => change((doc) => setRetryField(doc, node.id, field, value), `retry:${node.id}:${field}`);
  const num = (field: string): number | undefined => (typeof retry[field] === 'number' ? retry[field] : undefined);
  return (
    <fieldset className="field wf-retry">
      <legend>{spec.label}</legend>
      <NumberInput
        label="Most attempts"
        hint="1 to 100. Empty: the platform's default."
        value={num('maxAttempts')}
        min={1}
        max={100}
        disabled={disabled}
        onCommit={(v) => set('maxAttempts', v)}
      />
      <Field label="Delay between attempts">
        <select value={stringOf(retry['strategy'])} disabled={disabled} onChange={(event) => set('strategy', event.target.value === '' ? undefined : event.target.value)}>
          <option value="">(default)</option>
          <option value="fixed">fixed</option>
          <option value="exponential">exponential</option>
        </select>
      </Field>
      <DurationInput label="First delay" value={num('initialDelayMs')} disabled={disabled} onCommit={(ms) => set('initialDelayMs', ms)} />
      <DurationInput label="Longest delay" value={num('maxDelayMs')} disabled={disabled} onCommit={(ms) => set('maxDelayMs', ms)} />
      <CheckField
        label="Add jitter"
        hint="Spreads the retries of many runs so they do not arrive together."
        checked={retry['jitter'] === true}
        disabled={disabled}
        onChange={(checked) => set('jitter', checked ? true : undefined)}
      />
    </fieldset>
  );
}

/** The editor of one field of a node, chosen by what kind of value it holds. */
export function FieldEditor(props: FieldProps): ReactNode {
  switch (props.spec.kind) {
    case 'text':
      return <TextField {...props} />;
    case 'expression':
      return <ExpressionField {...props} />;
    case 'integer':
      return <IntegerField {...props} />;
    case 'duration':
      return <DurationField {...props} />;
    case 'select':
      return <SelectField {...props} />;
    case 'boolean':
      return <BooleanField {...props} />;
    case 'tags':
      return <TagsField {...props} />;
    case 'retry':
      return <RetryFields {...props} />;
  }
}
