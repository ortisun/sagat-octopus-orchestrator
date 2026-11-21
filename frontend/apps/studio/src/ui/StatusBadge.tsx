import { Badge as AstryxBadge } from '@astryxdesign/core/Badge';
import type { ReactNode } from 'react';
import type { StatusView } from '../model/status';

/** The tones a state can have. Each is a word and, on a status, a symbol: never colour alone. */
export type Tone = 'idle' | 'running' | 'waiting' | 'ok' | 'bad' | 'skip';
const VARIANT = { idle: 'neutral', running: 'info', waiting: 'warning', ok: 'success', bad: 'error', skip: 'neutral' } as const;

/** A short label for a state or a kind. */
export function Badge({ tone = 'idle', children, title }: { tone?: Tone; children: ReactNode; title?: string }) {
  return (
    <span data-tone={tone} {...(title ? { title } : {})}>
      <AstryxBadge variant={VARIANT[tone]} label={children} />
    </span>
  );
}

/** The state of a run or a step: its symbol and its word, in the state's tone. */
export function StatusBadge({ view }: { view: StatusView }) {
  return (
    <span data-tone={view.tone} title={view.help}>
      <AstryxBadge variant={VARIANT[view.tone]} icon={<span aria-hidden="true">{view.symbol}</span>} label={view.label} />
    </span>
  );
}
