import { forwardRef, useEffect, useMemo, useRef, useState } from 'react';
import type { Ref } from 'react';
import { useParams } from 'react-router';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { setEntrypoint } from '../../model/edit';
import type { IrDocument } from '../../model/ir';
import { canonical } from '../../model/ir';
import { kindOf } from '../../model/registry';
import { Banner } from '../../ui/Banner';
import { Button } from '../../ui/Button';
import { Field } from '../../ui/Field';
import { JsonField } from './JsonField';
import { TRIGGER_FIELDS, TRIGGER_KINDS, addTrigger, removeTrigger, setDescription, setSchema, setTriggerField, setTriggerKind, triggerKindOf } from './operations';
import type { TriggerKind } from './operations';
import type { Change } from './fields';

const KIND_WORDS: Record<TriggerKind, string> = {
  manual: 'Manual: somebody starts it',
  event: 'Event: an event of a type',
  webhook: 'Webhook: an endpoint of the namespace',
  message: 'Message: a connector consumes a queue or topic',
  cron: 'Schedule: a cron expression',
};

type Validation = components['schemas']['Validation'];
type DefinitionDiff = components['schemas']['DefinitionDiff'];
type ClassificationEntry = components['schemas']['ClassificationReportEntry'] & { bodyName?: string };

/** The attention codes of a carrier the version comparison could not walk completely. */
const UNCOMPARED_CODES = new Set(['CLASSIFICATION_TOO_COMPLEX', 'CLASSIFICATION_UNRESOLVED_REF', 'CLASSIFICATION_UNSUPPORTED_KEYWORD']);

const LEVEL_SHAPES = { public: '○', internal: '□', pii: '△', secret: '◇' } as const;

function level(levelName: ClassificationEntry['level']) {
  return (
    <span className={`rn-chip rn-level ${levelName}`} aria-label={`${levelName} classification`}>
      <span aria-hidden="true">{LEVEL_SHAPES[levelName]}</span> {levelName}
    </span>
  );
}

function carrierName(entry: ClassificationEntry): string {
  if (entry.carrier === 'inputSchema') return 'Workflow input';
  if (entry.carrier === 'outputSchema') return 'Workflow output';
  return `Step ${entry.nodeId ?? '(unknown)'} output${entry.bodyName ? ` in body ${entry.bodyName}` : ''}`;
}

interface ClassificationState {
  scope: string;
  phase: 'idle' | 'loading' | 'loaded';
  failures?: number;
  validation?: Validation;
  diff?: DefinitionDiff;
}

