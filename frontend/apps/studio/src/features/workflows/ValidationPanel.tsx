import { forwardRef, useImperativeHandle, useRef } from 'react';
import type { Advice } from '../../model/advice';
import { Button, Code, ErrorState, Heading, Hint, Row, Text } from '../../ui';
import { clip } from './canvasModel';
import type { ValidationState } from './useValidation';
import type { ProblemView } from './validationModel';

/** More than this many findings are summarised: nobody reads two thousand lines, and the DOM should not hold them. */
const SHOWN = 200;

export interface ValidationPanelHandle {
  focus: () => void;
}

function NodeChip({ id, exists, onSelect }: { id: string | undefined; exists: boolean; onSelect: (id: string) => void }) {
  if (id === undefined || id === '') return <span className="wf-chip-plain">Workflow</span>;
  if (!exists) return <Code>{clip(id, 64)}</Code>;
  return (
    <Button size="sm" aria-label={`Select node ${clip(id, 64)}`} onClick={() => onSelect(id)}>
      {clip(id, 64)}
    </Button>
  );
}

function Findings({
  items,
  kind,
  exists,
  onSelect,
}: {
  items: readonly ProblemView[];
  kind: 'problem' | 'warning';
  exists: (id: string) => boolean;
  onSelect: (id: string) => void;
}) {
  return (
    <ul className="wf-findings" data-kind={kind}>
      {items.slice(0, SHOWN).map((item, index) => (
        <li key={`${item.code}-${item.nodeId ?? ''}-${item.path ?? ''}-${index}`}>
          <span aria-hidden="true">{kind === 'problem' ? '✕' : '!'}</span>{' '}
          <NodeChip id={item.nodeId} exists={item.nodeId !== undefined && exists(item.nodeId)} onSelect={onSelect} />{' '}
          <span className="wf-finding-text">{clip(item.message, 400)}</span>{' '}
          <span className="wf-quiet">
            ({clip(item.code, 40)}
            {item.path ? ` ${clip(item.path, 80)}` : ''})
          </span>
        </li>
      ))}
      {items.length > SHOWN ? <li className="wf-quiet">and {items.length - SHOWN} more.</li> : null}
    </ul>
  );
}

/**
 * What the platform and the Studio have to say about the document, in two separate groups: the
 * platform's problems and warnings (its validation is the authority and gates publishing), and
 * advice the Studio works out itself, labelled as such and never blocking.
 */
export const ValidationPanel = forwardRef<
  ValidationPanelHandle,
  { state: ValidationState; advice: readonly Advice[]; exists: (id: string) => boolean; onSelectNode: (id: string) => void; checkedSummary: string }
>(function ValidationPanel({ state, advice, exists, onSelectNode, checkedSummary }, ref) {
  const root = useRef<HTMLElement>(null);
  useImperativeHandle(ref, () => ({ focus: () => root.current?.focus() }), []);
  const view = state.view;
  return (
    <section ref={root} tabIndex={-1} className="wf-validation" aria-label="Validation">
      <Heading level={2} look={4}>
        Validation
      </Heading>
      <Hint>{checkedSummary}</Hint>
      {state.phase === 'off' ? null : (
        <Row>
          <Button onClick={state.recheck} disabled={state.phase === 'checking'}>
            Check again
          </Button>
          <Hint>The platform's answer also depends on what exists: a script built or a secret created since it answered is not in it.</Hint>
        </Row>
      )}
      {state.phase === 'failed' ? <ErrorState error={state.error} what="Checking the workflow" /> : null}
      {view === undefined && state.phase === 'checking' ? <Text>Checking with the platform…</Text> : null}
      {view ? (
        <>
          <Heading level={3} look={5}>Problems ({view.problems.length})</Heading>
          {view.problems.length === 0 ? (
            <Text>{state.current ? 'The platform found no problems.' : 'No problems in the last answer.'}</Text>
          ) : (
            <Findings items={view.problems} kind="problem" exists={exists} onSelect={onSelectNode} />
          )}
          <Heading level={3} look={5}>Warnings ({view.warnings.length})</Heading>
          {view.warnings.length === 0 ? <Text>None.</Text> : <Findings items={view.warnings} kind="warning" exists={exists} onSelect={onSelectNode} />}
        </>
      ) : null}
      <Heading level={3} look={5}>Advice (not checked by the platform) ({advice.length})</Heading>
      <Hint>The Studio works these out from the shape of the workflow. They never stop a save or a publish.</Hint>
      {advice.length === 0 ? (
        <Text>None.</Text>
      ) : (
        <ul className="wf-findings" data-kind="advice">
          {advice.slice(0, SHOWN).map((item, index) => (
            <li key={`${item.code}-${item.nodeId}-${index}`}>
              <span aria-hidden="true">·</span> <NodeChip id={item.nodeId} exists={item.nodeId !== '' && exists(item.nodeId)} onSelect={onSelectNode} />{' '}
              <span className="wf-finding-text">{clip(item.message, 400)}</span>
            </li>
          ))}
          {advice.length > SHOWN ? <li className="wf-quiet">and {advice.length - SHOWN} more.</li> : null}
        </ul>
      )}
    </section>
  );
});
