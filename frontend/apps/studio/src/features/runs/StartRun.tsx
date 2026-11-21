import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useNavigate } from 'react-router';
import { path } from '../../api/client';
import type { Run } from '../../api/types';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { parseAuthoringJson, UnsafeJsonNumber } from '../../model/safeJson';
import { Actions, Banner, Button, Dialog, ErrorState, Select, Stack, Text, TextArea, TextInput, useAnnounce } from '../../ui';
import { CONFLICTS, REUSES, businessFields, businessIdProblem } from './businessId';
import type { Conflict, Reuse } from './businessId';
import { DELAY_UNITS, describeDelay, startDelayFields, startDelayProblem } from './startDelay';
import type { DelayUnit } from './startDelay';
import { clip } from './time';
import { shortId } from './display';

/** What a person may start: a workflow that has a published version. */
export interface Startable {
  id: string;
  name: string;
}

const MAX_INPUT_BYTES = 256 * 1024;

/** The input as the platform takes it: a JSON object, small enough to send. Nothing here is sent. */
export function parseInput(text: string): { input: Record<string, unknown> } | { problem: string } {
  let value: unknown;
  try {
    value = parseAuthoringJson(text);
  } catch (cause) {
    if (cause instanceof UnsafeJsonNumber) return { problem: cause.message };
    return { problem: 'Enter a valid JSON object.' };
  }
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return { problem: 'Enter a JSON object, for example {"orderId": "1234"}.' };
  if (new TextEncoder().encode(JSON.stringify(value)).length > MAX_INPUT_BYTES) return { problem: 'The input must be at most 256 KiB.' };
  return { input: value as Record<string, unknown> };
}

/**
 * *Start run…*: starts a run of a published workflow with an input the person types. Shown only to
 * somebody with `run:start`, and only when there is a workflow with a published version to start.
 * The form belongs to one session and one namespace and never keeps its input; a retry of the same
 * request keeps its idempotency key, so a double click or a lost answer starts one run, not two.
 */
export function StartRun({ choices, initialId, label = 'Start run…' }: { choices: readonly Startable[]; initialId?: string; label?: string }) {
  const allowed = useCan('run:start');
  const namespace = useNamespace();
  const { session } = useApp();
  useSession();
  if (!allowed || choices.length === 0) return null;
  return <ScopedStart key={`${namespace ?? ''}|${session.revision()}`} choices={choices} initialId={initialId} label={label} />;
}

function ScopedStart({ choices, initialId, label }: { choices: readonly Startable[]; initialId: string | undefined; label: string }) {
  const [open, setOpen] = useState(false);
  // In a row of a list the button says which workflow, for assistive technology; its visible text stays what it is.
  const only = initialId !== undefined && choices.length === 1 ? choices[0] : undefined;
  return (
    <>
      <Button onClick={() => setOpen(true)} {...(only ? { 'aria-label': `${label} of ${clip(only.name, 64)}` } : {})}>
        {label}
      </Button>
      <Dialog open={open} title="Start a run" onClose={() => setOpen(false)}>
        {open ? <StartForm choices={choices} initialId={initialId} onClose={() => setOpen(false)} /> : null}
      </Dialog>
    </>
  );
}

