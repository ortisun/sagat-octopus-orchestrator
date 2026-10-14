import { forwardRef, useState } from 'react';
import type { Ref } from 'react';
import { setEntrypoint } from '../../model/edit';
import type { IrDocument } from '../../model/ir';
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
        <div className="wf-duration-row">
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
        <textarea value={description} rows={2} disabled={disabled} onChange={(event) => change((d) => setDescription(d, event.target.value), 'description')} />
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
    </div>
  );
});
