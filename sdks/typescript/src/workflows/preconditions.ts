/*
 * The preconditions of the TypeScript determinism runtime (design 044, Technical Plan 1.2 and 1.11): Node 22.8 or later
 * for `vm.constants.DONT_CONTEXTIFY`, and a process time zone of UTC, because the local-time methods of `Date` follow
 * the process time zone inside a context too. The readers are parameters so that the tests can inject a version or a
 * zone; production passes the process's own.
 */

export const MINIMUM_NODE = [22, 8] as const;

/** The time zones that name UTC itself (an offset of zero is not enough: such a zone may have summer time). */
const UTC_ZONES = new Set(['UTC', 'Etc/UTC']);

export class WorkflowRuntimeUnavailable extends Error {
  readonly reason: 'NODE_VERSION' | 'TIME_ZONE';
  constructor(reason: 'NODE_VERSION' | 'TIME_ZONE', message: string) {
    super(message);
    this.name = 'WorkflowRuntimeUnavailable';
    this.reason = reason;
  }
}

export interface RuntimeReaders {
  nodeVersion(): string;
  timeZone(): string;
}

export const PROCESS_READERS: RuntimeReaders = {
  nodeVersion: () => process.versions.node,
  timeZone: () => Intl.DateTimeFormat().resolvedOptions().timeZone,
};

/** Throws unless the Node version supports a context without a host global. */
export function checkNodeVersion(readers: RuntimeReaders = PROCESS_READERS): void {
  const version = readers.nodeVersion();
  const [major = 0, minor = 0] = version.split('.').map((part) => Number.parseInt(part, 10));
  if (major < MINIMUM_NODE[0] || (major === MINIMUM_NODE[0] && minor < MINIMUM_NODE[1])) {
    throw new WorkflowRuntimeUnavailable('NODE_VERSION',
      `workflow code needs Node ${MINIMUM_NODE.join('.')} or later (vm.constants.DONT_CONTEXTIFY); this process runs ${version}`);
  }
}

/** Throws unless every precondition of the runtime holds: checked before a worker registers workflow types. */
export function checkWorkflowRuntime(readers: RuntimeReaders = PROCESS_READERS): void {
  checkNodeVersion(readers);
  const zone = readers.timeZone();
  if (!UTC_ZONES.has(zone)) {
    throw new WorkflowRuntimeUnavailable('TIME_ZONE', `workflow code needs the process time zone UTC (set TZ=UTC); this process runs in ${zone}`);
  }
}
