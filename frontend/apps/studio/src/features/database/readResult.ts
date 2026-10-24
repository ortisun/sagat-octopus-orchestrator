import { parse, isLosslessNumber } from 'lossless-json';
import type { ApiClient } from '../../api/client';
import { path } from '../../api/client';
import { ApiError } from '../../api/errors';
import type { components } from '../../api/schema';

type WireResult = components['schemas']['DatabaseResult'];
/** JSON numeric tokens stay lossless for display; there is no client arithmetic on cells. */
export type DatabaseResult = Omit<WireResult, 'rows'> & { rows: { type: components['schemas']['DatabaseResultCell']['type']; value: unknown }[][] };
export const MAX_RESULT_BYTES = 4_194_304;
const invalid = () => new ApiError({ code: 'INTERNAL', status: 502, message: 'The database result could not be read within its format or size limits.' });

async function boundedText(response: Response, signal: AbortSignal, limit: number): Promise<string> {
  const reader = response.body?.getReader();
  if (!reader) throw invalid();
  let size = 0; const parts: string[] = []; const decoder = new TextDecoder('utf-8', { fatal: true });
  try {
    if (Number(response.headers.get('content-length')) > limit) throw invalid();
    while (true) {
      const chunk = await reader.read();
      if (signal.aborted) throw invalid();
      if (chunk.done) break;
      size += chunk.value.byteLength;
      if (size > limit) throw invalid();
      parts.push(decoder.decode(chunk.value, { stream: true }));
    }
    parts.push(decoder.decode());
    return parts.join('');
  } catch { throw invalid(); }
  finally { parts.length = 0; await reader.cancel().catch(() => undefined); reader.releaseLock(); }
}

/** Complete bounded response only; errors also have a smaller cap. No query cache. */
export async function readDatabaseResult(api: ApiClient, namespace: string, run: string, node: string, signal: AbortSignal): Promise<DatabaseResult> {
  const deadline = AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
  const response = await api.open(path`/namespaces/${namespace}/runs/${run}/database-results/${node}`, { signal: deadline });
  if (response.status === 401) {
    api.unauthenticated(); await response.body?.cancel().catch(() => undefined);
    throw new ApiError({ code: 'UNAUTHENTICATED', status: 401, message: 'Sign in again to read this result.' });
  }
  const text = await boundedText(response, deadline, response.ok ? MAX_RESULT_BYTES : 65_536);
  // ApiClient now sees only the bounded error envelope, never the original stream.
  if (!response.ok) throw await api.failure(new Response(text, { status: response.status, headers: response.headers }));
  try {
    if (!response.headers.get('content-type')?.toLowerCase().startsWith('application/json')) throw invalid();
    const value = parse(text) as DatabaseResult;
    if (!value || !Array.isArray(value.columns) || !Array.isArray(value.rows) || value.rows.length > 10_000 || typeof value.affectedRows !== 'string' || value.truncated !== false || value.rows.some(row => !Array.isArray(row) || row.length !== value.columns.length)) throw invalid();
    const types = new Set(['null', 'boolean', 'integer', 'decimal', 'float', 'text', 'timestamp', 'date', 'binary', 'json']);
    if (value.columns.some(c => !c || typeof c.name !== 'string' || !types.has(c.type))) throw invalid();
    for (const row of value.rows) for (const cell of row) {
      if (!cell || !Object.hasOwn(cell, 'value') || !types.has(cell.type)) throw invalid();
      if (cell.type === 'null' ? cell.value !== null : cell.type === 'boolean' ? typeof cell.value !== 'boolean' : cell.type === 'float' ? !isLosslessNumber(cell.value) : cell.type === 'json' ? false : typeof cell.value !== 'string') throw invalid();
    }
    return value;
  } catch { throw invalid(); }
}
