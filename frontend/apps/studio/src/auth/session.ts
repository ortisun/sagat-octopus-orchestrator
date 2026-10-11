import type { AuthContext } from '../api/endpoints';

/** Who the platform says the user is. */
export interface SessionInfo {
  tenant: string;
  subject: string;
  role: AuthContext['role'];
  permissions: ReadonlySet<string>;
}

export type SessionEnd = 'signed-out' | 'expired' | 'rejected';

export interface SessionSnapshot {
  info: SessionInfo | undefined;
  /** Why the last session ended, until the next one starts. */
  ended: SessionEnd | undefined;
  /** When the token expires (seconds since the epoch), read from the token for display only. */
  expiresAt: number | undefined;
}

const JWT_SHAPE = /^[A-Za-z0-9_-]{1,4096}\.[A-Za-z0-9_-]{1,4096}\.[A-Za-z0-9_-]{0,4096}$/;

/** The `exp` of a token, for display. The platform verifies the token; the browser only reads the date. */
export function expiryOf(token: string): number | undefined {
  try {
    const payload = token.split('.')[1] ?? '';
    const json = atob(payload.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(payload.length / 4) * 4, '='));
    const exp = (JSON.parse(json) as { exp?: unknown }).exp;
    return typeof exp === 'number' && Number.isFinite(exp) ? exp : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The session of the user (D144): a bearer token held in memory, and what the platform says about
 * who it belongs to. The token is never written anywhere (no storage, no cookie, no URL) and is
 * gone when the page is reloaded; the page never prints it.
 */
export class SessionStore {
  #token: string | undefined;
  #snapshot: SessionSnapshot = { info: undefined, ended: undefined, expiresAt: undefined };
  readonly #listeners = new Set<() => void>();
  #timer: ReturnType<typeof setTimeout> | undefined;
  readonly #now: () => number;

  constructor(now: () => number = () => Date.now()) {
    this.#now = now;
  }

  token = (): string | undefined => this.#token;

  snapshot = (): SessionSnapshot => this.#snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  can(permission: string): boolean {
    return this.#snapshot.info?.permissions.has(permission) ?? false;
  }

  /** Whether `value` looks like a token worth sending to the platform. */
  static looksLikeToken(value: string): boolean {
    return JWT_SHAPE.test(value);
  }

  /**
   * Starts a session with `token` if the platform accepts it. The token is kept only after the
   * platform has said whose it is.
   */
  async signIn(token: string, whoAmI: (token: string) => Promise<AuthContext>): Promise<void> {
    const trimmed = token.trim();
    if (!SessionStore.looksLikeToken(trimmed)) throw new Error('That is not a token: it has three parts joined by dots.');
    const context = await whoAmI(trimmed);
    this.#token = trimmed;
    const expiresAt = expiryOf(trimmed);
    this.#set({
      info: { tenant: context.tenant, subject: context.subject, role: context.role, permissions: new Set(context.permissions) },
      ended: undefined,
      expiresAt,
    });
    this.#arm(expiresAt);
  }

  signOut(reason: SessionEnd = 'signed-out'): void {
    if (this.#token === undefined && this.#snapshot.info === undefined) return;
    this.#token = undefined;
    clearTimeout(this.#timer);
    this.#set({ info: undefined, ended: reason, expiresAt: undefined });
  }

  #arm(expiresAt: number | undefined): void {
    clearTimeout(this.#timer);
    if (expiresAt === undefined) return;
    const wait = expiresAt * 1000 - this.#now();
    // setTimeout cannot wait more than about 24 days; a token does not live that long anyway.
    this.#timer = setTimeout(() => this.signOut('expired'), Math.min(Math.max(wait, 0), 2 ** 31 - 1));
  }

  #set(next: SessionSnapshot): void {
    this.#snapshot = next;
    for (const listener of [...this.#listeners]) listener();
  }
}
