import type { IrNode } from './ir';

/** What a link between two nodes means; the same words the runtime graph uses. */
export type EdgeKind = 'next' | 'error' | 'true' | 'false' | 'branch' | 'timeout' | 'begin' | 'body';

export type FieldKind = 'text' | 'integer' | 'duration' | 'boolean' | 'select' | 'expression' | 'tags' | 'retry';

export interface FieldSpec {
  name: string;
  label: string;
  kind: FieldKind;
  required?: boolean;
  options?: readonly string[];
  hint?: string;
  min?: number;
  max?: number;
}

/** A way out of a node: the field of the document that holds the id of the node it leads to. */
export interface HandleSpec {
  field: string;
  edge: EdgeKind;
  label: string;
  /** The field is a list of ids (`branches`). */
  multiple?: boolean;
}

export type Category = 'work' | 'flow' | 'wait' | 'saga' | 'end';

export interface NodeKindSpec {
  type: string;
  label: string;
  category: Category;
  description: string;
  /** False for a node the schema accepts and the engine does not run yet (D145, G147). */
  runs: boolean;
  fields: readonly FieldSpec[];
  handles: readonly HandleSpec[];
  /** A new node of this kind, with the fields the schema requires filled with something valid. */
  make(id: string): IrNode;
}

const NEXT: HandleSpec = { field: 'next', edge: 'next', label: 'Next' };
const ON_ERROR: HandleSpec = { field: 'onError', edge: 'error', label: 'On error' };

