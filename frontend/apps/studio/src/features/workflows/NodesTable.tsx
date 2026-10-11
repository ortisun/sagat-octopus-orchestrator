import { useVirtualizer } from '@tanstack/react-virtual';
import { useId, useRef, useState } from 'react';
import type { IrDocument, IrNode } from '../../model/ir';
import { kindOf, linksOf } from '../../model/registry';
import { Button } from '../../ui/Button';
import { clip, keyField } from './canvasModel';

/** Above this many rows only the rows in view are in the page (D147). */
export const VIRTUAL_ABOVE = 200;
const ROW_HEIGHT = 72;

export interface NodesTableProps {
  doc: IrDocument;
  selectedId: string | undefined;
  problems: ReadonlyMap<string, number>;
  advice: ReadonlyMap<string, number>;
  /** Edit, delete, connect and set-as-first controls; a read-only document has none. */
  editable: boolean;
  /** The edit button opens the inspector; a table that only displays has none either. */
  inspectable?: boolean;
  caption: string;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
  onSetFirst: (id: string) => void;
  onConnect: (id: string) => void;
}

function leadsTo(node: IrNode): string {
  const links = linksOf(node);
  if (links.length === 0) return kindOf(node.type)?.handles.length === 0 ? 'Ends the run' : '—';
  const handles = kindOf(node.type)?.handles ?? [];
  return links
    .map((l) => {
      const label = handles.find((h) => h.field === l.field)?.label ?? l.field;
      return `${label}${l.index === undefined ? '' : ` ${l.index + 1}`} → ${clip(l.to, 40)}`;
    })
    .join('; ');
}

function Row({
  node,
  index,
  doc,
  selected,
  problems,
  advice,
  props,
  measure,
}: {
  node: IrNode;
  index: number | undefined;
  doc: IrDocument;
  selected: boolean;
  problems: number;
  advice: number;
  props: NodesTableProps;
  measure?: (element: HTMLTableRowElement | null) => void;
}) {
  const spec = kindOf(node.type);
  const first = doc.entrypoint === node.id;
  const id = clip(node.id, 64);
  return (
    <tr
      ref={measure}
      data-index={index}
      aria-current={selected ? 'true' : undefined}
      aria-rowindex={index === undefined ? undefined : index + 2}
      data-selected={selected ? 'true' : undefined}
      data-testid={`row-${node.id}`}
    >
      <th scope="row" className="wf-cell-id">
        <code>{id}</code>
        {first ? (
          <span className="wf-badge" data-tone="info">
            {' '}
            <span aria-hidden="true">▶</span> First step
          </span>
        ) : null}
      </th>
      <td>{spec?.label ?? clip(node.type, 24)}</td>
      <td className="wf-cell-key">{keyField(node)}</td>
      <td className="wf-cell-links">{leadsTo(node)}</td>
      <td>
        {problems > 0 ? (
          <span className="wf-badge" data-tone="bad">
            <span aria-hidden="true">✕</span> {problems} {problems === 1 ? 'problem' : 'problems'}
          </span>
        ) : null}
        {advice > 0 ? (
          <span className="wf-badge" data-tone="warn">
            <span aria-hidden="true">!</span> {advice} advice
          </span>
        ) : null}
        {spec && !spec.runs ? (
          <span className="wf-badge" data-tone="warn">
            <span aria-hidden="true">⊘</span> Not run yet
          </span>
        ) : null}
        {problems === 0 && advice === 0 && (spec?.runs ?? true) ? <span className="hint">None</span> : null}
      </td>
      <td>
        <div className="wf-cell-actions">
          {props.inspectable !== false ? (
            <Button aria-label={`${props.editable ? 'Edit' : 'Open'} ${id}`} onClick={() => props.onEdit(node.id)}>
              {props.editable ? 'Edit' : 'Open'}
            </Button>
          ) : null}
          {props.editable ? (
            <>
              <Button aria-label={`Connect ${id}…`} onClick={() => props.onConnect(node.id)} disabled={(spec?.handles ?? []).length === 0}>
                Connect…
              </Button>
              <Button aria-label={`Set ${id} as first step`} onClick={() => props.onSetFirst(node.id)} disabled={first}>
                Set as first step
              </Button>
              <Button variant="danger" aria-label={`Delete ${id}`} onClick={() => props.onDelete(node.id)}>
                Delete
              </Button>
            </>
          ) : null}
        </div>
      </td>
    </tr>
  );
}

