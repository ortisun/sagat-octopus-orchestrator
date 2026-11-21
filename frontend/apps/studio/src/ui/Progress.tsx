import { ProgressBar } from '@astryxdesign/core/ProgressBar';

/**
 * How far a piece of work has gone: a bar named by `label`, with `value` out of `max` as the value a
 * screen reader announces. The words that say it (for example "214 of 214 runs") stay beside it, as
 * text: the bar is never the only way to read the progress.
 */
export function Progress({ label, value, max }: { label: string; value: number; max: number }) {
  return <ProgressBar label={label} isLabelHidden value={value} max={Math.max(max, 1)} />;
}
