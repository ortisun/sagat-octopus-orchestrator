import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { setField } from '../../model/edit';
import type { IrDocument, IrNode } from '../../model/ir';
import type { FieldSpec } from '../../model/registry';
import { Button, Checkbox, Code, Hint, Select, Stack, TextArea, TextInput } from '../../ui';
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
 * Several controls that are one setting (a duration, the retry policy, one trigger): a named group, so
 * assistive technology says which setting a control belongs to. The interface module has no component
 * for a `fieldset`; the controls inside it are the module's.
 */
export function Group({ label, describedBy, className, children }: { label: string; describedBy?: string | undefined; className?: string; children: ReactNode }) {
  return (
    <fieldset className={className ? `wf-group ${className}` : 'wf-group'} aria-describedby={describedBy}>
      <legend className="wf-group-title">{label}</legend>
      <Stack gap={2}>{children}</Stack>
    </fieldset>
  );
}

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
    <TextInput
      label={spec.label}
      hint={spec.hint}
      value={stringOf(node[spec.name])}
      disabled={disabled}
      required={spec.required === true}
      autoComplete="off"
      onChange={(value) => change((doc) => setField(doc, node.id, spec.name, value), `field:${node.id}:${spec.name}`)}
    />
  );
}

export function ExpressionField({ spec, node, disabled, change }: FieldProps) {
  return (
    <TextArea
      label={spec.label}
      hint={spec.hint}
      value={stringOf(node[spec.name])}
      rows={3}
      spellCheck={false}
      mono
      disabled={disabled}
      required={spec.required === true}
      onChange={(value) => change((doc) => setField(doc, node.id, spec.name, value), `field:${node.id}:${spec.name}`)}
    />
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
    <TextInput
      label={label}
      hint={hint}
      error={error}
      numeric
      autoComplete="off"
      value={typing.shown}
      disabled={disabled}
      required={required === true}
      onBlur={typing.stop}
      onChange={(typed) => {
        const text = typed.trim();
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

const UNIT_OPTIONS = DURATION_UNITS.map((u) => ({ value: u.unit, label: u.unit }));

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
    <Group label={label} className="wf-duration" describedBy={hint ? `${id}-hint` : undefined}>
      <div className="wf-duration-row">
        <TextInput
          label={`${label} amount`}
          hideLabel
          numeric
          autoComplete="off"
          error={bad ? `Enter a whole number of ${unit}.` : undefined}
          required={required === true}
          value={text}
          disabled={disabled}
          onBlur={() => setPending(undefined)}
          onChange={(typed) => commit(typed.trim(), unit)}
        />
        <Select label={`${label} unit`} hideLabel value={unit} disabled={disabled} options={UNIT_OPTIONS} onChange={(next) => commit(text, next)} />
      </div>
      {hint ? <Hint id={`${id}-hint`}>{hint}</Hint> : null}
    </Group>
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
  const options = [...(spec.required === true ? [] : [{ value: '', label: '(not set)' }]), ...(spec.options ?? []).map((option) => ({ value: option, label: option }))];
  return (
    <Select
      label={spec.label}
      hint={spec.hint}
      value={stringOf(node[spec.name])}
      placeholder="(not set)"
      disabled={disabled}
      required={spec.required === true}
      options={options}
      onChange={(value) => change((doc) => setField(doc, node.id, spec.name, value), `field:${node.id}:${spec.name}`)}
    />
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
  return <Checkbox label={label} hint={hint} checked={checked} disabled={disabled} onChange={onChange} />;
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
    <Group label={spec.label} describedBy={spec.hint ? `${id}-hint` : undefined}>
      {tags.length > 0 ? (
        <ul className="wf-chips" aria-label={`${spec.label} in use`}>
          {tags.map((tag) => (
            <li key={tag}>
              <Code>{tag}</Code>
              {disabled ? null : (
                <Button variant="ghost" size="sm" aria-label={`Remove ${spec.label.toLowerCase()} ${tag}`} onClick={() => set(tags.filter((t) => t !== tag))}>
                  <span aria-hidden="true">×</span>
                </Button>
              )}
            </li>
          ))}
        </ul>
      ) : null}
      {disabled ? null : (
        <div className="wf-add-row">
          <TextInput
            label={`Add to ${spec.label.toLowerCase()}`}
            hideLabel
            autoComplete="off"
            error={error}
            value={text}
            onChange={(value) => {
              setText(value);
              setError(undefined);
            }}
            onEnter={add}
          />
          <Button onClick={add}>Add</Button>
        </div>
      )}
      {spec.hint ? <Hint id={`${id}-hint`}>{spec.hint}</Hint> : null}
    </Group>
  );
}

const STRATEGY_OPTIONS = [
  { value: '', label: '(default)' },
  { value: 'fixed', label: 'fixed' },
  { value: 'exponential', label: 'exponential' },
];

export function RetryFields({ spec, node, disabled, change }: FieldProps) {
  const raw = node[spec.name];
  const retry = typeof raw === 'object' && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const set = (field: string, value: unknown) => change((doc) => setRetryField(doc, node.id, field, value), `retry:${node.id}:${field}`);
  const num = (field: string): number | undefined => (typeof retry[field] === 'number' ? retry[field] : undefined);
  return (
    <Group label={spec.label}>
      <NumberInput
        label="Most attempts"
        hint="1 to 100. Empty: the platform's default."
        value={num('maxAttempts')}
        min={1}
        max={100}
        disabled={disabled}
        onCommit={(v) => set('maxAttempts', v)}
      />
      <Select
        label="Delay between attempts"
        value={stringOf(retry['strategy'])}
        placeholder="(default)"
        disabled={disabled}
        options={STRATEGY_OPTIONS}
        onChange={(value) => set('strategy', value === '' ? undefined : value)}
      />
      <DurationInput label="First delay" value={num('initialDelayMs')} disabled={disabled} onCommit={(ms) => set('initialDelayMs', ms)} />
      <DurationInput label="Longest delay" value={num('maxDelayMs')} disabled={disabled} onCommit={(ms) => set('maxDelayMs', ms)} />
      <CheckField
        label="Add jitter"
        hint="Spreads the retries of many runs so they do not arrive together."
        checked={retry['jitter'] === true}
        disabled={disabled}
        onChange={(checked) => set('jitter', checked ? true : undefined)}
      />
    </Group>
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