/**
 * Every node of the workflow as a table (D148): the same data and the same actions as the canvas,
 * reachable by keyboard and by screen reader. Above 200 rows only those in view are in the page.
 */
export function NodesTable(props: NodesTableProps) {
  const { doc, selectedId, problems, advice } = props;
  const [filter, setFilter] = useState('');
  const filterId = useId();
  const scroller = useRef<HTMLDivElement>(null);
  const needle = filter.trim().toLowerCase();
  const rows =
    needle === ''
      ? doc.nodes
      : doc.nodes.filter((n) => n.id.toLowerCase().includes(needle) || n.type.toLowerCase().includes(needle) || (kindOf(n.type)?.label.toLowerCase().includes(needle) ?? false));
  const virtual = rows.length > VIRTUAL_ABOVE;
  // The Studio is not built with the React Compiler, which is what this warning is about.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({ count: rows.length, getScrollElement: () => scroller.current, estimateSize: () => ROW_HEIGHT, overscan: 12, enabled: virtual });
  const items = virtual ? virtualizer.getVirtualItems() : [];
  const padTop = items[0]?.start ?? 0;
  const padBottom = virtual ? virtualizer.getTotalSize() - (items[items.length - 1]?.end ?? 0) : 0;

  return (
    <div className="wf-table-wrap">
      <div className="wf-table-tools">
        <label htmlFor={filterId}>Filter nodes</label>
        <input id={filterId} type="search" value={filter} placeholder="Id or type" autoComplete="off" onChange={(event) => setFilter(event.target.value)} />
        <span className="hint" data-testid="row-count">
          {rows.length === doc.nodes.length ? `${doc.nodes.length} ${doc.nodes.length === 1 ? 'node' : 'nodes'}` : `${rows.length} of ${doc.nodes.length} nodes`}
        </span>
      </div>
      {doc.nodes.length === 0 ? (
        <p className="empty">This workflow has no nodes yet. Use Add node to start.</p>
      ) : (
        <div
          ref={scroller}
          className="wf-table-scroll"
          data-virtual={virtual ? 'true' : undefined}
          tabIndex={virtual ? 0 : undefined}
          role={virtual ? 'region' : undefined}
          aria-label={virtual ? 'Nodes (scrolls)' : undefined}
        >
          <table className="data wf-table" aria-rowcount={virtual ? rows.length + 1 : undefined}>
            <caption className="visually-hidden">{props.caption}</caption>
            <thead>
              <tr aria-rowindex={virtual ? 1 : undefined}>
                <th scope="col">Id</th>
                <th scope="col">Type</th>
                <th scope="col">Key field</th>
                <th scope="col">Leads to</th>
                <th scope="col">Checks</th>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {virtual && padTop > 0 ? (
                <tr aria-hidden="true" style={{ height: padTop }}>
                  <td colSpan={6} />
                </tr>
              ) : null}
              {virtual
                ? items.map((item) => {
                    const node = rows[item.index];
                    return node ? (
                      <Row
                        key={node.id}
                        node={node}
                        index={item.index}
                        doc={doc}
                        selected={node.id === selectedId}
                        problems={problems.get(node.id) ?? 0}
                        advice={advice.get(node.id) ?? 0}
                        props={props}
                        measure={virtualizer.measureElement}
                      />
                    ) : null;
                  })
                : rows.map((node) => (
                    <Row
                      key={node.id}
                      node={node}
                      index={undefined}
                      doc={doc}
                      selected={node.id === selectedId}
                      problems={problems.get(node.id) ?? 0}
                      advice={advice.get(node.id) ?? 0}
                      props={props}
                    />
                  ))}
              {virtual && padBottom > 0 ? (
                <tr aria-hidden="true" style={{ height: padBottom }}>
                  <td colSpan={6} />
                </tr>
              ) : null}
            </tbody>
          </table>
          {rows.length === 0 ? <p className="empty">No node matches &quot;{clip(filter, 40)}&quot;.</p> : null}
        </div>
      )}
    </div>
  );
}
