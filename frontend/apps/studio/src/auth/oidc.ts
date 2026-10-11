/**
 * Sign-in with the deployment's identity provider: the authorization code flow with PKCE
 * (RFC 6749, RFC 7636, OpenID Connect Core), as a public client. The page holds no secret; what it
 * keeps across the redirect (a `state` and a code verifier) is useless without the issuer's code,
 * and is deleted the moment the callback is read, whatever it says. The access token that comes back
 * goes to the session and nowhere else (D144, G149).
 */
export interface OidcSettings {
  issuer: string;
  clientId: string;
  scope: string;
}

export interface StartDeps {
  fetch?: typeof globalThis.fetch;
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  /** The origin of the page, `https://host`. */
  origin?: string;
  assign?: (url: string) => void;
}

export interface FinishDeps {
  fetch?: typeof globalThis.fetch;
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  origin?: string;
  replace?: (path: string) => void;
}

const PENDING = 'studio.oidc.pending';
export const CALLBACK_PATH = '/studio/auth/callback';
const B64URL = /^[A-Za-z0-9_-]+$/;

function b64url(bytes: Uint8Array): string {
  let text = '';
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function random(length: number): string {
  return b64url(crypto.getRandomValues(new Uint8Array(length)));
}

/** 32 random bytes: 43 characters, the shortest verifier RFC 7636 allows. */
export function newVerifier(): string {
  return random(32);
}

export async function challengeOf(verifier: string): Promise<string> {
  return b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))));
}

interface Endpoints {
  authorization: string;
  token: string;
}

function allowed(url: string, issuer: URL): URL | undefined {
  try {
    const parsed = new URL(url);
    const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(parsed.hostname);
    const secure = parsed.protocol === 'https:' || (parsed.protocol === 'http:' && loopback);
    // The page may call the issuer's origin only (the policy of the page says the same).
    return secure && parsed.origin === issuer.origin && parsed.username === '' && parsed.password === '' ? parsed : undefined;
  } catch {
    return undefined;
  }
}

async function discover(settings: OidcSettings, fetcher: typeof globalThis.fetch): Promise<Endpoints> {
  const issuer = new URL(settings.issuer);
  const fail = () => new Error('The identity provider is not set up the way this Studio needs. Ask whoever runs the platform.');
  let document: Record<string, unknown>;
  try {
    const response = await fetcher(`${settings.issuer.replace(/\/+$/, '')}/.well-known/openid-configuration`, { credentials: 'omit', redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer' });
    if (!response.ok) throw fail();
    document = (await response.json()) as Record<string, unknown>;
  } catch {
    throw fail();
  }
  const authorization = typeof document['authorization_endpoint'] === 'string' ? allowed(document['authorization_endpoint'], issuer) : undefined;
  const token = typeof document['token_endpoint'] === 'string' ? allowed(document['token_endpoint'], issuer) : undefined;
  if (document['issuer'] !== settings.issuer || !authorization || !token) throw fail();
  return { authorization: authorization.toString(), token: token.toString() };
}

/** Sends the browser to the identity provider. */
export async function startLogin(settings: OidcSettings, deps: StartDeps = {}): Promise<void> {
  const fetcher = deps.fetch ?? globalThis.fetch.bind(globalThis);
  const storage = deps.storage ?? globalThis.sessionStorage;
  const origin = deps.origin ?? globalThis.location.origin;
  const endpoints = await discover(settings, fetcher);
  const state = random(16);
  const verifier = newVerifier();
  storage.setItem(PENDING, JSON.stringify({ state, verifier }));
  const url = new URL(endpoints.authorization);
  url.search = new URLSearchParams({
    response_type: 'code',
    client_id: settings.clientId,
    redirect_uri: `${origin}${CALLBACK_PATH}`,
    scope: settings.scope,
    state,
    code_challenge: await challengeOf(verifier),
    code_challenge_method: 'S256',
  }).toString();
  (deps.assign ?? ((target) => globalThis.location.assign(target)))(url.toString());
}

/**
 * Reads the callback: checks the state, exchanges the code and returns the access token. The
 * address bar is cleaned and the pending attempt deleted before anything else can go wrong.
 */
export async function finishLogin(search: string, settings: OidcSettings, deps: FinishDeps = {}): Promise<string> {
  const fetcher = deps.fetch ?? globalThis.fetch.bind(globalThis);
  const storage = deps.storage ?? globalThis.sessionStorage;
  const origin = deps.origin ?? globalThis.location.origin;
  const params = new URLSearchParams(search);
  const raw = storage.getItem(PENDING);
  storage.removeItem(PENDING);
  (deps.replace ?? ((path) => globalThis.history.replaceState(null, '', path)))('/studio/');

  let pending: { state?: unknown; verifier?: unknown } = {};
  try {
    pending = raw === null ? {} : (JSON.parse(raw) as typeof pending);
  } catch {
    pending = {};
  }
  if (typeof pending.state !== 'string' || typeof pending.verifier !== 'string' || params.get('state') !== pending.state) {
    throw new Error('That sign-in did not start in this page. Start again.');
  }
  const error = params.get('error');
  if (error !== null) {
    const description = params.get('error_description');
    throw new Error(`The identity provider refused the sign-in: ${error.slice(0, 64)}${description ? ` (${description.slice(0, 120)})` : ''}.`);
  }
  const code = params.get('code');
  if (code === null || code === '' || code.length > 2048) throw new Error('The identity provider sent no code. Start again.');

  const endpoints = await discover(settings, fetcher);
  let answer: Record<string, unknown>;
  try {
    const response = await fetcher(endpoints.token, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded', Accept: 'application/json' },
      body: new URLSearchParams({ grant_type: 'authorization_code', code, redirect_uri: `${origin}${CALLBACK_PATH}`, client_id: settings.clientId, code_verifier: pending.verifier }).toString(),
      credentials: 'omit',
      redirect: 'error',
      cache: 'no-store',
      referrerPolicy: 'no-referrer',
    });
    answer = (await response.json().catch(() => ({}))) as Record<string, unknown>;
    if (!response.ok) {
      const reason = typeof answer['error'] === 'string' ? answer['error'].slice(0, 64) : String(response.status);
      throw new Error(`The identity provider did not give a token: ${reason}.`);
    }
  } catch (cause) {
    if (cause instanceof Error && cause.message.startsWith('The identity provider')) throw cause;
    throw new Error('The identity provider could not be reached to give a token.');
  }
  const token = answer['access_token'];
  const type = typeof answer['token_type'] === 'string' ? answer['token_type'].toLowerCase() : '';
  if (type !== 'bearer' || typeof token !== 'string' || !/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/.test(token) || !B64URL.test(token.split('.')[0] ?? '')) {
    throw new Error('The identity provider did not give a usable token.');
  }
  return token;
}
