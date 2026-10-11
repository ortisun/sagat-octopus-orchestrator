import { isApiError } from '../../api/errors';
import type { Script, UpdateScriptRequest } from './api';
import { checkSchemaText, schemaToText } from './jsonText';
import { LIMITS } from './languages';
import type { Timers } from './polling';
import { formatBytes, utf8Length } from './text';

/** What the author edits in a draft. The lockfile is not edited by the Studio and is never sent. */
export interface DraftFields {
  source: string;
  inputSchema: string;
  outputSchema: string;
}

export type DraftField = keyof DraftFields;

export const FIELD_LABEL: Record<DraftField, string> = {
  source: 'The source',
  inputSchema: 'The input schema',
  outputSchema: 'The output schema',
};

/** A reason part of the draft is not sent. Seen before the platform is asked. */
export interface DraftProblem {
  field: DraftField;
  message: string;
}

export type SaveState =
  /** Everything the author typed is on the platform. */
  | { kind: 'saved' }
  /** Typed, waiting for the pause that starts a save. */
  | { kind: 'dirty' }
  | { kind: 'saving' }
  /** Someone else saved first (`409`). Nothing more is sent until the author decides. */
  | { kind: 'conflict'; currentRevision: number | undefined }
  /** The platform refused the draft (`422`); the same text would be refused again, so it waits for an edit. */
  | { kind: 'rejected'; error: unknown }
  /** The save failed; `retryInMs` is set while the Studio will try again by itself. */
  | { kind: 'error'; error: unknown; retryInMs: number | undefined };

export interface DraftSnapshot {
  fields: DraftFields;
  /** The revision of the platform's draft that these edits are based on. */
  revision: number;
  /** When the platform's draft last changed, by the platform's clock. */
  updatedAt: string;
  state: SaveState;
  /** Whether the platform's draft differs from what the author sees. */
  unsaved: boolean;
  /** Parts of the draft that cannot be sent as they are. */
  problems: DraftProblem[];
}

export type Update = (body: UpdateScriptRequest, key: string) => Promise<Script>;

export interface DraftOptions {
  script: Script;
  update: Update;
  /** The pause after the last keystroke before a save starts. */
  debounceMs?: number;
  /** The pauses before the automatic retries of a failed save; when they are used up, the author decides. */
  retryDelaysMs?: readonly number[];
  timers?: Timers;
  newKey?: () => string;
}

interface Saved {
  revision: number;
  updatedAt: string;
  fields: DraftFields;
}

function fieldsOf(script: Script): DraftFields {
  return { source: script.source, inputSchema: schemaToText(script.inputSchema), outputSchema: schemaToText(script.outputSchema) };
}

const FIELDS: readonly DraftField[] = ['source', 'inputSchema', 'outputSchema'];

