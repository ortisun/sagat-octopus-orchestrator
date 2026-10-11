import { Handle, Position } from '@xyflow/react';
import type { NodeProps } from '@xyflow/react';
import { Link } from 'react-router';
import type { RuntimeNode } from '../../api/types';
import { path } from '../../api/client';
import { nodeStatus } from '../../model/status';
import { StatusBadge } from '../../ui/StatusBadge';
import type { CardNode } from './canvas';
import { attemptWords, kindView, waitingWords } from './display';
import { Relative } from './Now';
import { clip } from './time';

const PHASES: Record<string, string> = { FORWARD: 'going forward', ABORTING: 'aborting', COMPENSATING: 'compensating', DONE: 'done' };
const OUTCOMES: Record<string, string> = { COMPENSATED: 'compensated', PARTIAL: 'partly undone', COMPENSATION_FAILED: 'compensation failed' };

export function sagaWords(node: RuntimeNode): string | undefined {
  const saga = node.saga;
  if (saga === null || saga === undefined) return undefined;
  const parts = [`saga ${PHASES[saga.phase] ?? clip(String(saga.phase), 20)}`];
  if (saga.outcome) parts.push(OUTCOMES[saga.outcome] ?? clip(String(saga.outcome), 24));
  if (saga.irreversible.length > 0) parts.push(`${saga.irreversible.length} cannot be undone`);
  return parts.join(', ');
}

/** The id of a child run is a link to it; the path is built by the router's own encoding, never by hand. */
export function childPath(id: string): string {
  return path`/runs/${id}`;
}

function Handles() {
  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} className="rn-handle" />
      <Handle type="source" position={Position.Right} isConnectable={false} className="rn-handle" />
    </>
  );
}

/**
 * A step of the run on the canvas: what it is, how it stands, and what a person has to look at.
 * Undo steps and lookups are dashed and say what they are about. Highlighting a step an update
 * touched is an overlay that restarts with each update and is switched off by `prefers-reduced-motion`.
 */
export function RunCard({ data }: NodeProps<CardNode>) {
  const { node, flash } = data;
  const kind = kindView(node);
  const attempt = attemptWords(node);
  const waiting = waitingWords(node);
  const saga = sagaWords(node);
  return (
    <div className="rn-card" data-status={node.status} data-variant={kind.variant}>
      {flash > 0 ? <span key={flash} className="rn-flash" aria-hidden="true" /> : null}
      <Handles />
      <strong className="rn-id" title={node.id}>
        {node.id}
      </strong>
      <span className="rn-row">
        <span className="rn-kind">{kind.label}</span>
        <StatusBadge view={nodeStatus(node.status)} />
      </span>
      <span className="rn-notes">
        {attempt ? <span className="rn-chip">{attempt}</span> : null}
        {node.failureCode ? <span className="rn-chip bad">failure {clip(node.failureCode, 32)}</span> : null}
        {waiting ? <span className={node.waitingOn === 'operator' ? 'rn-chip bad' : 'rn-chip'}>{waiting}</span> : null}
        {node.nextAttemptAt ? (
          <span className="rn-chip">
            retry <Relative at={node.nextAttemptAt} />
          </span>
        ) : null}
        {node.dueAt ? (
          <span className="rn-chip">
            due <Relative at={node.dueAt} />
          </span>
        ) : null}
        {saga ? <span className="rn-chip">{saga}</span> : null}
        {node.childRunId ? (
          <Link className="rn-chip nopan" to={childPath(node.childRunId)} aria-label={`Open the child run of ${clip(node.id, 40)}`}>
            child run
          </Link>
        ) : null}
      </span>
    </div>
  );
}

/** The simplified step of a big graph: its id and its status, nothing that costs. */
export function RunChip({ data }: NodeProps<CardNode>) {
  const { node, flash } = data;
  const view = nodeStatus(node.status);
  return (
    <div className="rn-chipnode" data-status={node.status} data-variant={kindView(node).variant}>
      {flash > 0 ? <span key={flash} className="rn-flash" aria-hidden="true" /> : null}
      <Handles />
      <span aria-hidden="true">{view.symbol}</span>
      <strong className="rn-id" title={node.id}>
        {node.id}
      </strong>
      <span className="rn-word">{view.label}</span>
    </div>
  );
}

/** Defined once, outside any component: React Flow redraws every node when this object changes. */
export const NODE_TYPES = { 'run-card': RunCard, 'run-chip': RunChip };
