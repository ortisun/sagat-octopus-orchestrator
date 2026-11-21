import { useEffect, useId, useRef, useState } from 'react';
import { path } from '../../api/client';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import type { SessionSnapshot } from '../../auth/session';
import { Badge, Button, Code, ErrorState, Facts, Hint, Loading, Row, Stack, Text } from '../../ui';

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
    <section className="rn-payload" aria-label={label}>
      <Row gap={1.5}>
        <Button {...(compact ? { size: 'sm' as const } : {})} onClick={() => void inspect()} disabled={state.phase === 'loading'} aria-controls={regionId}>
          {state.phase === 'loading' ? 'Inspecting…' : label}
        </Button>
        {state.phase === 'loaded' ? (
          <Button size="sm" onClick={() => setStored({ scope: scopeKey, session, state: { phase: 'idle' } })}>
            Close inspection
          </Button>
        ) : null}
      </Row>
      <div id={regionId} aria-live="polite">
        {state.phase === 'loading' ? <Loading>Reading the redacted structure…</Loading> : null}
        {state.phase === 'error' ? <ErrorState error={state.error} what="Inspecting the redacted payload" onRetry={() => void inspect()} /> : null}
        {state.phase === 'loaded' ? <InspectionView inspection={state.inspection} /> : null}
      </div>
    </section>
  );
}

function InspectionView({ inspection }: { inspection: PayloadInspection }) {
  return (
    <div className="rn-payload-result">
      <Stack gap={2}>
      <Text>
        <strong>Redacted structure only.</strong> Policy <Code>{inspection.policy}</Code> hides every field name and scalar value.
      </Text>
      <Facts
        items={[
          { term: 'Source', value: inspection.selection.source === 'input' ? 'Run input' : `Journal event ${inspection.selection.sequence}` },
          { term: 'Run version', value: inspection.runVersion },
          ...(inspection.nodeId !== undefined ? [{ term: 'Step', value: inspection.nodeId }] : []),
          ...(inspection.attempt !== undefined ? [{ term: 'Attempt', value: inspection.attempt }] : []),
          ...(inspection.sizeBytes !== undefined ? [{ term: 'Encoded size', value: `${inspection.sizeBytes.toLocaleString()} bytes` }] : []),
        ]}
      />
      {inspection.kind === 'inline-json' ? (
        <>
          {inspection.truncated ? <Hint>The structure reached an inspection limit. Omitted children are marked below.</Hint> : null}
          <StructureTree root={inspection.tree} />
        </>
      ) : inspection.kind === 'external' ? (
        <Hint>This is external {inspection.contentKind} content. Its metadata is shown, but the content itself was not inspected.</Hint>
      ) : (
        <Hint>
          {inspection.reason === 'NO_RETAINED_PAYLOAD'
            ? 'This event has no retained payload.'
            : 'The retained payload is not supported by the structural JSON inspector.'}
        </Hint>
      )}
      </Stack>
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
        <span>{label}</span> <Badge>{node.type}</Badge> <span className="rn-quiet">value hidden</span>
      </li>
    );
  }
  return (
    <li role="treeitem" aria-expanded="true" aria-selected="false">
      <span>{label}</span> <Badge>{node.type}</Badge> <span className="rn-quiet">{node.childCount.toLocaleString()} children</span>
      {node.children.length > 0 ? (
        <ul role="group">
          {node.children.map((child, index) => (
            <StructureNode key={index} node={child} label={`${node.type === 'object' ? 'Field' : 'Item'} ${index + 1}`} />
          ))}
        </ul>
      ) : null}
      {node.omittedChildren > 0 ? <Hint>{node.omittedChildren.toLocaleString()} children omitted by the inspection limits.</Hint> : null}
    </li>
  );
}
