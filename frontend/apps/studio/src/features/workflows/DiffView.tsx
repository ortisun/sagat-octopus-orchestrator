import type { DefinitionDiff, DiffFinding } from '../../api/types';
import { clip } from './canvasModel';

export type DiffLike = Pick<DefinitionDiff, 'identical' | 'nodes' | 'edges' | 'changes'> & Partial<Pick<DefinitionDiff, 'breaking' | 'attention' | 'from' | 'to' | 'changesOmitted'>>;

/** What is drawn of a list: the platform bounds what it sends, and a page of ten thousand lines helps nobody. */
const SHOWN = 200;

function More({ total, of = SHOWN }: { total: number; of?: number }) {
  return total > of ? (
    <li className="hint">
      and {total - of} more, not shown here
    </li>
  ) : null;
}

function side(s: DefinitionDiff['from'] | undefined): string {
  if (!s) return '?';
  if (s.kind === 'version') return `version ${s.number ?? '?'}`;
  if (s.kind === 'draft') return s.revision === null || s.revision === undefined ? 'the draft' : `the draft (revision ${s.revision})`;
  return 'nothing (no version yet)';
}

function Findings({ items }: { items: readonly DiffFinding[] }) {
  return (
    <ul>
      {items.slice(0, SHOWN).map((f, i) => (
        <li key={`${f.code}-${i}`}>
          {f.nodeId ? (
            <>
              <code>{clip(f.nodeId, 64)}</code>:{' '}
            </>
          ) : null}
          {clip(f.message, 300)} <span className="hint">({f.code})</span>
        </li>
      ))}
      <More total={items.length} />
    </ul>
  );
}

function Ids({ ids }: { ids: readonly string[] }) {
  return (
    <>
      {ids.slice(0, SHOWN).map((id, i) => (
        <span key={id}>
          {i > 0 ? ', ' : ''}
          <code>{clip(id, 64)}</code>
        </span>
      ))}
      {ids.length > SHOWN ? <span className="hint"> and {ids.length - SHOWN} more</span> : null}
    </>
  );
}

/**
 * The difference between two documents. With `classified`, the platform has judged it: breaking
 * changes (callers or event publishers can fail) and changes that need preparation are set apart in
 * blocks that are told by their words and a shape, not by colour. Without it (the one comparison the
 * Studio makes itself) no such judgement is shown, and the view says so.
 */
export function DiffView({ diff, classified, labels }: { diff: DiffLike; classified: boolean; labels?: { from: string; to: string } }) {
  const breaking = diff.breaking ?? [];
  const attention = diff.attention ?? [];
  const links = diff.edges.added.length + diff.edges.removed.length;
  const nodes = diff.nodes.added.length + diff.nodes.removed.length + diff.nodes.changed.length;
  return (
    <div className="wf-diff">
      <p>
        Comparing <strong>{labels?.from ?? side(diff.from)}</strong> with <strong>{labels?.to ?? side(diff.to)}</strong>.
      </p>
      {diff.identical ? <p>There are no differences.</p> : null}

      {classified && breaking.length > 0 ? (
        <section className="wf-breaking" aria-labelledby="wf-diff-breaking">
          <h3 id="wf-diff-breaking">
            <span aria-hidden="true">⚠</span> Breaking changes ({breaking.length})
          </h3>
          <p>These changes can make callers or event publishers fail. Runs already started keep the version they started on.</p>
          <Findings items={breaking} />
        </section>
      ) : null}
      {classified && attention.length > 0 ? (
        <section className="wf-attention" aria-labelledby="wf-diff-attention">
          <h3 id="wf-diff-attention">
            <span aria-hidden="true">!</span> Needs preparation ({attention.length})
          </h3>
          <p>These do not break the contract, but somebody has to act before new runs can use them.</p>
          <Findings items={attention} />
        </section>
      ) : null}
      {classified && breaking.length === 0 && !diff.identical ? <p>The platform found no breaking changes.</p> : null}
      {!classified ? <p className="hint">This comparison lists what differs. Whether a change breaks callers is judged by the platform when the draft is published.</p> : null}

      {nodes > 0 ? (
        <section aria-label="Node changes">
          <h3>Nodes</h3>
          <ul>
            {diff.nodes.added.length > 0 ? (
              <li>
                Added ({diff.nodes.added.length}): <Ids ids={diff.nodes.added} />
              </li>
            ) : null}
            {diff.nodes.removed.length > 0 ? (
              <li>
                Removed ({diff.nodes.removed.length}): <Ids ids={diff.nodes.removed} />
              </li>
            ) : null}
            {diff.nodes.changed.slice(0, SHOWN).map((c) => (
              <li key={c.id}>
                Changed: <code>{clip(c.id, 64)}</code> (<span>{c.fields.map((f) => clip(f, 40)).join(', ')}</span>)
              </li>
            ))}
            <More total={diff.nodes.changed.length} />
          </ul>
        </section>
      ) : null}
      {links > 0 ? (
        <section aria-label="Link changes">
          <h3>Links</h3>
          <ul>
            {diff.edges.added.slice(0, SHOWN).map((e) => (
              <li key={`a-${e.from}-${e.kind}-${e.to}`}>
                Added: <code>{clip(e.from, 64)}</code> → <code>{clip(e.to, 64)}</code> ({clip(e.kind, 20)})
              </li>
            ))}
            <More total={diff.edges.added.length} />
            {diff.edges.removed.slice(0, SHOWN).map((e) => (
              <li key={`r-${e.from}-${e.kind}-${e.to}`}>
                Removed: <code>{clip(e.from, 64)}</code> → <code>{clip(e.to, 64)}</code> ({clip(e.kind, 20)})
              </li>
            ))}
            <More total={diff.edges.removed.length} />
          </ul>
        </section>
      ) : null}
      {diff.changes.length > 0 ? (
        <section aria-label="Other changes">
          <h3>Other changes</h3>
          <ul>
            {diff.changes.slice(0, SHOWN).map((c) => (
              <li key={c.path}>
                <code>{clip(c.path, 80)}</code> {c.change}
              </li>
            ))}
            <More total={diff.changes.length} />
            {(diff.changesOmitted ?? 0) > 0 ? <li className="hint">and {diff.changesOmitted} more differences the platform did not list</li> : null}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
