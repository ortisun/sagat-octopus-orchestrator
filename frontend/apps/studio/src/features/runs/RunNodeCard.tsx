import { Handle, Position } from '@xyflow/react';
import type { NodeProps } from '@xyflow/react';
import type { RuntimeNode } from '../../api/types';
import { path } from '../../api/client';
import { nodeStatus } from '../../model/status';
import { StatusBadge } from '../../ui/StatusBadge';
import type { CardNode } from './canvas';
import { kindView } from './display';
import type { KindView } from './display';
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

const CARD_ID_LIMIT = 80;
const FAILURE_CODE_LIMIT = 32;

function cardIdentity(id: string): { text: string; full?: string } {
  if (id.length <= CARD_ID_LIMIT) return { text: id };
  return { text: 'Long step ID — select for details', full: id };
}

function cardKind(node: RuntimeNode, kind: KindView): { text: string; full?: string } {
  if (kind.variant === 'forward') {
    return node.type.length > 40 && kind.label.endsWith('…') ? { text: 'Type details available', full: node.type } : { text: kind.label };
  }
  const suffix = kind.variant === 'undo' ? /~c$/ : /~r$/;
  const forward = node.forward ?? node.id.replace(suffix, '');
  if (forward.length <= CARD_ID_LIMIT) return { text: kind.label };
  return kind.variant === 'undo'
    ? { text: 'Undo step — select for details', full: `Undo of ${forward}` }
    : { text: 'Lookup step — select for details', full: `Lookup for ${forward}` };
}

function attentionSummary(node: RuntimeNode): string | undefined {
  if (node.waitingOn === 'operator') return 'Needs a person';
  if (node.failureCode === undefined || node.failureCode === null || node.failureCode === '') return undefined;
  return node.failureCode.length <= FAILURE_CODE_LIMIT ? `Failure: ${node.failureCode}` : 'Failure details available';
}

/**
 * A step of the run on the canvas: what it is, how it stands, and what a person has to look at.
 * Undo steps and lookups are dashed and say what they are about. Highlighting a step an update
 * touched is an overlay that restarts with each update and is switched off by `prefers-reduced-motion`.
 */
export function RunCard({ data }: NodeProps<CardNode>) {
  const { node, flash } = data;
  const kind = kindView(node);
  const identity = cardIdentity(node.id);
  const kindLabel = cardKind(node, kind);
  const attention = attentionSummary(node);
  return (
    <div className="rn-card" data-status={node.status} data-variant={kind.variant}>
      {flash > 0 ? <span key={flash} className="rn-flash" aria-hidden="true" /> : null}
      <Handles />
      <div className="rn-card-identity">
        <span className="rn-card-label">Step ID</span>
        <strong className="rn-id" aria-label={identity.full}>
          {identity.text}
        </strong>
      </div>
      <div className="rn-card-kind">
        <span className="rn-card-label">Type</span>
        <span className="rn-kind" aria-label={kindLabel.full}>
          {kindLabel.text}
        </span>
      </div>
      <div className="rn-card-status">
        <span className="rn-card-label">State</span>
        <StatusBadge view={nodeStatus(node.status)} />
      </div>
      {attention ? <div className="rn-attention">{attention}</div> : null}
    </div>
  );
}

/** The simplified step of a big graph: its id and its status, nothing that costs. */
export function RunChip({ data }: NodeProps<CardNode>) {
  const { node, flash } = data;
  const view = nodeStatus(node.status);
  const identity = cardIdentity(node.id);
  return (
    <div className="rn-chipnode" data-status={node.status} data-variant={kindView(node).variant}>
      {flash > 0 ? <span key={flash} className="rn-flash" aria-hidden="true" /> : null}
      <Handles />
      <strong className="rn-id" aria-label={identity.full}>
        {identity.text}
      </strong>
      <span className="rn-card-status">
        <span aria-hidden="true">{view.symbol}</span>
        <span className="rn-word">{view.label}</span>
      </span>
    </div>
  );
}

/** Defined once, outside any component: React Flow redraws every node when this object changes. */
export const NODE_TYPES = { 'run-card': RunCard, 'run-chip': RunChip };
