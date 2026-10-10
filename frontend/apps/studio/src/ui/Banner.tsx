import type { ReactNode } from 'react';

type Tone = 'error' | 'warn' | 'info' | 'ok';

/** A message in the page. Errors and warnings are announced; information is not. */
export function Banner({ tone, children, title }: { tone: Tone; children: ReactNode; title?: string }) {
  return (
    <div className={`banner ${tone}`} role={tone === 'error' || tone === 'warn' ? 'alert' : 'status'}>
      {title ? <strong>{title} </strong> : null}
      {children}
    </div>
  );
}
