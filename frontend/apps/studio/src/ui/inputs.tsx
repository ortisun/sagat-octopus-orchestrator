import { CheckboxInput } from '@astryxdesign/core/CheckboxInput';
import { RadioList, RadioListItem } from '@astryxdesign/core/RadioList';
import { Selector } from '@astryxdesign/core/Selector';
import { TextArea as AstryxTextArea } from '@astryxdesign/core/TextArea';
import { TextInput as AstryxTextInput } from '@astryxdesign/core/TextInput';
import type { AriaAttributes, Ref } from 'react';

/** What every labelled control of the Studio takes: its label, a hint, and an error that is announced. */
interface Labelled {
  label: string;
  /** Keep the label for assistive technology and hide it from sight (a filter beside its own heading). */
  hideLabel?: boolean | undefined;
  hint?: string | undefined;
  error?: string | undefined;
  disabled?: boolean | undefined;
  required?: boolean | undefined;
  className?: string | undefined;
  width?: number | string | undefined;
}

const status = (error: string | undefined) => (error ? { status: { type: 'error' as const, message: error } } : {});
const common = ({ label, hideLabel, hint, error, disabled, required, className, width }: Labelled) => ({
  label,
  isLabelHidden: hideLabel === true,
  ...(hint ? { description: hint } : {}),
  ...status(error),
  isDisabled: disabled === true,
  isRequired: required === true,
  ...(className ? { containerClassName: className } : {}),
  width: width ?? '100%',
});

export interface TextInputProps extends Labelled, AriaAttributes {
  value: string;
  onChange: (value: string) => void;
  type?: 'text' | 'password' | 'email' | 'search' | 'url';
  /** Digits are expected: a touch keyboard shows the number pad. The value stays the text that was typed. */
  numeric?: boolean | undefined;
  /** A number with a fraction is expected: a touch keyboard shows the pad with a decimal point. The value stays the text that was typed. */
  decimal?: boolean | undefined;
  maxLength?: number | undefined;
  placeholder?: string | undefined;
  autoComplete?: string | undefined;
  autoFocus?: boolean | undefined;
  readOnly?: boolean | undefined;
  spellCheck?: boolean | undefined;
  mono?: boolean | undefined;
  onEnter?: (() => void) | undefined;
  onBlur?: (() => void) | undefined;
  id?: string | undefined;
  ref?: Ref<HTMLInputElement> | undefined;
}

/** One line of text, with its label, hint and error tied together for assistive technology. */
export function TextInput({ value, onChange, type = 'text', numeric, decimal, maxLength, autoFocus, readOnly, spellCheck, mono, onBlur, label, hideLabel, hint, error, disabled, required, className, width, ...rest }: TextInputProps) {
  return (
    <AstryxTextInput
      {...common({ label, hideLabel, hint, error, disabled, required, className, width })}
      type={type}
      value={value}
      onChange={(next) => onChange(next)}
      hasAutoFocus={autoFocus === true}
      isReadOnly={readOnly === true}
      {...(numeric ? { inputMode: 'numeric' as const } : decimal ? { inputMode: 'decimal' as const } : {})}
      {...(spellCheck !== undefined ? { hasSpellCheck: spellCheck } : {})}
      {...(mono ? { controlClassName: 'ui-mono' } : {})}
      {...(onBlur ? { onBlur } : {})}
      {...(maxLength !== undefined ? ({ maxLength } as object) : {})}
      {...rest}
    />
  );
}

export interface TextAreaProps extends Labelled, AriaAttributes {
  value: string;
  onChange: (value: string) => void;
  rows?: number | undefined;
  maxLength?: number | undefined;
  placeholder?: string | undefined;
  readOnly?: boolean | undefined;
  spellCheck?: boolean | undefined;
  mono?: boolean | undefined;
  onBlur?: (() => void) | undefined;
  id?: string | undefined;
  ref?: Ref<HTMLTextAreaElement> | undefined;
}

