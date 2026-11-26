/*
 * The replayer of exported histories (design 044, Requirement 044-025, Technical Plan 7.4): it replays a run's history
 * against a local bundle and reports the first difference, so that a team's own tests can prove that a change of its
 * workflow code still replays the runs it exported. It takes the bundle, not host functions, because no host function
 * enters a workflow context.
 */
import { hex } from './chain.js';
import type { CommandKind, HistoryEvent } from './context.js';
import { ActivationCutter, HistoryOrderError } from './history.js';
import { WorkflowThread, type RunHandle } from './threaded.js';

/** The export format versions this replayer reads. */
export const EXPORT_FORMATS: readonly number[] = [1];

/** The first line of an exported history. */
export interface ExportHeader {
  exportFormat: number;
  /** The run's seed, 64 hexadecimal characters (the export is the one read that returns it). */
  seed: string;
  /** The activity names of the run's version whose envelope entry requires approval. */
  gatedEntries: string[];
  /** The declared state's `maxBytes` of the run's version. */
  stateMaxBytes: number;
}

export class UnsupportedExportFormat extends Error {
  readonly exportFormat: unknown;
  constructor(exportFormat: unknown) {
    super(`this replayer reads export formats ${EXPORT_FORMATS.join(', ')}, not ${JSON.stringify(exportFormat) ?? 'none'}`);
    this.name = 'UnsupportedExportFormat';
    this.exportFormat = exportFormat;
  }
}

export class MalformedHistory extends Error {
  constructor(message: string) { super(message); this.name = 'MalformedHistory'; }
}

export type ReplayVerdict =
  | { verdict: 'MATCH'; activations: number; commands: number; chain: string }
  | { verdict: 'NONDETERMINISM'; activation: number; commandNumber: number; actualKind: string; actualTarget: string; expectedKind?: CommandKind; expectedTarget?: string }
  | { verdict: 'INCONCLUSIVE_REDACTED'; activation: number; commandNumber: number }
  | { verdict: 'WORKFLOW_TASK_FAILED' | 'ACTIVATION_DEADLINE' | 'VALIDATOR_COMMAND' | 'NONDETERMINISM_RISK'; activation: number; exceptionType?: string };

export interface ReplayOptions {
  /** The bundle file the worker loads (`sagat-workflows bundle` writes it), or its text. */
  bundlePath?: string;
  bundleCode?: string;
  workflowType: string;
  activationDeadlineMs?: number;
  /** The workflow thread to replay in; the process's shared one by default. */
  thread?: WorkflowThread;
}

/** Reads the lines of an export: the header, then one `HistoryEvent` per line. */
export async function readExport(lines: AsyncIterable<string> | Iterable<string>): Promise<{ header: ExportHeader; events: HistoryEvent[] }> {
  let header: ExportHeader | undefined;
  const events: HistoryEvent[] = [];
  for await (const line of lines) {
    if (line.trim() === '') continue;
    let value: unknown;
    try { value = JSON.parse(line); } catch { throw new MalformedHistory('a line of the history is not JSON'); }
    if (!header) { header = parseHeader(value); continue; }
    events.push(parseEvent(value));
  }
  if (!header) throw new MalformedHistory('the history has no header line');
  return { header, events };
}

function parseHeader(value: unknown): ExportHeader {
  const record = value as Record<string, unknown> | null;
  if (!record || typeof record !== 'object' || !EXPORT_FORMATS.includes(record['exportFormat'] as number)) {
    throw new UnsupportedExportFormat(record && typeof record === 'object' ? record['exportFormat'] : undefined);
  }
  const seed = record['seed'];
  if (typeof seed !== 'string' || !/^[0-9a-f]{64}$/.test(seed)) throw new MalformedHistory('the header has no seed of 64 hexadecimal characters');
  const gated = record['gatedEntries'] ?? [];
  if (!Array.isArray(gated) || !gated.every((name) => typeof name === 'string')) throw new MalformedHistory('gatedEntries is a list of names');
  const stateMaxBytes = record['stateMaxBytes'] ?? 0;
  if (!Number.isSafeInteger(stateMaxBytes) || (stateMaxBytes as number) < 0) throw new MalformedHistory('stateMaxBytes is a byte count');
  return { exportFormat: record['exportFormat'] as number, seed, gatedEntries: gated as string[], stateMaxBytes: stateMaxBytes as number };
}

