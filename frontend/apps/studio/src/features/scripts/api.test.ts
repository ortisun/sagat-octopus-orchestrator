import { describe, expect, it } from 'vitest';
import { ApiClient } from '../../api/client';
import { scriptsApi } from './api';

const NS = '11111111-1111-4111-8111-111111111111';
const SCRIPT = '22222222-2222-4222-8222-222222222222';

function harness(answer: () => Response = () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })) {
  const calls: { method: string; url: string; headers: Record<string, string>; body: unknown }[] = [];
  const fetcher = ((url: string, init: RequestInit = {}) => {
    calls.push({ method: init.method ?? 'GET', url, headers: init.headers as Record<string, string>, body: init.body === undefined ? undefined : JSON.parse(init.body as string) });
    return Promise.resolve(answer());
  }) as typeof fetch;
  let n = 0;
  const client = new ApiClient({ baseUrl: '/v1', token: () => 'tok', namespace: () => 'payments', onUnauthenticated: () => undefined, fetch: fetcher, newKey: () => `key-${++n}` });
  return { api: scriptsApi(client, NS), calls };
}

describe('scriptsApi', () => {
  it('addresses the scripts of the namespace by its id, and one script by its own', async () => {
    const { api, calls } = harness();
    await api.list({ after: 'alpha', limit: 50 });
    await api.get(SCRIPT);
    await api.versions(SCRIPT, { before: 7, limit: 20 });
    await api.version(SCRIPT, 3);
    await api.testRuns(SCRIPT, { limit: 20 });
    await api.testRun(SCRIPT, '33333333-3333-4333-8333-333333333333');
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual([
      `GET /v1/namespaces/${NS}/scripts?after=alpha&limit=50`,
      `GET /v1/namespaces/${NS}/scripts/${SCRIPT}`,
      `GET /v1/namespaces/${NS}/scripts/${SCRIPT}/versions?before=7&limit=20`,
      `GET /v1/namespaces/${NS}/scripts/${SCRIPT}/versions/3`,
      `GET /v1/namespaces/${NS}/scripts/${SCRIPT}/test-runs?limit=20`,
      `GET /v1/namespaces/${NS}/scripts/${SCRIPT}/test-runs/33333333-3333-4333-8333-333333333333`,
    ]);
  });

  it('leaves out the query it was not given', async () => {
    const { api, calls } = harness();
    await api.list();
    await api.versions(SCRIPT);
    expect(calls.map((c) => c.url)).toEqual([`/v1/namespaces/${NS}/scripts`, `/v1/namespaces/${NS}/scripts/${SCRIPT}/versions`]);
  });

  it('encodes every id, so one from the address bar cannot change which resource is meant', async () => {
    const { api, calls } = harness();
    await api.get('a/b?x=1#y');
    await api.testRun(SCRIPT, 'q/x\\r');
    expect(calls[0]?.url).toBe(`/v1/namespaces/${NS}/scripts/a%2Fb%3Fx%3D1%23y`);
    expect(calls[1]?.url).toBe(`/v1/namespaces/${NS}/scripts/${SCRIPT}/test-runs/q%2Fx%5Cr`);
  });

  it('refuses at the client a path that climbs (before any request is made)', async () => {
    const { api, calls } = harness();
    await expect(api.get('..')).rejects.toThrow();
    expect(calls).toHaveLength(0);
  });

  it('sends the bodies of the contract, and a key with every change', async () => {
    const { api, calls } = harness();
    await api.create({ name: 'double-it', language: 'typescript', source: 'export default 1' });
    await api.update(SCRIPT, { revision: 4, source: 'x', inputSchema: null });
    await api.publish(SCRIPT);
    await api.startTestRun(SCRIPT, { n: 21 });
    await api.remove(SCRIPT);
    expect(calls.map((c) => [c.method, c.body])).toEqual([
      ['POST', { name: 'double-it', language: 'typescript', source: 'export default 1' }],
      ['PATCH', { revision: 4, source: 'x', inputSchema: null }],
      ['POST', undefined],
      ['POST', { input: { n: 21 } }],
      ['DELETE', undefined],
    ]);
    expect(calls.map((c) => c.headers['Idempotency-Key'])).toEqual(['key-1', 'key-2', 'key-3', 'key-4', 'key-5']);
    expect(calls[0]?.url).toBe(`/v1/namespaces/${NS}/scripts`);
    expect(calls[2]?.url).toBe(`/v1/namespaces/${NS}/scripts/${SCRIPT}/versions`);
    expect(calls[3]?.url).toBe(`/v1/namespaces/${NS}/scripts/${SCRIPT}/test-runs`);
  });

  it('keeps the key it is given, so the retry of a change is recognised by the platform', async () => {
    const { api, calls } = harness();
    await api.update(SCRIPT, { revision: 1, source: 'x' }, 'same');
    await api.update(SCRIPT, { revision: 1, source: 'x' }, 'same');
    expect(calls.map((c) => c.headers['Idempotency-Key'])).toEqual(['same', 'same']);
  });

  it('sends no key with a read, and the token only as a header', async () => {
    const { api, calls } = harness();
    await api.get(SCRIPT);
    expect(calls[0]?.headers['Idempotency-Key']).toBeUndefined();
    expect(calls[0]?.headers['Authorization']).toBe('Bearer tok');
    expect(calls[0]?.url).not.toContain('tok');
  });

  it('sends a null input as the input (null is a value), and no input when there is none', async () => {
    const { api, calls } = harness();
    await api.startTestRun(SCRIPT, null);
    await api.startTestRun(SCRIPT, undefined);
    expect(calls.map((c) => c.body)).toEqual([{ input: null }, {}]);
  });

  it('reads the answer of a delete that has no body', async () => {
    const { api } = harness(() => new Response(null, { status: 204 }));
    await expect(api.remove(SCRIPT)).resolves.toBeUndefined();
  });
});
