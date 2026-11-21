import { forwardRef, useEffect, useMemo, useRef, useState } from 'react';
import type { Ref } from 'react';
import { useParams } from 'react-router';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { setEntrypoint } from '../../model/edit';
import type { IrDocument } from '../../model/ir';
import { canonical } from '../../model/ir';
import { kindOf } from '../../model/registry';
import { Badge, Banner, Button, Code, ColumnHeader, FocusHeading, Heading, Hint, Loading, Select, Stack, Table, TableBody, TableCell, TableHead, TableRow, Text, TextArea, TextInput } from '../../ui';
import type { Tone } from '../../ui';
import { JsonField } from './JsonField';
import { TRIGGER_FIELDS, TRIGGER_KINDS, addTrigger, removeTrigger, setDescription, setSchema, setTriggerField, setTriggerKind, triggerKindOf } from './operations';
import type { TriggerKind } from './operations';
import { Group } from './fields';
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

const LEVEL_TONES: Record<ClassificationEntry['level'], Tone> = { public: 'idle', internal: 'running', pii: 'waiting', secret: 'bad' };

/** A level is its word and its shape; the tone only repeats them. */
function level(levelName: ClassificationEntry['level']) {
  return (
    <Badge tone={LEVEL_TONES[levelName]} title={`${levelName} classification`}>
      <span aria-hidden="true">{LEVEL_SHAPES[levelName]}</span> {levelName}
    </Badge>
  );
}

