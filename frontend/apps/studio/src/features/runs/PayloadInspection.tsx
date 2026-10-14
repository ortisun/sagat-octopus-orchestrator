import { useEffect, useId, useRef, useState } from 'react';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import type { SessionSnapshot } from '../../auth/session';
import { Button } from '../../ui/Button';
import { ErrorState } from '../../ui/ErrorState';

export type PayloadSelector = components['schemas']['PayloadInspectionSelector'];
type Structure = components['schemas']['PayloadStructure'];
export type PayloadInspection = components['schemas']['PayloadInspection'];

type LoadState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'loaded'; inspection: PayloadInspection }
  | { phase: 'error'; error: unknown };

interface ScopedState {
  scope: string;
  session: SessionSnapshot;
  state: LoadState;
}

/**
 * A lazy, ephemeral view of one server-redacted projection. It deliberately uses component state
 * rather than the query cache: changing identity, tenant, namespace, run or selector aborts the
 * request and destroys its result.
 */
export function PayloadInspector({ runId, selector, label = 'Inspect redacted payload', compact = false }: { runId: string; selector: PayloadSelector; label?: string; compact?: boolean }) {
  const { api } = useApp();
  const allowed = useCan('run:payload:inspect');
  const namespace = useNamespace();
  const session = useSession();
  const { info } = session;
  const [stored, setStored] = useState<ScopedState>({ scope: '', session, state: { phase: 'idle' } });
  const controller = useRef<AbortController | undefined>(undefined);
  const regionId = useId();
  const selectionKey = selector.source === 'input' ? 'input' : `journal:${selector.sequence}`;
  const scopeKey = `${info?.tenant ?? ''}|${info?.subject ?? ''}|${namespace ?? ''}|${runId}|${selectionKey}`;
  const sameScope = stored.scope === scopeKey && stored.session === session;
  if (!sameScope) setStored({ scope: scopeKey, session, state: { phase: 'idle' } });
  const state: LoadState = sameScope ? stored.state : { phase: 'idle' };

  useEffect(() => {
    controller.current?.abort();
    controller.current = undefined;
    return () => controller.current?.abort();
  }, [scopeKey, session]);

  if (!allowed) return null;

  const inspect = async () => {
    controller.current?.abort();
    const next = new AbortController();
    controller.current = next;
    setStored({ scope: scopeKey, session, state: { phase: 'loading' } });
    try {
      const inspection = await api.post<PayloadInspection>(path`/runs/${runId}/payload-inspections`, { body: selector, signal: next.signal });
      if (!next.signal.aborted && controller.current === next) setStored({ scope: scopeKey, session, state: { phase: 'loaded', inspection } });
    } catch (error) {
      if (!next.signal.aborted && controller.current === next) setStored({ scope: scopeKey, session, state: { phase: 'error', error } });
    }
  };

  return (
    <section className={compact ? 'rn-payload compact' : 'rn-payload'} aria-label={label}>
      <div className="rn-payload-action">
        <Button className={compact ? 'small' : undefined} onClick={() => void inspect()} disabled={state.phase === 'loading'} aria-controls={regionId}>
          {state.phase === 'loading' ? 'Inspecting…' : label}
        </Button>
        {state.phase === 'loaded' ? (
          <Button className="small" onClick={() => setStored({ scope: scopeKey, session, state: { phase: 'idle' } })}>
            Close inspection
          </Button>
        ) : null}
      </div>
      <div id={regionId} aria-live="polite">
        {state.phase === 'loading' ? <p role="status">Reading the redacted structure…</p> : null}
        {state.phase === 'error' ? <ErrorState error={state.error} what="Inspecting the redacted payload" onRetry={() => void inspect()} /> : null}
        {state.phase === 'loaded' ? <InspectionView inspection={state.inspection} /> : null}
      </div>
    </section>
  );
}

function InspectionView({ inspection }: { inspection: PayloadInspection }) {
  return (
    <div className="rn-payload-result">
      <p className="rn-payload-policy">
        <strong>Redacted structure only.</strong> Policy <code>{inspection.policy}</code> hides every field name and scalar value.
      </p>
      <dl className="rn-facts">
        <dt>Source</dt>
        <dd>{inspection.selection.source === 'input' ? 'Run input' : `Journal event ${inspection.selection.sequence}`}</dd>
        <dt>Run version</dt>
        <dd>{inspection.runVersion}</dd>
        {inspection.nodeId !== undefined ? (
          <>
            <dt>Step</dt>
            <dd>{inspection.nodeId}</dd>
          </>
        ) : null}
        {inspection.attempt !== undefined ? (
          <>
            <dt>Attempt</dt>
            <dd>{inspection.attempt}</dd>
          </>
        ) : null}
        {inspection.sizeBytes !== undefined ? (
          <>
            <dt>Encoded size</dt>
            <dd>{inspection.sizeBytes.toLocaleString()} bytes</dd>
          </>
        ) : null}
      </dl>
      {inspection.kind === 'inline-json' ? (
        <>
          {inspection.truncated ? <p className="hint">The structure reached an inspection limit. Omitted children are marked below.</p> : null}
          <StructureTree root={inspection.tree} />
        </>
      ) : inspection.kind === 'external' ? (
        <p className="hint">This is external {inspection.contentKind} content. Its metadata is shown, but the content itself was not inspected.</p>
      ) : (
        <p className="hint">
          {inspection.reason === 'NO_RETAINED_PAYLOAD'
            ? 'This event has no retained payload.'
            : 'The retained payload is not supported by the structural JSON inspector.'}
        </p>
      )}
    </div>
  );
}

function StructureTree({ root }: { root: Structure }) {
  return (
    <div className="rn-payload-tree">
      <ul role="tree" aria-label="Redacted payload structure">
        <StructureNode node={root} label="Root" />
      </ul>
    </div>
  );
}

function StructureNode({ node, label }: { node: Structure; label: string }) {
  if (node.type !== 'object' && node.type !== 'array') {
    return (
      <li role="treeitem" aria-selected="false">
        <span>{label}</span> <span className="rn-chip">{node.type}</span> <span className="hint">value hidden</span>
      </li>
    );
  }
  return (
    <li role="treeitem" aria-expanded="true" aria-selected="false">
      <span>{label}</span> <span className="rn-chip">{node.type}</span> <span className="hint">{node.childCount.toLocaleString()} children</span>
      {node.children.length > 0 ? (
        <ul role="group">
          {node.children.map((child, index) => (
            <StructureNode key={index} node={child} label={`${node.type === 'object' ? 'Field' : 'Item'} ${index + 1}`} />
          ))}
        </ul>
      ) : null}
      {node.omittedChildren > 0 ? <p className="hint">{node.omittedChildren.toLocaleString()} children omitted by the inspection limits.</p> : null}
    </li>
  );
}
