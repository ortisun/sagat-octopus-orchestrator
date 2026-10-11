/**
 * The workflow document (IR, `workflow.schema.json`) as the Studio edits it. The Studio never
 * invents a field: the document is the one the platform stores, validates and publishes, and what
 * it does not understand it keeps untouched (a draft written by hand or by an SDK opens, shows
 * what it can and saves back what it was).
 */
export interface IrNode {
  id: string;
  type: string;
  next?: string;
  onError?: string;
  [field: string]: unknown;
}

export interface IrTrigger {
  kind: string;
  [field: string]: unknown;
}

export interface IrDocument {
  apiVersion: string;
  kind: string;
  metadata: { name: string; description?: string; labels?: Record<string, string>; [field: string]: unknown };
  entrypoint: string;
  inputSchema?: Record<string, unknown>;
  outputSchema?: Record<string, unknown>;
  triggers?: IrTrigger[];
  nodes: IrNode[];
  [field: string]: unknown;
}

export const API_VERSION = 'sagat-octopus-orchestrator.io/v1';

/** The pattern of a node id: it ends up in step identities, so no separators, and `~` is the platform's. */
export const NODE_ID = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

export function isNodeId(value: string): boolean {
  return NODE_ID.test(value);
}

export function emptyDocument(name: string): IrDocument {
  return {
    apiVersion: API_VERSION,
    kind: 'Workflow',
    metadata: { name },
    entrypoint: '',
    triggers: [{ kind: 'manual' }],
    nodes: [],
  };
}

export interface ParsedDocument {
  doc: IrDocument;
  /** Parts of the document the Studio could not place on the canvas (kept as they are). */
  unreadable: string[];
}

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Reads whatever the draft holds. A draft is saved even when it does not validate, so this never
 * refuses: nodes that have no id or no type stay in the document and are listed as unreadable.
 */
export function readDocument(value: unknown): ParsedDocument {
  const raw = isObject(value) ? value : {};
  const unreadable: string[] = [];
  const nodes = Array.isArray(raw['nodes']) ? (raw['nodes'] as unknown[]) : [];
  const metadata = isObject(raw['metadata']) ? (raw['metadata'] as IrDocument['metadata']) : { name: '' };
  const doc: IrDocument = {
    ...raw,
    apiVersion: typeof raw['apiVersion'] === 'string' ? raw['apiVersion'] : API_VERSION,
    kind: typeof raw['kind'] === 'string' ? raw['kind'] : 'Workflow',
    metadata: { ...metadata, name: typeof metadata.name === 'string' ? metadata.name : '' },
    entrypoint: typeof raw['entrypoint'] === 'string' ? raw['entrypoint'] : '',
    nodes: [],
  };
  const seen = new Set<string>();
  nodes.forEach((node, index) => {
    if (!isObject(node) || typeof node['id'] !== 'string' || typeof node['type'] !== 'string') {
      unreadable.push(`/nodes/${index}`);
      return;
    }
    if (seen.has(node['id'])) {
      unreadable.push(`/nodes/${index}`);
      return;
    }
    seen.add(node['id']);
    doc.nodes.push(node as IrNode);
  });
  return { doc, unreadable };
}

/** The same document with its keys sorted, to tell whether two documents say the same. */
export function canonical(value: unknown): string {
  return JSON.stringify(sorted(value));
}

function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sorted);
  if (isObject(value)) {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sorted(value[key])]),
    );
  }
  return value;
}

export function sameDocument(a: unknown, b: unknown): boolean {
  return canonical(a) === canonical(b);
}

export function findNode(doc: IrDocument, id: string): IrNode | undefined {
  return doc.nodes.find((node) => node.id === id);
}
