import { useMemo, useRef, useState } from 'react';
import type { RuntimeNode } from '../../api/types';
import type { RunModel } from '../../model/runGraph';
import { nodeStatus } from '../../model/status';
import { Button, Checkbox, Code, ColumnHeader, Empty, Hint, WindowedTable, Row, Stack, StatusBadge, TableBody, TableCell, TableHead, TableRow, TextInput } from '../../ui';
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
    <Stack gap={3}>
      <Row gap={3} align="end">
        <TextInput type="search" label="Find a step" width="16rem" value={query.text} autoComplete="off" spellCheck={false} onChange={(text) => setQuery((q) => ({ ...q, text }))} />
        <Checkbox label={`Only what needs attention (${attention})`} checked={query.attentionOnly} onChange={(attentionOnly) => setQuery((q) => ({ ...q, attentionOnly }))} />
        <Hint role="status">{rows.length === model.nodes.size ? `${rows.length} steps` : `${rows.length} of ${model.nodes.size} steps`}</Hint>
      </Row>
      {rows.length === 0 ? (
        <Empty title={model.nodes.size === 0 ? 'This run has no steps.' : query.attentionOnly && query.text === '' ? 'Nothing needs attention right now.' : 'No step matches.'} />
      ) : (
        <ScrollRegion label="Steps of the run" scroller={scroller}>
          <WindowedTable className="rn-fixed rn-steps" label="Steps of the run" rowCount={rows.length + 1}>
            <TableHead>
              <TableRow aria-rowindex={1}>
                <ColumnHeader scope="col" aria-sort={aria('id')}>
                  <Button variant="ghost" size="sm" onClick={() => sortBy('id')}>
                    Step{arrow('id')}
                  </Button>
                </ColumnHeader>
                <ColumnHeader scope="col">Kind</ColumnHeader>
                <ColumnHeader scope="col" aria-sort={aria('status')}>
                  <Button variant="ghost" size="sm" onClick={() => sortBy('status')}>
                    Status{arrow('status')}
                  </Button>
                </ColumnHeader>
                <ColumnHeader scope="col">Attempt</ColumnHeader>
                <ColumnHeader scope="col">Failure</ColumnHeader>
                <ColumnHeader scope="col">Waiting on</ColumnHeader>
                <ColumnHeader scope="col">Next attempt / due</ColumnHeader>
                <ColumnHeader scope="col">Actions</ColumnHeader>
              </TableRow>
            </TableHead>
            <TableBody>
              {padTop > 0 ? (
                <TableRow aria-hidden="true" className="rn-pad">
                  <TableCell colSpan={COLUMNS} style={{ height: padTop }} />
                </TableRow>
              ) : null}
              {items.map((item) => {
                const node = rows[item.index];
                if (node === undefined) return null;
                return <StepRow key={node.id} index={item.index} measure={measure} node={node} selected={node.id === selectedId} onSelect={onSelect} operations={operationsFor(node)} onChoose={onChoose} />;
              })}
              {padBottom > 0 ? (
                <TableRow aria-hidden="true" className="rn-pad">
                  <TableCell colSpan={COLUMNS} style={{ height: padBottom }} />
                </TableRow>
              ) : null}
            </TableBody>
          </WindowedTable>
        </ScrollRegion>
      )}
    </Stack>
  );
}

function StepRow({
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
    <TableRow ref={measure} data-index={index} aria-rowindex={index + 2} data-status={node.status} data-selected={selected ? 'true' : undefined}>
      <TableCell>
        <Button variant="ghost" size="sm" className="rn-id" aria-current={selected ? 'true' : undefined} {...(node.id.length > 60 ? { title: node.id } : {})} onClick={() => onSelect(node.id)}>
          {clip(node.id, 60)}
        </Button>
      </TableCell>
      <TableCell>{kind.label}</TableCell>
      <TableCell>
        <StatusBadge view={nodeStatus(node.status)} />
      </TableCell>
      <TableCell>{node.attempt && node.attempt > 0 ? node.attempt : '—'}</TableCell>
      <TableCell>{node.failureCode ? <Code>{clip(node.failureCode, 40)}</Code> : '—'}</TableCell>
      <TableCell>{waiting ?? '—'}</TableCell>
      <TableCell>
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
      </TableCell>
      <TableCell>
        <ActionsMenu inline label={`Actions for ${clip(node.id, 60)}`} buttonText="Actions" operations={operations} onChoose={(operation, opener) => onChoose(operation, node, opener)} />
      </TableCell>
    </TableRow>
  );
}
