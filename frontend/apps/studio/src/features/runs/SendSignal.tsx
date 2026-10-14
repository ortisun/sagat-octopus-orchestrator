import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { path } from '../../api/client';
import { isTerminal } from '../../api/follow';
import type { components } from '../../api/schema';
import { useApp, useCan, useNamespace, useSession } from '../../app/context';
import { Button } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { ErrorState } from '../../ui/ErrorState';
import { Field } from '../../ui/Field';
import { useAnnounce } from '../../ui/Announcer';

/** The form belongs to one authenticated run context and never persists its payload. */
export function SendSignal({ runId, status, onSent }: { runId: string; status: string; onSent: () => void }) {
  const allowed = useCan('run:signal');
  const namespace = useNamespace();
  useSession();
  const { session } = useApp();
  if (!allowed || isTerminal(status)) return null;
  return <ScopedSignal key={`${namespace ?? ''}|${session.revision()}|${runId}`} runId={runId} onSent={onSent} />;
}

function ScopedSignal({ runId, onSent }: { runId: string; onSent: () => void }) {
  const [open, setOpen] = useState(false);
  return <>
    <Button onClick={() => setOpen(true)}>Send signal</Button>
    <Dialog open={open} title="Send signal" onClose={() => setOpen(false)}>
      {open ? <SignalForm runId={runId} onClose={() => setOpen(false)} onSent={onSent} /> : null}
    </Dialog>
  </>;
}

function SignalForm({ runId, onClose, onSent }: { runId: string; onClose: () => void; onSent: () => void }) {
  const { api, session } = useApp();
  const announce = useAnnounce();
  const [name, setName] = useState('');
  const [payload, setPayload] = useState('{}');
  const [problem, setProblem] = useState<string>();
  const [error, setError] = useState<unknown>();
  const [sending, setSending] = useState(false);
  const previous = useRef<{ fingerprint: string; key: string } | undefined>(undefined);
  const pending = useRef<AbortController | undefined>(undefined);
  useEffect(() => () => pending.current?.abort(), []);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (sending) return;
    setError(undefined);
    if (!/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(name)) {
      setProblem('Use a signal name of 1–128 characters, starting with a letter or digit; then letters, digits, or _ . : - .'); return;
    }
    let body: unknown;
    try { body = JSON.parse(payload); } catch { setProblem('Enter a valid JSON object.'); return; }
    if (body === null || typeof body !== 'object' || Array.isArray(body)) { setProblem('Enter a JSON object.'); return; }
    const encoded = JSON.stringify(body);
    if (new TextEncoder().encode(encoded).length > 256 * 1024) { setProblem('The payload must be at most 256 KiB.'); return; }
    setProblem(undefined);
    const fingerprint = `${name}\n${encoded}`;
    if (previous.current?.fingerprint !== fingerprint) previous.current = { fingerprint, key: crypto.randomUUID() };
    const controller = new AbortController();
    pending.current = controller;
    const token = session.token();
    const current = () => !controller.signal.aborted && session.token() === token;
    setSending(true);
    try {
      const result = await api.post<components['schemas']['SignalResult']>(path`/runs/${runId}/signals/${name}`, {
        body, key: previous.current.key, signal: controller.signal,
      });
      if (!current()) return;
      announce(`Signal delivered to ${result.deliveredTo.length} waiting steps.`);
      onSent(); onClose();
    } catch (cause) {
      if (current()) setError(cause);
    } finally {
      if (current()) setSending(false);
    }
  };

  return <form onSubmit={(event) => { void submit(event); }}>
    <p>This sends the payload to every waiting step in this run that expects the signal. Those steps may resume workflow execution.</p>
    <p>Signals are not buffered. If no step is waiting for this name, the platform refuses the signal. Your identity and the delivery are audited.</p>
    <Field label="Signal name"><input autoComplete="off" spellCheck={false} value={name} maxLength={128} disabled={sending} onChange={(event) => setName(event.target.value)} /></Field>
    <Field label="Payload (JSON object)" hint="At most 256 KiB. Do not include credentials or secrets."><textarea rows={6} autoComplete="off" spellCheck={false} value={payload} disabled={sending} onChange={(event) => setPayload(event.target.value)} /></Field>
    {problem ? <p role="alert" className="error">{problem}</p> : null}
    {error ? <ErrorState error={error} what="Sending signal" /> : null}
    <div className="toolbar"><Button type="submit" disabled={sending}>{sending ? 'Sending…' : 'Confirm signal'}</Button><Button onClick={onClose}>Close</Button></div>
  </form>;
}