/** The platform's effective report and classified diff; this view never reinterprets JSON Schema. */
function ClassificationReport({ doc }: { doc: IrDocument }) {
  const { workflowId = '' } = useParams();
  const { ep } = useApp();
  const canValidate = useCan('definition:write');
  const canCompare = useCan('definition:read');
  const namespace = useNamespace();
  const session = useSession();
  const documentKey = useMemo(() => canonical(doc), [doc]);
  const scope = `${session.info?.tenant ?? ''}|${session.info?.subject ?? ''}|${namespace ?? ''}|${workflowId}|${documentKey}`;
  const [stored, setStored] = useState<ClassificationState>({ scope: '', phase: 'idle' });
  const controller = useRef<AbortController | undefined>(undefined);
  const eligible = workflowId !== '' && (canValidate || canCompare);
  const current: ClassificationState = stored.scope === scope ? stored : { scope, phase: 'idle' };

  useEffect(() => {
    controller.current?.abort();
    controller.current = undefined;
    return () => controller.current?.abort();
  }, [scope, session]);

  if (!eligible) return null;

  const load = () => {
    controller.current?.abort();
    const next = new AbortController();
    controller.current = next;
    setStored({ scope, phase: 'loading' });
    const validation = canValidate ? ep.validate(workflowId, doc, next.signal) : Promise.resolve(undefined);
    const diff = canCompare ? ep.diff(workflowId, undefined, undefined, next.signal) : Promise.resolve(undefined);
    void Promise.allSettled([validation, diff]).then(([validationResult, diffResult]) => {
      if (next.signal.aborted || controller.current !== next) return;
      const nextValidation = validationResult.status === 'fulfilled' ? validationResult.value : undefined;
      const nextDiff = diffResult.status === 'fulfilled' ? diffResult.value : undefined;
      const failures = Number(validationResult.status === 'rejected') + Number(diffResult.status === 'rejected');
      setStored({ scope, phase: 'loaded', failures, ...(nextValidation ? { validation: nextValidation } : {}), ...(nextDiff ? { diff: nextDiff } : {}) });
    });
  };

  const entries = current.validation?.classification ?? [];
  const inheritedWarnings = (current.validation?.warnings ?? []).filter((warning) => warning.code === 'CLASSIFICATION_INHERITED_OPEN');
  const classificationChanges = (current.diff?.attention ?? []).filter((finding) => finding.code === 'CLASSIFICATION_RELAXED' || finding.code === 'CLASSIFICATION_TIGHTENED');
  // A carrier the comparison could not walk completely is compared in no part: say so, because
  // a relaxation inside it is not in the list above.
  const uncomparedCarriers = (current.diff?.attention ?? []).filter((finding) => UNCOMPARED_CODES.has(finding.code));

  return (
    <section aria-label="Classification report">
      <h3>Classification report</h3>
      <p className="hint">The platform computes effective levels and compares this draft with the active version.</p>
      <Button onClick={load} disabled={current.phase === 'loading'}>{current.phase === 'loading' ? 'Reviewing classification…' : 'Review effective classification'}</Button>
      {current.phase === 'loading' ? <p role="status">Checking effective classification…</p> : null}
      {(current.failures ?? 0) > 0 ? <Banner tone="warn">The platform could not complete {current.failures} of the classification checks. Try the review again.</Banner> : null}
      {current.phase === 'loaded' && (current.failures ?? 0) === 0 && entries.length === 0 && inheritedWarnings.length === 0 && classificationChanges.length === 0 && uncomparedCarriers.length === 0 ? <p>No classified schema locations or classification changes were reported.</p> : null}
      {entries.length > 0 ? (
        <table className="data">
          <caption>Effective classification of declared schema locations</caption>
          <thead>
            <tr><th scope="col">Carrier</th><th scope="col">Path</th><th scope="col">Effective level</th><th scope="col">Source</th></tr>
          </thead>
          <tbody>
            {entries.map((entry, index) => (
              <tr key={`${entry.carrier}-${entry.nodeId ?? ''}-${entry.path}-${index}`}>
                <td>{carrierName(entry)}</td>
                <td><code>{entry.path === '' ? '/ (root)' : entry.path}</code></td>
                <td>{level(entry.level)}</td>
                <td>{entry.declared ? 'Declared here' : 'Inherited'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : null}
      {current.validation?.classificationTruncated ? <p className="hint">The report is limited to 500 locations. More classified locations exist.</p> : null}
      {inheritedWarnings.length > 0 ? (
        <div className="banner warn" role="alert">
          <strong>Inherited open levels ({inheritedWarnings.length}).</strong>
          <ul>
            {inheritedWarnings.map((warning, index) => <li key={`${warning.path ?? ''}-${index}`}><code>{warning.path ?? '/ (root)'}</code> {warning.message}</li>)}
          </ul>
        </div>
      ) : null}
      {uncomparedCarriers.length > 0 ? (
        <div className="banner warn" role="status">
          <strong>Some schemas could not be compared ({uncomparedCarriers.length}).</strong> Their levels were not compared, so a change inside them is not listed. Fix the schema and review again.
          <ul>
            {uncomparedCarriers.map((finding, index) => <li key={`${finding.code}-${finding.path ?? ''}-${index}`}><code>{finding.path ?? '/ (root)'}</code> {finding.message} <span className="hint">({finding.code})</span></li>)}
          </ul>
        </div>
      ) : null}
      {classificationChanges.length > 0 ? (
        <div className="wf-attention" role="status">
          <h4><span aria-hidden="true">!</span> Classification changes need attention ({classificationChanges.length})</h4>
          <ul>
            {classificationChanges.map((finding, index) => <li key={`${finding.code}-${finding.path ?? ''}-${index}`}>{finding.message} <span className="hint">({finding.code})</span></li>)}
          </ul>
        </div>
      ) : null}
    </section>
  );
}

function Triggers({ doc, disabled, change }: { doc: IrDocument; disabled: boolean; change: Change }) {
  const triggers = doc.triggers ?? [];
  const [kind, setKind] = useState<TriggerKind>('manual');
  return (
    <section aria-label="Triggers">
      <h3>Triggers</h3>
      <p className="hint">What starts a run. A workflow can be started through the API whatever is here.</p>
      {triggers.length === 0 ? <p className="hint">No triggers.</p> : null}
      {triggers.map((trigger, index) => {
        const current = triggerKindOf(trigger);
        const label = `Trigger ${index + 1}`;
        return (
          <fieldset key={index} className="field wf-trigger">
            <legend>{label}</legend>
            {current === undefined ? (
              <Banner tone="warn">A trigger of kind &quot;{String(trigger.kind).slice(0, 40)}&quot; is not one this Studio knows. It is kept as it is.</Banner>
            ) : null}
            <Field label={`${label}: kind`}>
              <select
                value={current ?? ''}
                disabled={disabled || current === undefined}
                onChange={(event) => change((d) => setTriggerKind(d, index, event.target.value as TriggerKind), `trigger:${index}:kind`)}
              >
                {current === undefined ? <option value="">unknown</option> : null}
                {TRIGGER_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {KIND_WORDS[k]}
                  </option>
                ))}
              </select>
            </Field>
            {current === undefined
              ? null
              : TRIGGER_FIELDS[current].map((field) => (
                  <Field key={field.name} label={`${label}: ${field.label}`} {...(field.hint ? { hint: field.hint } : {})}>
                    <input
                      type="text"
                      value={typeof trigger[field.name] === 'string' ? (trigger[field.name] as string) : ''}
                      disabled={disabled}
                      autoComplete="off"
                      aria-required={field.required ? true : undefined}
                      onChange={(event) => change((d) => setTriggerField(d, index, field.name, event.target.value), `trigger:${index}:${field.name}`)}
                    />
                  </Field>
                ))}
            {disabled ? null : (
              <Button aria-label={`Remove ${label.toLowerCase()}`} onClick={() => change((d) => removeTrigger(d, index), `trigger:${index}:remove`)}>
                Remove
              </Button>
            )}
          </fieldset>
        );
      })}
      {disabled ? null : (
        <div className="wf-duration-row wf-add-trigger">
          <Field label="New trigger">
            <select value={kind} onChange={(event) => setKind(event.target.value as TriggerKind)}>
              {TRIGGER_KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_WORDS[k]}
                </option>
              ))}
            </select>
          </Field>
          <Button onClick={() => change((d) => addTrigger(d, kind), 'trigger:add')}>Add trigger</Button>
        </div>
      )}
    </section>
  );
}