/** The node types of the schema, in the order of the palette. */
export const NODE_KINDS: readonly NodeKindSpec[] = [
  {
    type: 'activity',
    label: 'Step',
    category: 'work',
    description: 'A durable task that a worker of your own runs.',
    runs: true,
    fields: [
      { name: 'activity', label: 'Activity', kind: 'text', required: true, hint: 'The name a worker registers, such as payment.submit.' },
      { name: 'timeoutMs', label: 'Timeout', kind: 'duration', hint: 'The longest one attempt may take.' },
      { name: 'retry', label: 'Retries', kind: 'retry' },
      { name: 'compensateWith', label: 'Compensation activity', kind: 'text', hint: 'Undoes this step when a saga around it compensates.' },
      { name: 'reconcileWith', label: 'Reconciliation activity', kind: 'text', hint: 'Finds out whether this step happened when an attempt ends with an unknown outcome.' },
      { name: 'idempotencyTemplate', label: 'Idempotency key', kind: 'expression', hint: 'For example reserve:${run.id}.' },
      { name: 'secrets', label: 'Secrets', kind: 'tags', hint: 'Names of secrets the worker may read for this step.' },
      { name: 'irreversible', label: 'Cannot be undone', kind: 'boolean' },
      { name: 'highRisk', label: 'High risk', kind: 'boolean', hint: 'An attempt with an unknown outcome is never retried by itself.' },
    ],
    handles: [NEXT, ON_ERROR],
    make: (id) => ({ id, type: 'activity', activity: '' }),
  },
  {
    type: 'script',
    label: 'Script',
    category: 'work',
    description: 'Code you wrote in the Studio, built and signed by the platform, run in a sandbox.',
    runs: true,
    fields: [
      { name: 'artifactDigest', label: 'Artifact', kind: 'text', required: true, hint: 'The digest of a built version of a script: sha256:…' },
      { name: 'language', label: 'Language', kind: 'select', options: ['typescript', 'python', 'csharp', 'go', 'java'] },
      { name: 'timeoutMs', label: 'Timeout', kind: 'duration' },
      { name: 'retry', label: 'Retries', kind: 'retry' },
      { name: 'compensateWith', label: 'Compensation activity', kind: 'text' },
      { name: 'secrets', label: 'Secrets', kind: 'tags' },
    ],
    handles: [NEXT, ON_ERROR],
    make: (id) => ({ id, type: 'script', artifactDigest: '' }),
  },
  {
    type: 'condition',
    label: 'Condition',
    category: 'flow',
    description: 'Takes one way or the other by looking at the data of the run.',
    runs: true,
    fields: [{ name: 'expression', label: 'Expression', kind: 'expression', required: true, hint: 'For example input.amount > 1000.' }],
    handles: [
      { field: 'ifTrue', edge: 'true', label: 'If true' },
      { field: 'ifFalse', edge: 'false', label: 'If false' },
    ],
    make: (id) => ({ id, type: 'condition', expression: 'true', ifTrue: '', ifFalse: '' }),
  },
  {
    type: 'parallel',
    label: 'Parallel',
    category: 'flow',
    description: 'Runs several branches at once and goes on when they are done (the join).',
    runs: true,
    fields: [{ name: 'join', label: 'Join', kind: 'select', required: true, options: ['all', 'any'], hint: 'All branches, or the first to finish.' }],
    handles: [{ field: 'branches', edge: 'branch', label: 'Branch', multiple: true }, NEXT],
    make: (id) => ({ id, type: 'parallel', join: 'all', branches: [] }),
  },
  {
    type: 'timer',
    label: 'Timer',
    category: 'wait',
    description: 'Waits for a delay without holding any process.',
    runs: true,
    fields: [{ name: 'durationMs', label: 'Delay', kind: 'duration', required: true }],
    handles: [NEXT],
    make: (id) => ({ id, type: 'timer', durationMs: 60_000 }),
  },
  {
    type: 'waitEvent',
    label: 'Wait for an event',
    category: 'wait',
    description: 'Waits until an event of a type arrives for this run, or the time runs out.',
    runs: true,
    fields: [
      { name: 'eventType', label: 'Event type', kind: 'text', required: true },
      { name: 'correlationExpr', label: 'Correlation', kind: 'expression', required: true, hint: 'Which events are for this run, for example input.orderId.' },
      { name: 'timeoutMs', label: 'Wait at most', kind: 'duration', required: true },
      { name: 'lookbackMs', label: 'Look back', kind: 'duration', hint: 'An event that arrived this long before the wait started also counts.' },
    ],
    handles: [NEXT, { field: 'onTimeout', edge: 'timeout', label: 'On timeout' }],
    make: (id) => ({ id, type: 'waitEvent', eventType: '', correlationExpr: 'input.id', timeoutMs: 300_000 }),
  },
  {
    type: 'child',
    label: 'Child workflow',
    category: 'flow',
    description: 'Starts another workflow and waits for it.',
    runs: true,
    fields: [
      { name: 'workflowId', label: 'Workflow', kind: 'text', required: true, hint: 'The name of the workflow to start.' },
      { name: 'version', label: 'Version', kind: 'integer', min: 1, hint: 'Empty: the active version.' },
      { name: 'input', label: 'Input', kind: 'expression', hint: 'An expression that gives a JSON object.' },
      { name: 'cancelPolicy', label: 'If this run is stopped', kind: 'select', options: ['propagate', 'request_cancel', 'detach'] },
    ],
    handles: [NEXT, ON_ERROR],
    make: (id) => ({ id, type: 'child', workflowId: '' }),
  },
  {
    type: 'saga',
    label: 'Saga',
    category: 'saga',
    description: 'A group of steps that are undone, in reverse order, if one of them fails for good.',
    runs: true,
    fields: [
      { name: 'onCompensationFailure', label: 'If an undo fails', kind: 'select', options: ['manual_review', 'fail'] },
      { name: 'cancelPolicy', label: 'If it is cancelled', kind: 'select', options: ['compensate_completed', 'abort_no_compensation', 'wait_in_flight_then_compensate'] },
    ],
    handles: [{ field: 'begin', edge: 'begin', label: 'First step' }, NEXT],
    make: (id) => ({ id, type: 'saga', begin: '' }),
  },
  {
    type: 'end',
    label: 'End',
    category: 'end',
    description: 'Finishes the run, optionally with an output.',
    runs: true,
    fields: [{ name: 'outputExpr', label: 'Output', kind: 'expression', hint: 'An expression over the data of the run.' }],
    handles: [],
    make: (id) => ({ id, type: 'end' }),
  },
  {
    type: 'foreach',
    label: 'For each',
    category: 'flow',
    description: 'Repeats a step for every item of a list. The platform does not run it yet.',
    runs: false,
    fields: [
      { name: 'collectionExpr', label: 'Collection', kind: 'expression', required: true },
      { name: 'itemKeyExpr', label: 'Item key', kind: 'expression', required: true },
      { name: 'maxItems', label: 'Most items', kind: 'integer', required: true, min: 1, max: 100_000 },
      { name: 'maxConcurrency', label: 'At a time', kind: 'integer', min: 1 },
    ],
    handles: [{ field: 'body', edge: 'body', label: 'Body' }, NEXT],
    make: (id) => ({ id, type: 'foreach', collectionExpr: 'input.items', itemKeyExpr: 'item.id', body: '', maxItems: 100 }),
  },
  {
    type: 'humanApproval',
    label: 'Human approval',
    category: 'wait',
    description: 'Waits for a person to approve. The platform does not run it yet.',
    runs: false,
    fields: [
      { name: 'approverPolicy', label: 'Who approves', kind: 'text', required: true },
      { name: 'timeoutMs', label: 'Wait at most', kind: 'duration', required: true },
    ],
    handles: [NEXT, { field: 'onTimeout', edge: 'timeout', label: 'On timeout' }],
    make: (id) => ({ id, type: 'humanApproval', approverPolicy: '', timeoutMs: 86_400_000, formSchema: { type: 'object' } }),
  },
];

const BY_TYPE = new Map(NODE_KINDS.map((kind) => [kind.type, kind]));

export function kindOf(type: string): NodeKindSpec | undefined {
  return BY_TYPE.get(type);
}

/** The ids a node points at, with the field and kind of each link. */
export interface Link {
  from: string;
  to: string;
  kind: EdgeKind;
  field: string;
  /** The position in a list field. */
  index?: number;
}

export function linksOf(node: IrNode): Link[] {
  const spec = kindOf(node.type);
  const handles = spec?.handles ?? [NEXT, ON_ERROR];
  const out: Link[] = [];
  for (const handle of handles) {
    const value = node[handle.field];
    if (handle.multiple) {
      if (Array.isArray(value)) {
        value.forEach((target, index) => {
          if (typeof target === 'string' && target !== '') out.push({ from: node.id, to: target, kind: handle.edge, field: handle.field, index });
        });
      }
    } else if (typeof value === 'string' && value !== '') {
      out.push({ from: node.id, to: value, kind: handle.edge, field: handle.field });
    }
  }
  return out;
}
