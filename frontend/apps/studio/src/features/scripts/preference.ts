const KEY = 'studio.scripts.plainEditor';

/**
 * Whether the person asked for a plain text box instead of the code editor. A per-viewer
 * convenience: the page works without it, and a blocked storage just means the choice lasts until
 * the page is closed.
 */
export function readPlainPreference(): boolean {
  try {
    return globalThis.localStorage?.getItem(KEY) === '1';
  } catch {
    return false;
  }
}

export function writePlainPreference(plain: boolean): void {
  try {
    if (plain) globalThis.localStorage?.setItem(KEY, '1');
    else globalThis.localStorage?.removeItem(KEY);
  } catch {
    // Not remembering is acceptable.
  }
}