const KIND_OPTIONS = TRIGGER_KINDS.map((k) => ({ value: k, label: KIND_WORDS[k] }));

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
    <Stack as="section" gap={3} aria-label="Classification report">
      <Heading level={3} look={4}>
        Classification report
      </Heading>
      <Hint>The platform computes effective levels and compares this draft with the active version.</Hint>
      <div>
        <Button onClick={load} disabled={current.phase === 'loading'}>
          {current.phase === 'loading' ? 'Reviewing classification…' : 'Review effective classification'}
        </Button>
      </div>
      {current.phase === 'loading' ? <Loading>Checking effective classification…</Loading> : null}
      {(current.failures ?? 0) > 0 ? <Banner tone="warn">The platform could not complete {current.failures} of the classification checks. Try the review again.</Banner> : null}
      {current.phase === 'loaded' && (current.failures ?? 0) === 0 && entries.length === 0 && inheritedWarnings.length === 0 && classificationChanges.length === 0 && uncomparedCarriers.length === 0 ? (
        <Text>No classified schema locations or classification changes were reported.</Text>
      ) : null}
      {entries.length > 0 ? (
        <Table label="Effective classification of declared schema locations">
          <TableHead>
            <TableRow>
              <ColumnHeader>Carrier</ColumnHeader>
              <ColumnHeader>Path</ColumnHeader>
              <ColumnHeader>Effective level</ColumnHeader>
              <ColumnHeader>Source</ColumnHeader>
            </TableRow>
          </TableHead>
          <TableBody>
            {entries.map((entry, index) => (
              <TableRow key={`${entry.carrier}-${entry.nodeId ?? ''}-${entry.path}-${index}`}>
                <TableCell>{carrierName(entry)}</TableCell>
                <TableCell>
                  <Code>{entry.path === '' ? '/ (root)' : entry.path}</Code>
                </TableCell>
                <TableCell>{level(entry.level)}</TableCell>
                <TableCell>{entry.declared ? 'Declared here' : 'Inherited'}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : null}
      {current.validation?.classificationTruncated ? <Hint>The report is limited to 500 locations. More classified locations exist.</Hint> : null}
      {inheritedWarnings.length > 0 ? (
        <Banner tone="warn" title={`Inherited open levels (${inheritedWarnings.length}).`}>
          {inheritedWarnings.map((warning, index) => (
            <span key={`${warning.path ?? ''}-${index}`} className="wf-banner-line">
              <Code>{warning.path ?? '/ (root)'}</Code> {warning.message}
            </span>
          ))}
        </Banner>
      ) : null}
      {uncomparedCarriers.length > 0 ? (
        <Banner tone="info" title={`Some schemas could not be compared (${uncomparedCarriers.length}).`}>
          Their levels were not compared, so a change inside them is not listed. Fix the schema and review again.
          {uncomparedCarriers.map((finding, index) => (
            <span key={`${finding.code}-${finding.path ?? ''}-${index}`} className="wf-banner-line">
              <Code>{finding.path ?? '/ (root)'}</Code> {finding.message} ({finding.code})
            </span>
          ))}
        </Banner>
      ) : null}
      {classificationChanges.length > 0 ? (
        <div className="wf-attention" role="status">
          <Heading level={4}>
            <span aria-hidden="true">!</span> Classification changes need attention ({classificationChanges.length})
          </Heading>
          <ul className="wf-plain-list">
            {classificationChanges.map((finding, index) => (
              <li key={`${finding.code}-${finding.path ?? ''}-${index}`}>
                {finding.message} ({finding.code})
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Stack>
  );
}

function Triggers({ doc, disabled, change }: { doc: IrDocument; disabled: boolean; change: Change }) {
  const triggers = doc.triggers ?? [];
  const [kind, setKind] = useState<TriggerKind>('manual');
  return (
    <Stack as="section" gap={3} aria-label="Triggers">
      <Heading level={3} look={4}>
        Triggers
      </Heading>
      <Hint>What starts a run. A workflow can be started through the API whatever is here.</Hint>
      {triggers.length === 0 ? <Hint>No triggers.</Hint> : null}
      {triggers.map((trigger, index) => {
        const current = triggerKindOf(trigger);
        const label = `Trigger ${index + 1}`;
        return (
          <Group key={index} label={label}>
            {current === undefined ? (
              <Banner tone="warn">A trigger of kind &quot;{String(trigger.kind).slice(0, 40)}&quot; is not one this Studio knows. It is kept as it is.</Banner>
            ) : null}
            <Select
              label={`${label}: kind`}
              value={current ?? ''}
              placeholder="unknown"
              disabled={disabled || current === undefined}
              options={KIND_OPTIONS}
              onChange={(next) => change((d) => setTriggerKind(d, index, next), `trigger:${index}:kind`)}
            />
            {current === undefined
              ? null
              : TRIGGER_FIELDS[current].map((field) => (
                  <TextInput
                    key={field.name}
                    label={`${label}: ${field.label}`}
                    hint={field.hint}
                    value={typeof trigger[field.name] === 'string' ? (trigger[field.name] as string) : ''}
                    disabled={disabled}
                    autoComplete="off"
                    required={field.required === true}
                    onChange={(value) => change((d) => setTriggerField(d, index, field.name, value), `trigger:${index}:${field.name}`)}
                  />
                ))}
            {disabled ? null : (
              <div>
                <Button aria-label={`Remove ${label.toLowerCase()}`} onClick={() => change((d) => removeTrigger(d, index), `trigger:${index}:remove`)}>
                  Remove
                </Button>
              </div>
            )}
          </Group>
        );
      })}
      {disabled ? null : (
        <div className="wf-add-row">
          <Select label="New trigger" value={kind} options={KIND_OPTIONS} onChange={setKind} />
          <Button onClick={() => change((d) => addTrigger(d, kind), 'trigger:add')}>Add trigger</Button>
        </div>
      )}
    </Stack>
  );
}

/** What the inspector shows when no node is selected: the workflow itself. */
export const WorkflowSettings = forwardRef(function WorkflowSettings(
  { doc, disabled, change }: { doc: IrDocument; disabled: boolean; change: Change },
  ref: Ref<HTMLHeadingElement>,
) {
  const description = typeof doc.metadata.description === 'string' ? doc.metadata.description : '';
  return (
    <Stack gap={3}>
      <FocusHeading level={2} look={3} ref={ref}>
        Workflow settings
      </FocusHeading>
      <Text>
        Name: <Code>{doc.metadata.name}</Code>
      </Text>
      <TextArea label="Description" value={description} rows={3} disabled={disabled} onChange={(value) => change((d) => setDescription(d, value), 'description')} />
      <Select
        label="First step"
        hint="The node a run starts at."
        value={doc.entrypoint}
        placeholder="(none)"
        disabled={disabled || doc.nodes.length === 0}
        options={[
          ...(doc.entrypoint !== '' && !doc.nodes.some((n) => n.id === doc.entrypoint) ? [{ value: doc.entrypoint, label: `${doc.entrypoint} (no such node)` }] : []),
          ...doc.nodes.map((n) => ({ value: n.id, label: `${n.id} (${kindOf(n.type)?.label ?? n.type})` })),
        ]}
        onChange={(value) => change((d) => setEntrypoint(d, value), 'entrypoint')}
      />
      <Triggers doc={doc} disabled={disabled} change={change} />
      <Stack as="section" gap={3} aria-label="Data contract">
        <Heading level={3} look={4}>
          Data contract
        </Heading>
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
      </Stack>
      <ClassificationReport doc={doc} />
    </Stack>
  );
});
