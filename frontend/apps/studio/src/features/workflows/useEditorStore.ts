import { useState, useSyncExternalStore } from 'react';
import type { IrDocument } from '../../model/ir';
import { canRedo, canUndo, commit, redo as redoHistory, startHistory, undo as undoHistory } from './history';
import type { History } from './history';

export interface EditorSnapshot {
  history: History<IrDocument>;
  selectedId: string | undefined;
}

export interface EditOptions {
  /** Edits with the same key close together are one step to undo (typing in one field). */
  key?: string;
  /** Select this node after the change; `null` clears the selection. */
  select?: string | null;
}

/**
 * The document being edited, its undo history and the selected node. Every change goes through
 * `edit`, which is synchronous (two changes in one event see each other) and lets the caller's
 * function throw to refuse: nothing is then changed.
 */
export class EditorStore {
  #snapshot: EditorSnapshot;
  readonly #listeners = new Set<() => void>();

  constructor(initial: IrDocument) {
    this.#snapshot = { history: startHistory(initial), selectedId: undefined };
  }

  getSnapshot = (): EditorSnapshot => this.#snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  #publish(next: EditorSnapshot): void {
    this.#snapshot = next;
    for (const listener of [...this.#listeners]) listener();
  }

  /** The document right now, not as of the last render. */
  doc(): IrDocument {
    return this.#snapshot.history.present;
  }

  /** Returns whether the document changed. */
  edit(change: (doc: IrDocument) => IrDocument, options: EditOptions = {}): boolean {
    const before = this.#snapshot;
    const next = change(before.history.present);
    const selectedId = options.select === undefined ? before.selectedId : (options.select ?? undefined);
    if (next === before.history.present && selectedId === before.selectedId) return false;
    this.#publish({ history: commit(before.history, next, { key: options.key }), selectedId });
    return next !== before.history.present;
  }

  select(id: string | undefined): void {
    if (this.#snapshot.selectedId !== id) this.#publish({ ...this.#snapshot, selectedId: id });
  }

  undo(): boolean {
    const history = undoHistory(this.#snapshot.history);
    if (history === this.#snapshot.history) return false;
    this.#publish({ ...this.#snapshot, history });
    return true;
  }

  redo(): boolean {
    const history = redoHistory(this.#snapshot.history);
    if (history === this.#snapshot.history) return false;
    this.#publish({ ...this.#snapshot, history });
    return true;
  }

  /** A document that replaces everything, with no history to go back to (the platform's copy was taken). */
  reset(doc: IrDocument): void {
    this.#publish({ history: startHistory(doc), selectedId: undefined });
  }
}

export function useEditorStore(initial: IrDocument) {
  const [store] = useState(() => new EditorStore(initial));
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const doc = snapshot.history.present;
  // A selection that no longer points at a node (deleted, renamed, undone) selects nothing.
  const selectedId = snapshot.selectedId !== undefined && doc.nodes.some((n) => n.id === snapshot.selectedId) ? snapshot.selectedId : undefined;
  return { store, doc, selectedId, canUndo: canUndo(snapshot.history), canRedo: canRedo(snapshot.history) };
}
