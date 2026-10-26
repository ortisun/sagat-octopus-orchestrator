import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useNavigate } from 'react-router';
import { path } from '../../api/client';
import type { Run } from '../../api/types';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { parseAuthoringJson, UnsafeJsonNumber } from '../../model/safeJson';
import { useAnnounce } from '../../ui/Announcer';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { clip } from './time';

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
      <Button onClick={() => setOpen(true)}>
        {label}
        {only ? (
          <>
            {' '}
            <span className="visually-hidden">of {clip(only.name, 64)}</span>
          </>
        ) : null}
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
    const fingerprint = `${workflowId}\n${JSON.stringify(parsed.input)}`;
    if (previous.current?.fingerprint !== fingerprint) previous.current = { fingerprint, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    const token = session.token();
    const current = () => !controller.signal.aborted && session.token() === token;
    setStarting(true);
    try {
      const run = await api.post<Run>('/runs', { body: { workflowId, input: parsed.input }, key: previous.current.key, signal: controller.signal });
      if (!current()) return;
      announce(`Started run ${run.runId.slice(-8)}.`);
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
      <p>This starts a new run of the workflow's active version. It runs at once and is recorded with your name; steps with effects outside the platform will run.</p>
      <Field label="Workflow">
        <select value={workflowId} disabled={starting || choices.length === 1} onChange={(event) => setWorkflowId(event.target.value)}>
          {choices.map((choice) => (
            <option key={choice.id} value={choice.id}>
              {clip(choice.name, 80)}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Input (JSON object)" hint="At most 256 KiB. Do not include credentials or secrets: the input is kept with the run.">
        <textarea rows={6} autoComplete="off" spellCheck={false} value={text} disabled={starting} onChange={(event) => setText(event.target.value)} />
      </Field>
      {problem ? (
        <p role="alert" className="error">
          {problem}
        </p>
      ) : null}
      {error ? <ErrorState error={error} what="Starting the run" /> : null}
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant="primary" disabled={starting}>
          {starting ? 'Starting…' : 'Start run'}
        </Button>
      </div>
    </form>
  );
}
