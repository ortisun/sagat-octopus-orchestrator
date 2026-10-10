import type { StatusView } from '../model/status';

export function StatusBadge({ view }: { view: StatusView }) {
  return (
    <span className="badge" data-tone={view.tone} title={view.help}>
      <span aria-hidden="true">{view.symbol}</span>
      <span>{view.label}</span>
    </span>
  );
}
