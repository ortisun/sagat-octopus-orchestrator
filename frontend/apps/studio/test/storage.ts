/**
 * An in-memory `localStorage`. Some runtimes (recent Node) shadow the one jsdom brings with a stub
 * that is not usable, and the Studio's view preferences live there, so a test that needs them
 * installs this one. `broken()` makes every access throw, as a blocked or full storage does.
 */
export class MemoryStorage implements Storage {
  #items = new Map<string, string>();
  #broken = false;

  get length(): number {
    return this.#items.size;
  }

  broken(on = true): void {
    this.#broken = on;
  }

  #check(): void {
    if (this.#broken) throw new DOMException('storage is blocked', 'SecurityError');
  }

  clear(): void {
    this.#check();
    this.#items.clear();
  }

  getItem(key: string): string | null {
    this.#check();
    return this.#items.get(key) ?? null;
  }

  key(index: number): string | null {
    this.#check();
    return [...this.#items.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.#check();
    this.#items.delete(key);
  }

  setItem(key: string, value: string): void {
    this.#check();
    this.#items.set(key, String(value));
  }
}

export function installStorage(): MemoryStorage {
  const storage = new MemoryStorage();
  Object.defineProperty(globalThis, 'localStorage', { value: storage, configurable: true, writable: true });
  return storage;
}
