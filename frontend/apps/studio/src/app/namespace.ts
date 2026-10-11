const KEY = 'studio.namespace';

/** The namespace the user works in. A slug is not a secret, so it may survive a reload. */
export class NamespaceStore {
  #current: string | undefined;
  readonly #listeners = new Set<() => void>();

  constructor() {
    try {
      const saved = globalThis.localStorage?.getItem(KEY);
      if (saved !== null && saved !== undefined && NamespaceStore.valid(saved)) this.#current = saved;
    } catch {
      // Storage may be blocked; the choice then lasts until the page is closed.
    }
  }

  static valid(slug: string): boolean {
    return /^[a-z0-9][a-z0-9-]{0,62}$/.test(slug);
  }

  current = (): string | undefined => this.#current;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  set(slug: string | undefined): void {
    if (slug !== undefined && !NamespaceStore.valid(slug)) return;
    if (slug === this.#current) return;
    this.#current = slug;
    try {
      if (slug === undefined) globalThis.localStorage?.removeItem(KEY);
      else globalThis.localStorage?.setItem(KEY, slug);
    } catch {
      // Not remembering is acceptable.
    }
    for (const listener of [...this.#listeners]) listener();
  }
}
