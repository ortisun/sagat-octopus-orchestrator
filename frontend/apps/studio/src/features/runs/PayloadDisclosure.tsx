import { useEffect, useId, useRef, useState } from 'react';
import { path } from '../../api/client';
import { ApiError } from '../../api/errors';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import type { SessionSnapshot } from '../../auth/session';
import { Actions, Badge, Button, Code, Dialog, ErrorState, Facts, Hint, Loading, Row, Stack, Text } from '../../ui';

export type PayloadDisclosureSelector = components['schemas']['PayloadDisclosureSelector'];
export type PayloadDisclosure = components['schemas']['PayloadDisclosure'];
type DisclosureNode = components['schemas']['PayloadDisclosureNode'];

type LoadState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'loaded'; disclosure: PayloadDisclosure }
  | { phase: 'error'; error: unknown };

interface ScopedState {
  scope: string;
  session: SessionSnapshot;
  state: LoadState;
}

const LEVEL_SHAPES = { public: '○', internal: '□', pii: '△', secret: '◇' } as const;

/** Text and shape both identify a classification level, including in forced-colour mode. */
function Level({ level }: { level: keyof typeof LEVEL_SHAPES }) {
  return (
    <span aria-label={`${level} classification`}>
      <Badge>
        <span aria-hidden="true">{LEVEL_SHAPES[level]}</span> {level}
      </Badge>
    </span>
  );
}

function scalar(value: string | number | boolean | null): string {
  return JSON.stringify(value);
}

function DisclosureTree({ root }: { root: DisclosureNode }) {
  return (
    <div className="rn-payload-tree">
      <ul role="tree" aria-label="Classified payload values">
        <DisclosureTreeNode node={root} label="Root" />
      </ul>
    </div>
  );
}

function DisclosureTreeNode({ node, label }: { node: DisclosureNode; label: string }) {
  if (node.kind === 'withheld') {
    return (
      <li role="treeitem" aria-selected="false">
        <span>{label}</span> <Level level={node.level} /> <Badge>{node.type}</Badge>{' '}
        <span className="rn-quiet">[withheld: {node.level}]</span>
      </li>
    );
  }
  if (node.kind === 'value') {
    return (
      <li role="treeitem" aria-selected="false">
        <span>{label}</span> <Level level={node.level} /> <Badge>{node.type}</Badge>{' '}
        <Code>{scalar(node.value)}</Code>
        {node.valueTruncated ? <span className="rn-quiet"> value shortened from {node.length?.toLocaleString() ?? 'an unknown number of'} bytes</span> : null}
      </li>
    );
  }
  return (
    <li role="treeitem" aria-expanded="true" aria-selected="false">
      <span>{label}</span> <Level level={node.level} /> <Badge>{node.type}</Badge>{' '}
      <span className="rn-quiet">{node.childCount.toLocaleString()} children</span>
      {node.children.length > 0 ? (
        <ul role="group">
          {node.children.map((child) => (
            <DisclosureTreeNode
              key={child.ordinal}
              node={child.node}
              label={child.name ?? `${node.type === 'array' ? 'Item' : 'Field'} ${child.ordinal + 1}`}
            />
          ))}
        </ul>
      ) : null}
      {node.omittedChildren > 0 ? <Hint>{node.omittedChildren.toLocaleString()} children omitted by the disclosure limits.</Hint> : null}
    </li>
  );
}

function DisclosureView({ disclosure }: { disclosure: PayloadDisclosure }) {
  return (
    <div className="rn-payload-result">
      <Stack gap={2}>
      <Text>
        <strong>Classified values.</strong> Policy <Code>{disclosure.policy}</Code> returned only the levels listed by the platform.
      </Text>
      <Facts
        items={[
          {
            term: 'Source',
            value: disclosure.selection.source === 'input' ? 'Run input' : disclosure.selection.source === 'result' ? 'Run result' : `Journal event ${disclosure.selection.sequence}`,
          },
          { term: 'Run version', value: disclosure.runVersion },
          { term: 'Levels returned', value: disclosure.caller.disclosedLevels.length > 0 ? disclosure.caller.disclosedLevels.map((level) => <Level key={level} level={level} />) : 'None' },
          ...(disclosure.nodeId !== undefined ? [{ term: 'Step', value: disclosure.nodeId }] : []),
          ...(disclosure.attempt !== undefined ? [{ term: 'Attempt', value: disclosure.attempt }] : []),
        ]}
      />
      {disclosure.kind === 'disclosed' ? (
        <>
          {disclosure.truncated ? <Hint>The answer reached a disclosure limit. Omitted children and shortened values are marked below.</Hint> : null}
          <DisclosureTree root={disclosure.tree} />
        </>
      ) : disclosure.kind === 'structure-only' ? (
        <Hint>Only redacted structure is available ({disclosure.reason}). No payload value was returned.</Hint>
      ) : disclosure.kind === 'external' ? (
        <Hint>This is external {disclosure.contentKind} content. The content itself was not read or returned.</Hint>
      ) : (
        <Hint>
          {disclosure.reason === 'NO_RETAINED_PAYLOAD'
            ? 'No retained payload is available for this source.'
            : disclosure.reason === 'NOT_DISCLOSABLE'
              ? 'This payload is not disclosable. No value or validation detail was returned.'
              : 'The retained payload format cannot be disclosed.'}
        </Hint>
      )}
      </Stack>
    </div>
  );
}

