import { useMemo, useRef, useState } from 'react';
import type { RuntimeNode } from '../../api/types';
import type { RunModel } from '../../model/runGraph';
import { nodeStatus } from '../../model/status';
import { StatusBadge } from '../../ui/StatusBadge';
import { ScrollRegion } from './ScrollRegion';
import { isAttention, kindView, waitingWords } from './display';
import { Relative } from './Now';
import { ActionsMenu } from './Operations';
import type { Operation } from './ops';
import { DEFAULT_QUERY, tableNodes } from './table';
import type { SortKey, TableQuery } from './table';
import { clip } from './time';
import { useWindowedRows } from './useWindow';

const ROW_ESTIMATE = 52;
const COLUMNS = 8;

/**
 * Every step of the run as a table: the same data as the graph and the same actions, reachable by
 * keyboard, sortable by status or id, filterable to what needs attention. Only the rows in view
 * are drawn, so 50 000 steps cost what 30 do.
 */
export function TableView({
  model,
  order,
  selectedId,
  onSelect,
  operationsFor,
  onChoose,
}: {
  model: RunModel;
  order: readonly string[];
  selectedId: string | undefined;
  onSelect: (id: string) => void;
  operationsFor: (node: RuntimeNode) => readonly Operation[];
  onChoose: (operation: Operation, node: RuntimeNode, opener: HTMLElement | null) => void;
}) {
  const [query, setQuery] = useState<TableQuery>(DEFAULT_QUERY);
  const scroller = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => tableNodes(model.nodes, order, query), [model.nodes, order, query]);
  const attention = useMemo(() => {
    let n = 0;
    for (const node of model.nodes.values()) if (isAttention(node)) n += 1;
    return n;
  }, [model.nodes]);
  const { items, padTop, padBottom, measure } = useWindowedRows({ count: rows.length, estimate: ROW_ESTIMATE, scroller });

  const sortBy = (key: SortKey) => setQuery((q) => (q.sort === key ? { ...q, descending: !q.descending } : { ...q, sort: key, descending: false }));
  const aria = (key: SortKey): 'ascending' | 'descending' | 'none' => (query.sort !== key ? 'none' : query.descending ? 'descending' : 'ascending');
  const arrow = (key: SortKey) => (query.sort !== key ? '' : query.descending ? ' ▾' : ' ▴');

  return (
    <div className="rn-table">
      <div className="toolbar">
        <label className="rn-check">
          <input type="checkbox" checked={query.attentionOnly} onChange={(event) => setQuery((q) => ({ ...q, attentionOnly: event.target.checked }))} /> Only what needs attention ({attention})
        </label>
        <label className="rn-find">
          <span>Find a step</span>
          <input type="search" className="input" value={query.text} autoComplete="off" spellCheck={false} onChange={(event) => setQuery((q) => ({ ...q, text: event.target.value }))} />
        </label>
        <span className="hint" role="status">
          {rows.length === model.nodes.size ? `${rows.length} steps` : `${rows.length} of ${model.nodes.size} steps`}
        </span>
      </div>
      {rows.length === 0 ? (
        <p className="empty">{model.nodes.size === 0 ? 'This run has no steps.' : query.attentionOnly && query.text === '' ? 'Nothing needs attention right now.' : 'No step matches.'}</p>
      ) : (
        <ScrollRegion label="Steps of the run" scroller={scroller}>
          <table className="data rn-fixed rn-steps" aria-rowcount={rows.length + 1}>
            <caption className="visually-hidden">Steps of the run</caption>
            <thead>
              <tr aria-rowindex={1}>
                <th scope="col" aria-sort={aria('id')}>
                  <button type="button" className="link" onClick={() => sortBy('id')}>
                    Step{arrow('id')}
                  </button>
                </th>
                <th scope="col">Kind</th>
                <th scope="col" aria-sort={aria('status')}>
                  <button type="button" className="link" onClick={() => sortBy('status')}>
                    Status{arrow('status')}
                  </button>
                </th>
                <th scope="col">Attempt</th>
                <th scope="col">Failure</th>
                <th scope="col">Waiting on</th>
                <th scope="col">Next attempt / due</th>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {padTop > 0 ? (
                <tr aria-hidden="true" className="rn-pad">
                  <td colSpan={COLUMNS} style={{ height: padTop }} />
                </tr>
              ) : null}
              {items.map((item) => {
                const node = rows[item.index];
                if (node === undefined) return null;
                return <Row key={node.id} index={item.index} measure={measure} node={node} selected={node.id === selectedId} onSelect={onSelect} operations={operationsFor(node)} onChoose={onChoose} />;
              })}
              {padBottom > 0 ? (
                <tr aria-hidden="true" className="rn-pad">
                  <td colSpan={COLUMNS} style={{ height: padBottom }} />
                </tr>
              ) : null}
            </tbody>
          </table>
        </ScrollRegion>
      )}
    </div>
  );
}

function Row({
  index,
  measure,
  node,
  selected,
  onSelect,
  operations,
  onChoose,
}: {
  index: number;
  measure: (element: Element | null) => void;
  node: RuntimeNode;
  selected: boolean;
  onSelect: (id: string) => void;
  operations: readonly Operation[];
  onChoose: (operation: Operation, node: RuntimeNode, opener: HTMLElement | null) => void;
}) {
  const kind = kindView(node);
  const waiting = waitingWords(node);
  return (
    <tr ref={measure} data-index={index} aria-rowindex={index + 2} data-status={node.status} data-selected={selected ? 'true' : undefined}>
      <td>
        <button type="button" className="link rn-id" aria-current={selected ? 'true' : undefined} title={node.id} onClick={() => onSelect(node.id)}>
          {clip(node.id, 60)}
        </button>
      </td>
      <td>{kind.label}</td>
      <td>
        <StatusBadge view={nodeStatus(node.status)} />
      </td>
      <td>{node.attempt && node.attempt > 0 ? node.attempt : '—'}</td>
      <td>{node.failureCode ? <code>{clip(node.failureCode, 40)}</code> : '—'}</td>
      <td>{waiting ?? '—'}</td>
      <td>
        {node.nextAttemptAt ? (
          <span>
            retry <Relative at={node.nextAttemptAt} />
          </span>
        ) : null}
        {node.dueAt ? (
          <span>
            due <Relative at={node.dueAt} />
          </span>
        ) : null}
        {!node.nextAttemptAt && !node.dueAt ? '—' : null}
      </td>
      <td>
        <ActionsMenu inline label={`Actions for ${clip(node.id, 60)}`} buttonText="Actions" operations={operations} onChoose={(operation, opener) => onChoose(operation, node, opener)} />
      </td>
    </tr>
  );
}
