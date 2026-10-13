import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { useAnnounce } from '../../ui/Announcer';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import type { OperationsApi } from './api';

const MIN_REASON = 8;
const MAX_REASON = 2_000;

type Action = 'drain' | 'resume';

function reasonProblem(reason: string): string | undefined {
  const length = [...reason.trim()].length;
  if (length < MIN_REASON) return `Give a reason of at least ${MIN_REASON} characters.`;
  if (length > MAX_REASON) return `Keep the reason to ${MAX_REASON} characters or fewer.`;
  return undefined;
}

export function WorkerActionDialog({
  action,
  api,
  namespace,
  subject,
  onClose,
  onChanged,
}: {
  action: Action | undefined;
  api: OperationsApi;
  namespace: string;
  subject: string | undefined;
  onClose: () => void;
  onChanged: () => void;
}) {
  return (
    <Dialog open={action !== undefined && subject !== undefined} title={action === 'resume' ? 'Resume worker identity' : 'Drain worker identity'} onClose={onClose}>
      {action === undefined || subject === undefined ? null : (
        <WorkerActionForm action={action} api={api} namespace={namespace} subject={subject} onClose={onClose} onChanged={onChanged} />
      )}
    </Dialog>
  );
}

function WorkerActionForm({ action, api, namespace, subject, onClose, onChanged }: {
  action: Action;
  api: OperationsApi;
  namespace: string;
  subject: string;
  onClose: () => void;
  onChanged: () => void;
}) {
  const announce = useAnnounce();
  const [reason, setReason] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<unknown>();
  const reasonRef = useRef<HTMLTextAreaElement>(null);
  const request = useRef<{ print: string; key: string } | undefined>(undefined);
  const inFlight = useRef<AbortController | undefined>(undefined);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      inFlight.current?.abort();
    };
  }, []);

  const send = async () => {
    const trimmed = reason.trim();
    const print = JSON.stringify({ action, namespace, subject, reason: trimmed });
    if (request.current?.print !== print) request.current = { print, key: crypto.randomUUID() };
    const controller = new AbortController();
    inFlight.current?.abort();
    inFlight.current = controller;
    setSending(true);
    setError(undefined);
    try {
      let changed = true;
      if (action === 'drain') await api.drain(subject, trimmed, request.current.key, controller.signal);
      else changed = (await api.resume(subject, trimmed, request.current.key, controller.signal)).wasDraining;
      if (controller.signal.aborted || !mounted.current) return;
      onChanged();
      announce(action === 'drain' ? 'Worker identity drained.' : changed ? 'Worker identity resumed.' : 'Worker identity was already accepting claims.');
      onClose();
    } catch (cause) {
      if (!controller.signal.aborted && mounted.current) setError(cause);
    } finally {
      if (!controller.signal.aborted && mounted.current) setSending(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    setSubmitted(true);
    if (reasonProblem(reason)) {
      reasonRef.current?.focus();
      return;
    }
    void send();
  };

  const problem = submitted ? reasonProblem(reason) : undefined;
  return (
    <form className="operations-action" onSubmit={submit} noValidate>
      {action === 'drain' ? (
        <p>
          This applies to every session of <strong>{subject}</strong> in <strong>{namespace}</strong>. It prevents this identity from claiming new attempts and does not cancel attempts already in flight.
        </p>
      ) : (
        <p>
          This applies to every session of <strong>{subject}</strong> in <strong>{namespace}</strong>. It lets this identity claim new attempts again; attempts already in flight are unchanged.
        </p>
      )}
      <p className="hint">The platform records this action with your identity and reason.</p>
      <Field label="Reason" hint={`${[...reason.trim()].length} / ${MAX_REASON} characters; stored in the audit trail.`} error={problem}>
        <textarea ref={reasonRef} rows={4} value={reason} onChange={(event) => setReason(event.target.value)} />
      </Field>
      {error === undefined ? null : <ErrorState error={error} what={action === 'drain' ? 'Draining the worker identity' : 'Resuming the worker identity'} onRetry={() => void send()} />}
      <div className="dialog-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button type="submit" variant={action === 'drain' ? 'danger' : 'primary'} disabled={sending}>
          {sending ? 'Sending…' : action === 'drain' ? 'Confirm drain' : 'Confirm resume'}
        </Button>
      </div>
    </form>
  );
}
