import { CheckboxList, CheckboxListItem } from '@astryxdesign/core/CheckboxList';

export interface CheckboxGroupProps<V extends string> {
  /** What the options are; it names the group. */
  label: string;
  hint?: string | undefined;
  /** Why the choice is refused (none chosen where one is needed, for example). It is shown under the group. */
  error?: string | undefined;
  /** The chosen options, in any order. */
  value: readonly V[];
  onChange: (value: V[]) => void;
  options: readonly { value: V; label: string; disabled?: boolean | undefined }[];
  disabled?: boolean | undefined;
}

/** Several answers out of a few that are all shown: a group of checkboxes named by what they are. */
export function CheckboxGroup<V extends string>({ label, hint, error, value, onChange, options, disabled }: CheckboxGroupProps<V>) {
  return (
    <CheckboxList
      label={label}
      {...(hint ? { description: hint } : {})}
      {...(error ? { status: { type: 'error' as const, message: error } } : {})}
      value={[...value]}
      onChange={(next) => onChange(next as V[])}
      isDisabled={disabled === true}
      width="100%"
    >
      {options.map((option) => (
        <CheckboxListItem key={option.value} value={option.value} label={option.label} isDisabled={option.disabled === true} />
      ))}
    </CheckboxList>
  );
}