// The export format's event lines are the worker protocol's `HistoryEvent` in the proto3 JSON mapping (Task 044-2);
// until that contract merges they are read in the runtime's own model, with the same names.
function parseEvent(value: unknown): HistoryEvent {
  const event = value as Record<string, unknown> | null;
  if (!event || typeof event !== 'object' || typeof event['kind'] !== 'string') throw new MalformedHistory('a history event has no kind');
  const seq = typeof event['seq'] === 'string' ? Number(event['seq']) : event['seq'];
  if (!Number.isSafeInteger(seq)) throw new MalformedHistory('a history event has no sequence');
  const atUs = typeof event['atUs'] === 'string' ? Number(event['atUs']) : event['atUs'];
  return { ...event, seq, ...(atUs === undefined ? {} : { atUs }) } as HistoryEvent;
}

export const Replayer = {
  /** Replays a history (the lines of an export) and answers the verdict of its first difference. */
  async replay(history: AsyncIterable<string> | Iterable<string>, options: ReplayOptions): Promise<ReplayVerdict> {
    const { header, events } = await readExport(history);
    return await replayEvents(header, events, options);
  },
};

/** Replays the events of a history, every completed activation in turn. */
export async function replayEvents(header: ExportHeader, events: HistoryEvent[], options: ReplayOptions): Promise<ReplayVerdict> {
  const thread = options.thread ?? WorkflowThread.shared();
  const bundle = options.bundleCode !== undefined ? await thread.loadBundle(options.bundleCode)
    : options.bundlePath !== undefined ? await thread.loadBundleFile(options.bundlePath) : undefined;
  if (!bundle) throw new TypeError('a replay needs bundlePath or bundleCode');
  const init = { workflowType: options.workflowType, seed: header.seed, stateMaxBytes: header.stateMaxBytes, gatedEntries: header.gatedEntries };
  const run = await thread.createRun(bundle, init, options.activationDeadlineMs === undefined ? {} : { activationDeadlineMs: options.activationDeadlineMs });
  try {
    return await replayOn(run, events);
  } finally {
    run.dispose();
  }
}

async function replayOn(run: RunHandle, events: HistoryEvent[]): Promise<ReplayVerdict> {
  const cutter = new ActivationCutter();
  let activations;
  try {
    activations = [...cutter.push(events), ...cutter.flush().activations];
  } catch (error) {
    if (error instanceof HistoryOrderError) throw new MalformedHistory(error.message);
    throw error;
  }
  let index = 0;
  for (const activation of activations) {
    index++;
    const step = await run.replay(activation);
    if (step.kind === 'inconclusive') return { verdict: 'INCONCLUSIVE_REDACTED', activation: index, commandNumber: step.commandNumber };
    if (step.kind === 'failure') {
      const failure = step.failure;
      if (failure.cause === 'NONDETERMINISM') {
        const verdict: ReplayVerdict = {
          verdict: 'NONDETERMINISM', activation: index, commandNumber: failure.commandNumber ?? 0,
          actualKind: failure.actualKind ?? '', actualTarget: failure.actualTarget ?? '',
        };
        const expected = activation.records.find((record) => record.commandNumber === failure.commandNumber);
        if (expected) { verdict.expectedKind = expected.kind; verdict.expectedTarget = expected.target; }
        else if (activation.deferral) { verdict.expectedKind = activation.deferral.kind; verdict.expectedTarget = activation.deferral.target; }
        return verdict;
      }
      const verdict: ReplayVerdict = { verdict: failure.cause, activation: index };
      if (failure.exceptionType) verdict.exceptionType = failure.exceptionType;
      return verdict;
    }
  }
  return { verdict: 'MATCH', activations: index, commands: run.acceptedCommands, chain: hex(run.chain) };
}