function StartForm({ choices, initialId, onClose }: { choices: readonly Startable[]; initialId: string | undefined; onClose: () => void }) {
  const { api, session } = useApp();
  const announce = useAnnounce();
  const navigate = useNavigate();
  const [workflowId, setWorkflowId] = useState(initialId !== undefined && choices.some((c) => c.id === initialId) ? initialId : (choices[0]?.id ?? ''));
  const [text, setText] = useState('{}');
  const [businessId, setBusinessId] = useState('');
  const [conflict, setConflict] = useState<Conflict>('FAIL');
  const [reuse, setReuse] = useState<Reuse>('ALLOW_DUPLICATE');
  const [idProblem, setIdProblem] = useState<string>();
  const [delayAmount, setDelayAmount] = useState('');
  const [delayUnit, setDelayUnit] = useState<DelayUnit>('hours');
  const [delayProblem, setDelayProblem] = useState<string>();
  const [problem, setProblem] = useState<string>();
  const [error, setError] = useState<unknown>();
  const [starting, setStarting] = useState(false);
  const previous = useRef<{ fingerprint: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => pending.current?.abort(), []);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (starting) return;
    setError(undefined);
    const parsed = parseInput(text);
    if ('problem' in parsed) {
      setProblem(parsed.problem);
      return;
    }
    setProblem(undefined);
    const id = businessId.trim();
    const wrong = businessIdProblem(id);
    setIdProblem(wrong);
    if (wrong) return;
    const business = businessFields(id, conflict, reuse);
    const lateness = startDelayProblem(delayAmount, delayUnit);
    setDelayProblem(lateness);
    if (lateness) return;
    const delay = startDelayFields(delayAmount, delayUnit);
    const fingerprint = `${workflowId}\n${JSON.stringify(parsed.input)}\n${JSON.stringify(business)}\n${JSON.stringify(delay)}`;
    if (previous.current?.fingerprint !== fingerprint) previous.current = { fingerprint, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    const token = session.token();
    const current = () => !controller.signal.aborted && session.token() === token;
    setStarting(true);
    try {
      // `200` rather than `202`: an open run already had the identifier and was returned, nothing was started
      let existing = false;
      const run = await api.post<Run>('/runs', {
        body: { workflowId, input: parsed.input, ...business, ...delay },
        key: previous.current.key,
        signal: controller.signal,
        onStatus: (status) => {
          existing = status === 200;
        },
      });
      if (!current()) return;
      const waits = delay.startDelayMs === undefined ? '' : ` It waits ${describeDelay(delayAmount, delayUnit)} before it starts.`;
      announce(existing ? `That identifier already has an open run, ${shortId(run.runId)}. Nothing was started.` : `Started run ${shortId(run.runId)}.${waits}`);
      onClose();
      void navigate(path`/runs/${run.runId}`);
    } catch (cause) {
      if (current()) setError(cause);
    } finally {
      if (current()) setStarting(false);
    }
  };

  return (
    <form
      onSubmit={(event) => {
        void submit(event);
      }}
    >
      <Stack gap={3}>
        <Text>This starts a new run of the workflow's active version. It runs at once, unless you give it a delay, and is recorded with your name; steps with effects outside the platform will run.</Text>
        <Select label="Workflow" value={workflowId} disabled={starting || choices.length === 1} onChange={setWorkflowId} options={choices.map((choice) => ({ value: choice.id, label: clip(choice.name, 80) }))} />
        <TextArea label="Input (JSON object)" hint="At most 256 KiB. Do not include credentials or secrets: the input is kept with the run." rows={6} spellCheck={false} mono value={text} disabled={starting} onChange={setText} />
        <TextInput
          label="Business identifier (optional)"
          hint="Your own name for the run, such as an order number: 1 to 200 letters, digits and punctuation. Only one open run can have it."
          error={idProblem}
          autoComplete="off"
          spellCheck={false}
          maxLength={200}
          value={businessId}
          disabled={starting}
          onChange={setBusinessId}
        />
        {businessId.trim() !== '' ? (
          <>
            <Select label="If an open run already has it" value={conflict} disabled={starting} onChange={setConflict} options={CONFLICTS} />
            <Select label="If a finished run still remembers it" hint="A finished run remembers its identifier for 30 days." value={reuse} disabled={starting} onChange={setReuse} options={REUSES} />
          </>
        ) : null}
        <TextInput
          label="Start after (optional)"
          hint="The run is created now and waits; its first step is released after this long (at most 366 days). While it waits it counts as an active run and holds its business identifier."
          error={delayProblem}
          numeric
          autoComplete="off"
          spellCheck={false}
          maxLength={9}
          value={delayAmount}
          disabled={starting}
          onChange={setDelayAmount}
        />
        <Select label="Unit of the delay" value={delayUnit} disabled={starting} onChange={setDelayUnit} options={DELAY_UNITS} />
        {problem ? <Banner tone="error">{problem}</Banner> : null}
        {error ? <ErrorState error={error} what="Starting the run" /> : null}
        <Actions>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={starting}>
            {starting ? 'Starting…' : 'Start run'}
          </Button>
        </Actions>
      </Stack>
    </form>
  );
}
