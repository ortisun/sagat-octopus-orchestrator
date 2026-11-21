import { useVirtualizer } from '@tanstack/react-virtual';
import { useRef, useState } from 'react';
import type { IrDocument, IrNode } from '../../model/ir';
import { kindOf, linksOf } from '../../model/registry';
import { Badge, Button, Code, ColumnHeader, Empty, TableBody, TableCell, TableHead, TableRow, TextInput, WindowedTable } from '../../ui';
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
    <TableRow
      ref={measure}
      data-index={index}
      aria-current={selected ? 'true' : undefined}
      aria-rowindex={index === undefined ? undefined : index + 2}
      data-selected={selected ? 'true' : undefined}
      data-testid={`row-${node.id}`}
    >
      <ColumnHeader scope="row">
        <span className="wf-cell-id">
          <Code>{id}</Code>{' '}
          {first ? (
            <Badge tone="running">
              <span aria-hidden="true">▶</span> First step
            </Badge>
          ) : null}
        </span>
      </ColumnHeader>
      <TableCell>{spec?.label ?? clip(node.type, 24)}</TableCell>
      <TableCell>{keyField(node)}</TableCell>
      <TableCell>{leadsTo(node)}</TableCell>
      <TableCell>
        <span className="wf-cell-checks">
          {problems > 0 ? (
            <Badge tone="bad">
              <span aria-hidden="true">✕</span> {problems} {problems === 1 ? 'problem' : 'problems'}
            </Badge>
          ) : null}
          {advice > 0 ? (
            <Badge tone="waiting">
              <span aria-hidden="true">!</span> {advice} advice
            </Badge>
          ) : null}
          {spec && !spec.runs ? (
            <Badge tone="waiting">
              <span aria-hidden="true">⊘</span> Not run yet
            </Badge>
          ) : null}
          {problems === 0 && advice === 0 && (spec?.runs ?? true) ? <span className="wf-quiet">None</span> : null}
        </span>
      </TableCell>
      <TableCell>
        <div className="wf-cell-actions">
          {props.inspectable !== false ? (
            <Button size="sm" aria-label={`${props.editable ? 'Edit' : 'Open'} ${id}`} onClick={() => props.onEdit(node.id)}>
              {props.editable ? 'Edit' : 'Open'}
            </Button>
          ) : null}
          {props.editable ? (
            <>
              <Button size="sm" aria-label={`Connect ${id}…`} onClick={() => props.onConnect(node.id)} disabled={(spec?.handles ?? []).length === 0}>
                Connect…
              </Button>
              <Button size="sm" aria-label={`Set ${id} as first step`} onClick={() => props.onSetFirst(node.id)} disabled={first}>
                Set as first step
              </Button>
              <Button size="sm" variant="danger" aria-label={`Delete ${id}`} onClick={() => props.onDelete(node.id)}>
                Delete
              </Button>
            </>
          ) : null}
        </div>
      </TableCell>
    </TableRow>
  );
}

/**
 * Every node of the workflow as a table (D148): the same data and the same actions as the canvas,
 * reachable by keyboard and by screen reader. Above 200 rows only those in view are in the page.
 */
export function NodesTable(props: NodesTableProps) {
  const { doc, selectedId, problems, advice } = props;
  const [filter, setFilter] = useState('');
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
    <div>
      <div className="wf-table-tools">
        <TextInput label="Filter nodes" type="search" value={filter} placeholder="Id or type" autoComplete="off" width="16rem" onChange={setFilter} />
        <span className="wf-quiet" data-testid="row-count">
          {rows.length === doc.nodes.length ? `${doc.nodes.length} ${doc.nodes.length === 1 ? 'node' : 'nodes'}` : `${rows.length} of ${doc.nodes.length} nodes`}
        </span>
      </div>
      {doc.nodes.length === 0 ? (
        <Empty title="This workflow has no nodes yet. Use Add node to start." />
      ) : (
        <div
          ref={scroller}
          className="wf-table-scroll"
          data-virtual={virtual ? 'true' : undefined}
          tabIndex={virtual ? 0 : undefined}
          role={virtual ? 'region' : undefined}
          aria-label={virtual ? 'Nodes (scrolls)' : undefined}
        >
          <WindowedTable label={props.caption} className="wf-table" rowCount={virtual ? rows.length + 1 : undefined}>
            <TableHead>
              <TableRow aria-rowindex={virtual ? 1 : undefined}>
                <ColumnHeader>Id</ColumnHeader>
                <ColumnHeader>Type</ColumnHeader>
                <ColumnHeader>Key field</ColumnHeader>
                <ColumnHeader>Leads to</ColumnHeader>
                <ColumnHeader>Checks</ColumnHeader>
                <ColumnHeader>Actions</ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {virtual && padTop > 0 ? (
                <TableRow aria-hidden="true" style={{ height: padTop }}>
                  <TableCell colSpan={6} />
                </TableRow>
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
                <TableRow aria-hidden="true" style={{ height: padBottom }}>
                  <TableCell colSpan={6} />
                </TableRow>
              ) : null}
            </TableBody>
          </WindowedTable>
          {rows.length === 0 ? <Empty title={`No node matches "${clip(filter, 40)}".`} /> : null}
        </div>
      )}
    </div>
  );
}