/** Several lines of text. */
export function TextArea({ value, onChange, rows = 3, maxLength, readOnly, spellCheck, mono, onBlur, label, hideLabel, hint, error, disabled, required, className, width, ...rest }: TextAreaProps) {
  return (
    <AstryxTextArea
      {...common({ label, hideLabel, hint, error, disabled, required, className, width })}
      value={value}
      // The design system counts against `maxLength` and lets the text run over; the Studio's limits
      // are limits of the platform, so what is typed or pasted beyond one is dropped.
      onChange={(next) => onChange(maxLength !== undefined && next.length > maxLength ? next.slice(0, maxLength) : next)}
      rows={rows}
      isReadOnly={readOnly === true}
      {...(spellCheck !== undefined ? { hasSpellCheck: spellCheck } : {})}
      {...(mono ? { controlClassName: 'ui-mono' } : {})}
      {...(onBlur ? { onBlur } : {})}
      {...(maxLength !== undefined ? ({ maxLength } as object) : {})}
      {...rest}
    />
  );
}

export interface SelectOption<V extends string = string> {
  value: V;
  label: string;
  disabled?: boolean | undefined;
}

export interface SelectProps<V extends string> extends Labelled {
  value: V | '';
  onChange: (value: V) => void;
  options: readonly SelectOption<V>[];
  /** What the control says while nothing is chosen. */
  placeholder?: string | undefined;
  id?: string | undefined;
}

/**
 * One choice of a list. Astryx draws it as a button with the `combobox` role that opens a `listbox`
 * (Decision 2403), not as the platform's native picker; an empty value shows the placeholder.
 */
export function Select<V extends string>({ value, onChange, options, placeholder, label, hideLabel, hint, error, disabled, required, className, width, ...rest }: SelectProps<V>) {
  return (
    <Selector
      {...common({ label, hideLabel, hint, error, disabled, required, className, width })}
      value={value === '' ? undefined : value}
      onChange={(next) => {
        if (next !== null) onChange(next as V);
      }}
      options={options.map((option) => ({ value: option.value, label: option.label, ...(option.disabled ? { disabled: true } : {}) }))}
      // The value behind the words: what is chosen on the control, and each option's own value in the
      // list. A person reads the labels; a browser journey and a test choose by the value, which does
      // not change when a label is reworded.
      data-value={value}
      renderOption={(option) => <span data-option-value={option.value}>{option.label}</span>}
      {...(placeholder ? { placeholder } : {})}
      {...rest}
    />
  );
}

export interface CheckboxProps extends AriaAttributes {
  label: string;
  hint?: string | undefined;
  /** What is missing, tied to the checkbox and announced, as for every other field. */
  error?: string | undefined;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean | undefined;
  hideLabel?: boolean | undefined;
  id?: string | undefined;
}

/** A yes or no. */
export function Checkbox({ label, hint, error, checked, onChange, disabled, hideLabel, ...rest }: CheckboxProps) {
  return (
    <CheckboxInput label={label} isLabelHidden={hideLabel === true} {...(hint ? { description: hint } : {})} {...status(error)} value={checked} onChange={(next) => onChange(next)} isDisabled={disabled === true} {...rest} />
  );
}

export interface RadioGroupProps<V extends string> {
  /** The question the options answer; it names the group. */
  label: string;
  hint?: string | undefined;
  error?: string | undefined;
  /** The chosen option, or an empty string while none is. */
  value: V | '';
  onChange: (value: V) => void;
  options: readonly { value: V; label: string }[];
  disabled?: boolean | undefined;
}

/** One answer out of a few that are all shown: a group of radio buttons named by its question. */
export function RadioGroup<V extends string>({ label, hint, error, value, onChange, options, disabled }: RadioGroupProps<V>) {
  return (
    <RadioList label={label} {...(hint ? { description: hint } : {})} {...status(error)} value={value} onChange={(next) => onChange(next as V)} isDisabled={disabled === true}>
      {options.map((option) => (
        <RadioListItem key={option.value} value={option.value} label={option.label} />
      ))}
    </RadioList>
  );
}
