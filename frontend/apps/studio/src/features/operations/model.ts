import type { Drain, Worker } from './api';

export interface WorkerIdentity {
  subject: string;
  sessions: Worker[];
  drain?: Drain;
  draining: boolean;
}

export function groupWorkerIdentities(workers: Worker[], drains: Drain[]): WorkerIdentity[] {
  const bySubject = new Map<string, WorkerIdentity>();
  for (const worker of workers) {
    const identity = bySubject.get(worker.subject) ?? { subject: worker.subject, sessions: [], draining: false };
    identity.sessions.push(worker);
    identity.draining ||= worker.draining;
    bySubject.set(worker.subject, identity);
  }
  for (const drain of drains) {
    const identity = bySubject.get(drain.subject) ?? { subject: drain.subject, sessions: [], draining: true };
    identity.drain = drain;
    identity.draining = true;
    bySubject.set(drain.subject, identity);
  }
  return [...bySubject.values()].sort((left, right) => left.subject.localeCompare(right.subject));
}

export function alertKind(kind: string): string {
  const words = kind.toLowerCase().replaceAll('_', ' ');
  return `${words.charAt(0).toUpperCase()}${words.slice(1)}`;
}

/** Links only same-origin references and HTTPS runbooks; every other value remains visible as text. */
export function safeRunbookHref(reference: string): string | undefined {
  try {
    if (reference.startsWith('/') && !reference.startsWith('//') && !reference.includes('\\')) {
      const target = new URL(reference, window.location.origin);
      if (target.origin === window.location.origin) return target.href;
    }
    const target = new URL(reference);
    if (target.protocol === 'https:') return target.href;
  } catch {
    return undefined;
  }
  return undefined;
}
