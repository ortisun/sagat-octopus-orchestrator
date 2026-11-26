/*
 * The activation window (design 044, Technical Plan 4.3, Architecture decision 0104). Let S(k) be the journal sequence of
 * the `workflow_task.started` of the workflow task that produced completion k (the last start before that completion:
 * the starts of tasks that failed or timed out are skipped). Activation k delivers the visible events with a sequence in
 * (S(k − 1), S(k)], in journal order, with S(0) = 0; the time of S(k) is the activation's time. A live worker and a
 * replaying worker cut the history the same way.
 */
import type { BeginActivation, CommandKind, ExpectedRecord, HistoryEvent } from './context.js';

/** One completed activation of a history, with what its completion recorded. */
export interface ReplayActivation {
  /** The visible events of the window, in journal order. */
  window: HistoryEvent[];
  /** The `workflow_task.started` at S(k). */
  started: Extract<HistoryEvent, { kind: 'workflow_task.started' }>;
  /** The completion's command records, with side-effect values. */
  records: ExpectedRecord[];
  /** The deferral that completion recorded, if any. */
  deferral?: { kind: CommandKind; target: string };
}

/** The window of an activation that no completion answered yet: the live one. */
export interface OpenWindow {
  window: HistoryEvent[];
  started: Extract<HistoryEvent, { kind: 'workflow_task.started' }> | undefined;
  /** S(k − 1): the start of the last completed activation, or 0. */
  windowStart: number;
}

export class HistoryOrderError extends Error {
  constructor(message: string) { super(message); this.name = 'HistoryOrderError'; }
}

interface PendingActivation {
  window: HistoryEvent[];
  started: ReplayActivation['started'];
  completedSeq: number;
  records: ExpectedRecord[];
  deferral?: ReplayActivation['deferral'];
}

/**
 * Cuts a history into activations as its events arrive (page by page). An activation is handed out once the next
 * completion shows that its records are complete, or when the history is flushed.
 */
export class ActivationCutter {
  #buffer: HistoryEvent[] = [];
  #windowStart = 0;
  #lastSeq = 0;
  #pending: PendingActivation | undefined;

  /** The sequence after which the open window starts: S of the last completion seen. */
  get windowStart(): number { return this.#windowStart; }

  /** Adds events in journal order; answers the activations they completed. */
  push(events: Iterable<HistoryEvent>): ReplayActivation[] {
    const done: ReplayActivation[] = [];
    for (const event of events) {
      if (!Number.isSafeInteger(event.seq) || event.seq <= this.#lastSeq) {
        throw new HistoryOrderError(`history event ${String(event.seq)} is not after ${this.#lastSeq}`);
      }
      this.#lastSeq = event.seq;
      if (event.kind === 'command.record' || event.kind === 'marker.recorded') {
        // A command record belongs to the completion that wrote it, the last one before it.
        if (!this.#pending) throw new HistoryOrderError(`command record ${event.seq} follows no completion`);
        const record: ExpectedRecord = event.kind === 'command.record'
          ? { commandNumber: event.commandNumber, kind: event.commandKind, target: event.target }
          : { commandNumber: event.commandNumber, kind: event.markerKind === 'patch' ? 'marker_patch' : 'marker_side_effect', target: event.markerId };
        if (event.kind === 'marker.recorded' && 'value' in event) record.value = event.value;
        if (event.redacted) record.redacted = true;
        this.#pending.records.push(record);
        this.#buffer.push(event);
        continue;
      }
      if (event.kind === 'workflow_task.completed') {
        if (this.#pending) done.push(this.#release());
        let startedIndex = -1;
        for (let i = this.#buffer.length - 1; i >= 0; i--) {
          if (this.#buffer[i]!.kind === 'workflow_task.started') { startedIndex = i; break; }
        }
        if (startedIndex < 0) throw new HistoryOrderError(`completion ${event.seq} follows no workflow_task.started`);
        const started = this.#buffer[startedIndex] as ReplayActivation['started'];
        const pending: PendingActivation = {
          window: this.#buffer.slice(0, startedIndex + 1),
          started,
          completedSeq: event.seq,
          records: [],
        };
        if (event.deferral) pending.deferral = event.deferral;
        this.#pending = pending;
        this.#buffer = [...this.#buffer.slice(startedIndex + 1), event];
        continue;
      }
      this.#buffer.push(event);
    }
    return done;
  }

  /**
   * Ends the input: answers the last completed activation (if any) and the open window after it, cut at its last
   * `workflow_task.started` (the events after it belong to the next activation).
   */
  flush(): { activations: ReplayActivation[]; open: OpenWindow } {
    const activations = this.#pending ? [this.#release()] : [];
    let started: OpenWindow['started'];
    let end = this.#buffer.length;
    for (let i = this.#buffer.length - 1; i >= 0; i--) {
      if (this.#buffer[i]!.kind === 'workflow_task.started') { started = this.#buffer[i] as OpenWindow['started']; end = i + 1; break; }
    }
    return { activations, open: { window: this.#buffer.slice(0, end), started, windowStart: this.#windowStart } };
  }

  #release(): ReplayActivation {
    const pending = this.#pending!;
    this.#pending = undefined;
    this.#windowStart = pending.started.seq;
    const activation: ReplayActivation = { window: pending.window, started: pending.started, records: pending.records };
    if (pending.deferral) activation.deferral = pending.deferral;
    return activation;
  }
}

/** What the runtime needs at the start of an activation, from its window. */
export function beginOf(window: HistoryEvent[], started: ReplayActivation['started'], replaying: boolean): BeginActivation {
  const begin: BeginActivation = {
    replaying,
    atUs: started.atUs ?? 0,
    continueAsNewSuggested: started.continueAsNewSuggested === true,
    historyLimitReached: started.historyLimitReached === true,
    windowSignals: window.flatMap((event) => (event.kind === 'signal.buffered' ? [{ seq: event.seq, name: event.name }] : [])),
  };
  return begin;
}

/** The events of a window the function must see: bookkeeping (records, workflow-task events) is left out. */
export function appliedEvents(window: HistoryEvent[]): HistoryEvent[] {
  return window.filter((event) => event.kind !== 'command.record' && event.kind !== 'marker.recorded'
    && event.kind !== 'workflow_task.started' && event.kind !== 'workflow_task.completed');
}