/**
 * A lazy view of the server's classified projection. Payload values live only in component state:
 * changing identity, tenant, namespace, run or selected event aborts the request and discards them.
 */
export function PayloadDisclosureView({
  runId,
  selector,
  label = 'View permitted values',
  compact = false,
}: {
  runId: string;
  selector: PayloadDisclosureSelector;
  label?: string;
  compact?: boolean;
}) {
  const { api } = useApp();
  const allowed = useCan('run:read');
  const namespace = useNamespace();
  const session = useSession();
  const { info } = session;
  const [stored, setStored] = useState<ScopedState>({ scope: '', session, state: { phase: 'idle' } });
  const [confirmScope, setConfirmScope] = useState<string | undefined>(undefined);
  const controller = useRef<AbortController | undefined>(undefined);
  const regionId = useId();
  const selectionKey = selector.source === 'journal' ? `journal:${selector.sequence}` : selector.source;
  const scopeKey = `${info?.tenant ?? ''}|${info?.subject ?? ''}|${namespace ?? ''}|${runId}|${selectionKey}`;
  const sameScope = stored.scope === scopeKey && stored.session === session;
  if (!sameScope) setStored({ scope: scopeKey, session, state: { phase: 'idle' } });
  const state: LoadState = sameScope ? stored.state : { phase: 'idle' };
  const confirmationOpen = confirmScope === scopeKey && state.phase === 'loaded';

  useEffect(() => {
    controller.current?.abort();
    controller.current = undefined;
    return () => controller.current?.abort();
  }, [scopeKey, session]);

  if (!allowed) return null;

  const disclose = async (reveal: boolean) => {
    controller.current?.abort();
    const next = new AbortController();
    controller.current = next;
    setConfirmScope(undefined);
    setStored({ scope: scopeKey, session, state: { phase: 'loading' } });
    try {
      const body: components['schemas']['PayloadDisclosureRequest'] = reveal ? { ...selector, reveal: ['pii'] } : selector;
      const disclosure = await api.post<PayloadDisclosure>(path`/runs/${runId}/payload-disclosures`, { body, signal: next.signal, safeNumbers: true });
      if (!next.signal.aborted && controller.current === next) setStored({ scope: scopeKey, session, state: { phase: 'loaded', disclosure } });
    } catch (error) {
      const displayError = error instanceof ApiError && error.code === 'VALIDATION_FAILED' && error.status >= 200 && error.status < 300
        ? new ApiError({ code: 'VALIDATION_FAILED', status: error.status, message: 'This payload contains JSON numbers the Studio cannot display exactly. Use the API or CLI to read their exact representation.' })
        : error;
      if (!next.signal.aborted && controller.current === next) setStored({ scope: scopeKey, session, state: { phase: 'error', error: displayError } });
    }
  };

  const clear = () => {
    controller.current?.abort();
    controller.current = undefined;
    setStored({ scope: scopeKey, session, state: { phase: 'idle' } });
  };

  const canReveal = state.phase === 'loaded' && state.disclosure.caller.revealPii === 'granted' && !state.disclosure.caller.revealed;

  return (
    <section className="rn-payload" aria-label={label}>
      <Row gap={1.5}>
        <Button {...(compact ? { size: 'sm' as const } : {})} onClick={() => void disclose(false)} disabled={state.phase === 'loading'} aria-controls={regionId}>
          {state.phase === 'loading' ? 'Reading values…' : label}
        </Button>
        {canReveal ? <Button size="sm" onClick={() => setConfirmScope(scopeKey)}>Reveal personal data</Button> : null}
        {state.phase === 'loaded' ? <Button size="sm" onClick={clear}>Close values</Button> : null}
      </Row>
      <div id={regionId} aria-live="polite">
        {state.phase === 'loading' ? <Loading>Reading the classified projection…</Loading> : null}
        {state.phase === 'error' ? <ErrorState error={state.error} what="Reading permitted payload values" onRetry={() => void disclose(false)} /> : null}
        {state.phase === 'loaded' ? <DisclosureView disclosure={state.disclosure} /> : null}
      </div>
      <Dialog open={confirmationOpen} title="Reveal personal data?" onClose={() => setConfirmScope(undefined)}>
        <Text>This audited request may return personal data. Use it only for the current diagnostic task.</Text>
        <Actions>
          <Button onClick={() => setConfirmScope(undefined)}>Cancel</Button>
          <Button variant="primary" onClick={() => void disclose(true)}>Reveal personal data</Button>
        </Actions>
      </Dialog>
    </section>
  );
}