/** What the inspector shows when no node is selected: the workflow itself. */
export const WorkflowSettings = forwardRef(function WorkflowSettings(
  { doc, disabled, change }: { doc: IrDocument; disabled: boolean; change: Change },
  ref: Ref<HTMLHeadingElement>,
) {
  const description = typeof doc.metadata.description === 'string' ? doc.metadata.description : '';
  return (
    <div className="wf-inspector-body">
      <h2 ref={ref} tabIndex={-1}>
        Workflow settings
      </h2>
      <p>
        Name: <code>{doc.metadata.name}</code>
      </p>
      <Field label="Description">
        <textarea className="wf-prose" value={description} rows={3} disabled={disabled} onChange={(event) => change((d) => setDescription(d, event.target.value), 'description')} />
      </Field>
      <Field label="First step" hint="The node a run starts at.">
        <select value={doc.entrypoint} disabled={disabled || doc.nodes.length === 0} onChange={(event) => change((d) => setEntrypoint(d, event.target.value), 'entrypoint')}>
          {doc.entrypoint === '' || !doc.nodes.some((n) => n.id === doc.entrypoint) ? (
            <option value={doc.entrypoint}>{doc.entrypoint === '' ? '(none)' : `${doc.entrypoint} (no such node)`}</option>
          ) : null}
          {doc.nodes.map((n) => (
            <option key={n.id} value={n.id}>
              {n.id} ({kindOf(n.type)?.label ?? n.type})
            </option>
          ))}
        </select>
      </Field>
      <Triggers doc={doc} disabled={disabled} change={change} />
      <section aria-label="Data contract">
        <h3>Data contract</h3>
        <JsonField
          model="workflowInputSchema"
          label="Input schema"
          hint="A JSON Schema object that the input of a run must satisfy. Empty: any input."
          value={doc.inputSchema}
          disabled={disabled}
          onCommit={(value) => change((d) => setSchema(d, 'inputSchema', value), 'inputSchema')}
        />
        <JsonField
          model="workflowOutputSchema"
          label="Output schema"
          hint="A JSON Schema object that describes the output of a run."
          value={doc.outputSchema}
          disabled={disabled}
          onCommit={(value) => change((d) => setSchema(d, 'outputSchema', value), 'outputSchema')}
        />
      </section>
      <ClassificationReport doc={doc} />
    </div>
  );
});
