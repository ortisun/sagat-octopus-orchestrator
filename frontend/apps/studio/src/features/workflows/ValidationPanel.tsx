import { forwardRef, useImperativeHandle, useRef } from 'react';
import type { Advice } from '../../model/advice';
import { ErrorState } from '../../ui/ErrorState';
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
  if (!exists) return <code>{clip(id, 64)}</code>;
  return (
    <button type="button" className="wf-chip" aria-label={`Select node ${clip(id, 64)}`} onClick={() => onSelect(id)}>
      {clip(id, 64)}
    </button>
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
          <span className="hint">
            ({clip(item.code, 40)}
            {item.path ? ` ${clip(item.path, 80)}` : ''})
          </span>
        </li>
      ))}
      {items.length > SHOWN ? <li className="hint">and {items.length - SHOWN} more.</li> : null}
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
      <h2 className="wf-panel-title">Validation</h2>
      <p className="hint">{checkedSummary}</p>
      {state.phase === 'failed' ? <ErrorState error={state.error} what="Checking the workflow" /> : null}
      {view === undefined && state.phase === 'checking' ? <p>Checking with the platform…</p> : null}
      {view ? (
        <>
          <h3>Problems ({view.problems.length})</h3>
          {view.problems.length === 0 ? (
            <p>{state.current ? 'The platform found no problems.' : 'No problems in the last answer.'}</p>
          ) : (
            <Findings items={view.problems} kind="problem" exists={exists} onSelect={onSelectNode} />
          )}
          <h3>Warnings ({view.warnings.length})</h3>
          {view.warnings.length === 0 ? <p>None.</p> : <Findings items={view.warnings} kind="warning" exists={exists} onSelect={onSelectNode} />}
        </>
      ) : null}
      <h3>Advice (not checked by the platform) ({advice.length})</h3>
      <p className="hint">The Studio works these out from the shape of the workflow. They never stop a save or a publish.</p>
      {advice.length === 0 ? (
        <p>None.</p>
      ) : (
        <ul className="wf-findings" data-kind="advice">
          {advice.slice(0, SHOWN).map((item, index) => (
            <li key={`${item.code}-${item.nodeId}-${index}`}>
              <span aria-hidden="true">·</span> <NodeChip id={item.nodeId} exists={item.nodeId !== '' && exists(item.nodeId)} onSelect={onSelectNode} />{' '}
              <span className="wf-finding-text">{clip(item.message, 400)}</span>
            </li>
          ))}
          {advice.length > SHOWN ? <li className="hint">and {advice.length - SHOWN} more.</li> : null}
        </ul>
      )}
    </section>
  );
});