const browserTimers: Timers = {
  setTimeout: (callback, ms) => globalThis.setTimeout(callback, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
  now: () => Date.now(),
};

/**
 * The autosave of a draft. It owns the author's text, the revision it is based on and the one save
 * that may be in flight, and it is what makes the rules of the draft hold (D146 of feature 007 and
 * `updateScript` of the API):
 * - a save carries the revision the author read and only the fields that changed;
 * - a `409` stops the autosave, keeps the author's text and waits for a decision: nothing is
 *   overwritten silently;
 * - a retry of the same save keeps its idempotency key, so a save whose answer was lost is not
 *   applied twice;
 * - a text the platform would refuse (a schema that is not JSON, a source over the limit) is not
 *   sent, and the author is told why.
 * It is not a React thing: a component subscribes to it.
 */
export class DraftSaver {
  readonly #update: Update;
  readonly #debounceMs: number;
  readonly #retryDelays: readonly number[];
  readonly #timers: Timers;
  readonly #newKey: () => string;
  readonly #listeners = new Set<() => void>();
  #server: Saved;
  #fields: DraftFields;
  #state: SaveState = { kind: 'saved' };
  #snapshot: DraftSnapshot;
  #timer: unknown;
  #inflight: Promise<void> | undefined;
  #attempt: { fingerprint: string; key: string } | undefined;
  #retries = 0;

  constructor(options: DraftOptions) {
    this.#update = options.update;
    this.#debounceMs = options.debounceMs ?? 1500;
    this.#retryDelays = options.retryDelaysMs ?? [2000, 6000, 15_000];
    this.#timers = options.timers ?? browserTimers;
    this.#newKey = options.newKey ?? (() => crypto.randomUUID());
    this.#fields = fieldsOf(options.script);
    this.#server = { revision: options.script.revision, updatedAt: options.script.updatedAt, fields: { ...this.#fields } };
    this.#snapshot = this.#build();
  }

  /** For `useSyncExternalStore`. The last unsubscribe is the page being left: pending edits are sent then. */
  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1 && this.#snapshot.unsaved) this.#resume();
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) this.#leave();
    };
  };

  getSnapshot = (): DraftSnapshot => this.#snapshot;

  /** The author typed. */
  set(field: DraftField, value: string): void {
    if (this.#fields[field] === value) return;
    this.#fields = { ...this.#fields, [field]: value };
    // A refusal is about the text that was sent: new text deserves a new try.
    if (this.#state.kind === 'rejected') this.#state = { kind: 'saved' };
    this.#retries = 0;
    this.#plan();
  }

  /**
   * Sends everything that is pending now and says whether the platform has it all. False when a
   * conflict, a refusal, a failure or a text that cannot be sent is in the way: the page then says
   * why, and a test run or a publication (which use the platform's copy) must not go on.
   */
  async save(): Promise<boolean> {
    this.#clearTimer();
    if (this.#state.kind === 'conflict') return false;
    this.#retries = 0;
    for (let round = 0; round < 6; round += 1) {
      if (this.#inflight !== undefined) {
        await this.#inflight;
        if (this.#stuck()) return false;
      }
      if (Object.keys(this.#changes().body).length === 0) break;
      await this.#start();
      if (this.#stuck()) return false;
    }
    return !this.#snapshot.unsaved && this.#state.kind === 'saved';
  }

  /** The author chose the platform's copy: their own text is dropped. */
  adoptTheirs(theirs: Script): void {
    this.#clearTimer();
    this.#server = { revision: theirs.revision, updatedAt: theirs.updatedAt, fields: fieldsOf(theirs) };
    this.#fields = { ...this.#server.fields };
    this.#attempt = undefined;
    this.#retries = 0;
    this.#state = { kind: 'saved' };
    this.#publish();
  }

  /**
   * The author chose to keep their changes over the platform's copy, which they have seen: what
   * they changed stays, what they did not touch becomes the platform's, and the next save is based
   * on the platform's revision. Nothing is sent until `save()` or the next edit.
   */
  keepMine(theirs: Script): void {
    this.#clearTimer();
    const theirFields = fieldsOf(theirs);
    const next = { ...this.#fields };
    for (const field of FIELDS) {
      if (this.#fields[field] === this.#server.fields[field]) next[field] = theirFields[field];
    }
    this.#fields = next;
    this.#server = { revision: theirs.revision, updatedAt: theirs.updatedAt, fields: theirFields };
    this.#attempt = undefined;
    this.#retries = 0;
    this.#state = { kind: 'saved' };
    this.#plan();
  }

  /**
   * The page found out, some other way than a refused save, that the platform's draft is not the one
   * these edits are based on (a publication checks before it asks the platform to publish *its* copy).
   */
  reportConflict(currentRevision: number | undefined): void {
    this.#clearTimer();
    this.#state = { kind: 'conflict', currentRevision };
    this.#attempt = undefined;
    this.#publish();
  }

  /** What the author changed and the platform's copy has not (for showing a diff). */
  changedFields(): DraftField[] {
    return FIELDS.filter((field) => this.#fields[field] !== this.#server.fields[field]);
  }

  // ------------------------------------------------------------------ internals

  /** The parts of the draft to send, and the parts that cannot be sent. */
  #changes(): { body: Partial<UpdateScriptRequest>; problems: DraftProblem[]; sent: Partial<DraftFields> } {
    const body: Partial<UpdateScriptRequest> = {};
    const sent: Partial<DraftFields> = {};
    const problems: DraftProblem[] = [];
    const source = this.#fields.source;
    if (source !== this.#server.fields.source) {
      const bytes = utf8Length(source);
      if (source.trim() === '') problems.push({ field: 'source', message: 'The source is empty; the platform needs at least one character.' });
      else if (source.includes('\0')) problems.push({ field: 'source', message: 'The source contains a NUL character; the platform refuses it.' });
      else if (bytes > LIMITS.sourceBytes) {
        problems.push({ field: 'source', message: `The source is ${formatBytes(bytes)}; the limit is ${formatBytes(LIMITS.sourceBytes)}.` });
      } else {
        body.source = source;
        sent.source = source;
      }
    }
    for (const field of ['inputSchema', 'outputSchema'] as const) {
      if (this.#fields[field] === this.#server.fields[field]) continue;
      const check = checkSchemaText(this.#fields[field]);
      if (check.kind === 'invalid') problems.push({ field, message: `${FIELD_LABEL[field]}: ${check.message}` });
      else {
        body[field] = check.kind === 'empty' ? null : check.value;
        sent[field] = this.#fields[field];
      }
    }
    return { body, problems, sent };
  }

  #build(): DraftSnapshot {
    const { problems } = this.#changes();
    return {
      fields: this.#fields,
      revision: this.#server.revision,
      updatedAt: this.#server.updatedAt,
      state: this.#state,
      unsaved: FIELDS.some((field) => this.#fields[field] !== this.#server.fields[field]),
      problems,
    };
  }

  #publish(): void {
    this.#snapshot = this.#build();
    for (const listener of [...this.#listeners]) listener();
  }

  #clearTimer(): void {
    if (this.#timer !== undefined) this.#timers.clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  /** Decides what happens after a change: nothing, a wait for the pause, or nothing to send. */
  #plan(): void {
    this.#clearTimer();
    const kind = this.#state.kind;
    if (kind === 'saving' || kind === 'conflict' || kind === 'rejected') {
      // The save in flight plans again when it ends; the others wait for the author.
      this.#publish();
      return;
    }
    const unsaved = FIELDS.some((field) => this.#fields[field] !== this.#server.fields[field]);
    if (!unsaved) this.#state = { kind: 'saved' };
    else if (kind !== 'error') this.#state = { kind: 'dirty' };
    this.#arm();
    this.#publish();
  }

  /** A save after the pause, if there is anything the platform would take. */
  #arm(): void {
    if (Object.keys(this.#changes().body).length === 0) return;
    this.#timer = this.#timers.setTimeout(() => {
      this.#timer = undefined;
      void this.#start();
    }, this.#debounceMs);
  }

  /** Subscribed again (or for the first time) with edits pending: wait for the pause again. */
  #resume(): void {
    this.#clearTimer();
    if (this.#state.kind === 'dirty' || this.#state.kind === 'error') this.#arm();
  }

  /** A failure the author has to look at: a retry will not change it. */
  #stuck(): boolean {
    const kind = this.#state.kind;
    return kind === 'conflict' || kind === 'rejected' || kind === 'error';
  }

  #start(): Promise<void> {
    if (this.#inflight !== undefined) return this.#inflight;
    const run = this.#send().finally(() => {
      if (this.#inflight === run) this.#inflight = undefined;
    });
    this.#inflight = run;
    return run;
  }

  async #send(): Promise<void> {
    this.#clearTimer();
    const { body, sent } = this.#changes();
    if (Object.keys(body).length === 0) {
      this.#plan();
      return;
    }
    const revision = this.#server.revision;
    const request: UpdateScriptRequest = { revision, ...body };
    // The same text on the same revision is the same intent: after a lost answer the platform
    // recognises the key and answers again instead of applying it twice.
    const fingerprint = JSON.stringify(request);
    const key = this.#attempt?.fingerprint === fingerprint ? this.#attempt.key : this.#newKey();
    this.#attempt = { fingerprint, key };
    this.#state = { kind: 'saving' };
    this.#publish();
    try {
      const saved = await this.#update(request, key);
      this.#server = { revision: saved.revision, updatedAt: saved.updatedAt, fields: { ...this.#server.fields, ...sent } };
      this.#attempt = undefined;
      this.#retries = 0;
      this.#state = { kind: 'saved' };
    } catch (error) {
      this.#failed(error);
      return;
    }
    this.#plan();
  }

  #failed(error: unknown): void {
    if (isApiError(error) && error.status === 409) {
      const current = error.details?.['currentRevision'];
      this.#state = { kind: 'conflict', currentRevision: typeof current === 'number' ? current : undefined };
      this.#attempt = undefined;
    } else if (isApiError(error) && (error.status === 422 || error.status === 400)) {
      this.#state = { kind: 'rejected', error };
      this.#attempt = undefined;
    } else if (isApiError(error) && (error.retryable || error.code === 'TRANSPORT') && this.#retries < this.#retryDelays.length) {
      const retryInMs = this.#retryDelays[this.#retries] ?? 0;
      this.#retries += 1;
      this.#state = { kind: 'error', error, retryInMs };
      this.#timer = this.#timers.setTimeout(() => {
        this.#timer = undefined;
        void this.#start();
      }, retryInMs);
    } else {
      this.#state = { kind: 'error', error, retryInMs: undefined };
    }
    this.#publish();
  }

  /** The page was left: what is pending is sent once, if it can be, and nothing is scheduled any more. */
  #leave(): void {
    this.#clearTimer();
    const idle = this.#state.kind === 'dirty' || this.#state.kind === 'error';
    if (idle && Object.keys(this.#changes().body).length > 0) void this.#start();
  }
}
