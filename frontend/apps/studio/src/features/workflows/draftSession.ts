import { isApiError } from '../../api/errors';
import type { Endpoints } from '../../api/endpoints';
import type { Workflow } from '../../api/types';
import { canonical } from '../../model/ir';

export type SavePhase = 'idle' | 'saving' | 'conflict' | 'failed';

export interface SaveSnapshot {
  phase: SavePhase;
  /** The draft revision this editor holds; the next save is compare-and-swap on it. */
  revision: number;
  /** The document the platform has as the draft, as far as this editor knows (canonical form). */
  savedKey: string;
  /** The document in the editor (canonical form). */
  currentKey: string;
  /** The editor holds something the platform does not. */
  dirty: boolean;
  /** Why the last save failed. */
  error: unknown;
  /** The document whose save failed, so an autosave does not retry the very same one in a loop. */
  failedKey: string | undefined;
  /** The platform's copy, when somebody else saved first. */
  theirs: Workflow | undefined;
}

/** `saved`: nothing is left unsaved. */
export type SaveOutcome = 'saved' | 'conflict' | 'failed';

type SaveApi = Pick<Endpoints, 'updateDraft' | 'getWorkflow'>;

/** Errors after which the platform may or may not have stored the document. */
function outcomeUnknown(error: unknown): boolean {
  return isApiError(error) && (error.code === 'TRANSPORT' || error.code === 'UNAVAILABLE' || error.code === 'INTERNAL');
}

/**
 * The saving of one draft (D146). It owns three things the screen must not get wrong:
 *
 * - the revision: every save is compare-and-swap on the revision the editor last knew, and a save
 *   that answers sets the next one from the answer;
 * - the idempotency key: a retry of the same save (same revision, same document) keeps its key, so
 *   a save whose answer was lost is not applied twice, and a different document gets a new one;
 * - conflicts: a 409 never overwrites anything. If the platform's copy is the very document being
 *   saved (another tab, or an earlier save whose answer was lost) the revision is taken and nothing
 *   is lost; otherwise the editor stops and keeps both copies for the person to decide.
 */
export class DraftSession {
  readonly #ep: SaveApi;
  readonly #id: string;
  readonly #newKey: () => string;
  #wire: Record<string, unknown>;
  #snapshot: SaveSnapshot;
  #attempt: { signature: string; key: string } | undefined;
  /** A document whose save ended without an answer: the platform may hold it. */
  #unknown: string | undefined;
  #loop: Promise<SaveOutcome> | undefined;
  readonly #listeners = new Set<() => void>();

  constructor(options: { ep: SaveApi; id: string; revision: number; wire: Record<string, unknown>; newKey?: () => string }) {
    this.#ep = options.ep;
    this.#id = options.id;
    this.#newKey = options.newKey ?? (() => crypto.randomUUID());
    this.#wire = options.wire;
    const key = canonical(options.wire);
    this.#snapshot = { phase: 'idle', revision: options.revision, savedKey: key, currentKey: key, dirty: false, error: undefined, failedKey: undefined, theirs: undefined };
  }

  getSnapshot = (): SaveSnapshot => this.#snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  #set(change: Partial<SaveSnapshot>): void {
    const next = { ...this.#snapshot, ...change };
    next.dirty = next.currentKey !== next.savedKey;
    this.#snapshot = next;
    for (const listener of [...this.#listeners]) listener();
  }

  /** The document in the editor changed. */
  update(wire: Record<string, unknown>, key: string = canonical(wire)): void {
    this.#wire = wire;
    if (key !== this.#snapshot.currentKey) this.#set({ currentKey: key });
  }

  /**
   * Saves what is unsaved. Resolves when nothing is left unsaved, or the save cannot go on. Callers
   * that come while a save is under way join it: there is one save at a time, and it looks again at
   * the document after each, so what was typed meanwhile goes too.
   */
  save(): Promise<SaveOutcome> {
    if (this.#loop) return this.#loop;
    const loop = this.#run().finally(() => {
      this.#loop = undefined;
    });
    this.#loop = loop;
    return loop;
  }

  async #run(): Promise<SaveOutcome> {
    for (let round = 0; round < 4; round++) {
      const now = this.#snapshot;
      if (now.phase === 'conflict') return 'conflict';
      if (!now.dirty) return 'saved';
      const outcome = await this.#attemptSave();
      if (outcome !== 'saved') return outcome;
    }
    // The author keeps typing: what is left goes with the next autosave.
    return 'saved';
  }

  async #attemptSave(): Promise<SaveOutcome> {
    const wire = this.#wire;
    const bodyKey = this.#snapshot.currentKey;
    const revision = this.#snapshot.revision;
    const signature = `${revision}\u0000${bodyKey}`;
    const key = this.#attempt?.signature === signature ? this.#attempt.key : this.#newKey();
    this.#attempt = { signature, key };
    this.#set({ phase: 'saving', error: undefined });
    try {
      const saved = await this.#ep.updateDraft(this.#id, revision, wire, key);
      this.#attempt = undefined;
      this.#unknown = undefined;
      this.#set({ phase: 'idle', revision: saved.draftRevision, savedKey: bodyKey, failedKey: undefined, error: undefined, theirs: undefined });
      return 'saved';
    } catch (error) {
      if (isApiError(error) && error.code === 'CONFLICT') return await this.#conflicted(bodyKey);
      if (outcomeUnknown(error)) this.#unknown = bodyKey;
      this.#set({ phase: 'failed', error, failedKey: bodyKey });
      return 'failed';
    }
  }

  async #conflicted(bodyKey: string): Promise<SaveOutcome> {
    let theirs: Workflow;
    try {
      theirs = await this.#ep.getWorkflow(this.#id);
    } catch (error) {
      this.#set({ phase: 'failed', error, failedKey: bodyKey });
      return 'failed';
    }
    const theirKey = canonical(theirs.ir);
    if (theirKey === bodyKey || theirKey === this.#unknown) {
      // Not somebody else's work: the same document, saved by an earlier attempt whose answer never arrived.
      this.#attempt = undefined;
      this.#unknown = undefined;
      this.#set({ phase: 'idle', revision: theirs.draftRevision, savedKey: theirKey, failedKey: undefined, error: undefined, theirs: undefined });
      return 'saved';
    }
    this.#set({ phase: 'conflict', theirs, error: undefined });
    return 'conflict';
  }

  /** The author takes the platform's copy: the editor now holds it, saved. */
  adopt(theirs: Workflow, wire: Record<string, unknown>): void {
    const key = canonical(wire);
    this.#wire = wire;
    this.#attempt = undefined;
    this.#unknown = undefined;
    this.#set({ phase: 'idle', revision: theirs.draftRevision, savedKey: key, currentKey: key, failedKey: undefined, error: undefined, theirs: undefined });
  }

  /** The author keeps their copy and saves over the platform's, on top of the revision it has now. */
  overwrite(theirs: Workflow): Promise<SaveOutcome> {
    this.#attempt = undefined;
    this.#unknown = undefined;
    this.#set({ phase: 'idle', revision: theirs.draftRevision, failedKey: undefined, error: undefined, theirs: undefined });
    return this.save();
  }

  /** Somebody else's copy turned up somewhere else than a save (a publish was refused because the draft moved on). */
  conflictWith(theirs: Workflow): void {
    this.#set({ phase: 'conflict', theirs, error: undefined });
  }

  /** Back from a conflict dialog that was dismissed: the next save looks at the platform again. */
  clearFailure(): void {
    this.#set({ phase: 'idle', failedKey: undefined, error: undefined });
  }
}
