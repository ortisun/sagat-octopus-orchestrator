import type { ButtonHTMLAttributes } from 'react';

type Variant = 'default' | 'primary' | 'danger';

export function Button({ variant = 'default', className, type = 'button', ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  const classes = ['btn', variant === 'default' ? '' : variant, className ?? ''].filter(Boolean).join(' ');
  return <button type={type} className={classes} {...rest} />;
}
