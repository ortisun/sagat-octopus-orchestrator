import { useSyncExternalStore } from 'react';
import { ApiError, isApiError } from '../../api/errors';
import type { Run } from '../../api/types';
import type { SessionStore } from '../../auth/session';

export interface DatabaseIntent<T> { key: string; body: T }
type State = { phase: 'idle' } | { phase: 'sending' } | { phase: 'failed'; error: unknown; refused: boolean } | { phase: 'accepted'; run: Run };
type Execute<T> = (intent: DatabaseIntent<T>, signal: AbortSignal) => Promise<Run>;

/** Survives page/namespace navigation; disposed only when the owning session ends. */
class Command<T> {
  constructor(private readonly mayRetain: () => boolean) {}
  state: State = { phase: 'idle' };
  intent: DatabaseIntent<T> | undefined;
  private uncertain = false;
  private disposed = false;
  private controller: AbortController | undefined;
  private listeners = new Set<() => void>();
  snapshot = () => this.state;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(state: State) { this.state = state; for (const listener of this.listeners) listener(); }
  dispose() { this.disposed = true; this.controller?.abort(); this.intent = undefined; this.update({ phase: 'idle' }); }
  reset = () => {
    if (this.state.phase !== 'accepted' && !(this.state.phase === 'failed' && this.state.refused)) return;
    this.intent = undefined; this.uncertain = false; this.update({ phase: 'idle' });
  };
  async send(execute: Execute<T>, body?: T) {
    if (this.disposed || this.state.phase === 'sending' || this.state.phase === 'accepted') return;
    if (!this.intent) {
      if (body === undefined) return;
      if (!this.mayRetain() || new TextEncoder().encode(JSON.stringify(body)).length > 2_097_152) {
        this.update({ phase: 'failed', refused: true, error: new ApiError({ code: 'QUOTA_EXCEEDED', status: 413, message: 'At most eight pending database requests can be retained in this session, each up to 2 MiB. Recover existing requests or reduce this definition.' }) });
        return;
      }
      this.intent = { key: crypto.randomUUID(), body: structuredClone(body) };
    }
    const controller = new AbortController(); this.controller = controller; this.update({ phase: 'sending' });
    try {
      const run = await execute(this.intent, controller.signal);
      if (!run || typeof run.runId !== 'string' || run.runId === '') throw new Error('Missing run receipt');
      if (!this.disposed) { this.intent = undefined; this.update({ phase: 'accepted', run }); }
    } catch (error) {
      // A later rate/ACL refusal cannot prove an earlier ambiguous attempt never committed.
      const refusal = isApiError(error) && [400, 403, 404, 413, 422, 429].includes(error.status);
      if (!refusal) this.uncertain = true;
      if (!this.disposed) this.update({ phase: 'failed', error, refused: refusal && !this.uncertain });
    }
  }
}
const sessions = new WeakMap<SessionStore, Map<string, Command<unknown>>>();
function commandFor<T>(session: SessionStore, scope: string): Command<T> {
  let commands = sessions.get(session);
  if (!commands) {
    commands = new Map(); sessions.set(session, commands);
    let identity = session.snapshot().info;
    const owned = commands;
    session.subscribe(() => {
      const current = session.snapshot().info;
      if (!current || current.tenant !== identity?.tenant || current.subject !== identity?.subject) {
        for (const command of owned.values()) command.dispose();
        owned.clear();
      }
      identity = current;
    });
  }
  let command = commands.get(scope);
  if (!command) { const owned = commands; command = new Command(() => [...owned.values()].filter(c => c.intent !== undefined).length < 8); commands.set(scope, command); }
  return command as Command<T>;
}

/** One immutable session-memory intent per scope. Ambiguous acknowledgements only resend it. */
export function useDatabaseCommand<T>(execute: Execute<T>, session: SessionStore, scope: string) {
  const command = commandFor<T>(session, scope);
  const state = useSyncExternalStore(command.subscribe, command.snapshot);
  return { state, body: command.intent?.body, locked: state.phase !== 'idle', send: (body?: T) => command.send(execute, body), reset: command.reset };
}
