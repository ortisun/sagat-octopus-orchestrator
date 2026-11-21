import { Button as AstryxButton } from '@astryxdesign/core/Button';
import type { AriaAttributes, MouseEvent, ReactNode, Ref } from 'react';

type Variant = 'default' | 'primary' | 'danger' | 'ghost';
const VARIANT = { default: 'secondary', primary: 'primary', danger: 'destructive', ghost: 'ghost' } as const;

export interface ButtonProps extends AriaAttributes {
  /** What the button says. A string is also its accessible name unless `aria-label` gives a fuller one. */
  children: ReactNode;
  variant?: Variant;
  type?: 'button' | 'submit' | 'reset';
  size?: 'sm' | 'md' | 'lg';
  /** A disabled button is announced as unavailable and activating it does nothing (D2403). The design system marks it with the native attribute or, when it carries an explanation or is busy, with `aria-disabled` so that it stays reachable. */
  disabled?: boolean | undefined;
  /** The action is in flight: the button shows it and ignores another activation. */
  busy?: boolean | undefined;
  onClick?: ((event: MouseEvent) => void) | undefined;
  /** An explanation shown on hover and focus. */
  title?: string | undefined;
  className?: string | undefined;
  id?: string | undefined;
  form?: string | undefined;
  ref?: Ref<HTMLButtonElement> | undefined;
}

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(textOf).join('');
  return '';
}

/** An action. Astryx draws it; the Studio keeps its own words for the variants. */
export function Button({ children, variant = 'default', disabled, busy, title, 'aria-label': ariaLabel, ...rest }: ButtonProps) {
  const label = ariaLabel ?? textOf(children);
  return (
    <AstryxButton label={label} variant={VARIANT[variant]} isDisabled={disabled === true} isLoading={busy === true} {...(title ? { tooltip: title } : {})} {...rest}>
      {children}
    </AstryxButton>
  );
}
