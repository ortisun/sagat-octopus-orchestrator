import type { IrNode } from './ir';
import { kindOf } from './registry';

/**
 * Where an expression reads the output of another step: `steps.charge.output` and
 * `steps["charge-card"].output` (the second form is the only one that can name an id with a dash or
 * a dot). The expression language is a subset of CEL (`crates/infra-expr`): a name is letters, digits
 * and underscores, a string is quoted with `'` or `"`, and a template (`reserve:${run.id}`) is text
 * with expressions inside `${…}`.
 */
export interface StepReference {
  /** The id of the step it reads. */
  id: string;
  /** Where the id is written, so that it can be replaced: from `start` to `end` of the expression. */
  start: number;
  end: number;
  /** `dot` for `steps.id`, `index` for `steps["id"]` (the span is then the whole string literal). */
  form: 'dot' | 'index';
  /** The quote of an `index` reference. */
  quote?: string;
}

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
const NAME_CHAR = /[A-Za-z0-9_]/;

/** The ranges of `source` that are code: all of it, or the insides of `${…}` of a template. */
function codeRanges(source: string, template: boolean): Array<[number, number]> {
  if (!template) return [[0, source.length]];
  const ranges: Array<[number, number]> = [];
  let i = 0;
  while (i < source.length) {
    const open = source.indexOf('${', i);
    if (open === -1) break;
    let j = open + 2;
    let quote: string | undefined;
    for (; j < source.length; j++) {
      const c = source[j] as string;
      if (quote !== undefined) {
        if (c === '\\') j++;
        else if (c === quote) quote = undefined;
      } else if (c === "'" || c === '"') quote = c;
      else if (c === '}') break;
    }
    ranges.push([open + 2, j]);
    i = j + 1;
  }
  return ranges;
}

/** Every read of another step's output in `source`, in order. */
export function stepReferences(source: string, template = false): StepReference[] {
  const found: StepReference[] = [];
  for (const [from, to] of codeRanges(source, template)) {
    let i = from;
    while (i < to) {
      const c = source[i] as string;
      if (c === "'" || c === '"') {
        // A string literal: nothing in it is code.
        i++;
        while (i < to && source[i] !== c) i += source[i] === '\\' ? 2 : 1;
        i++;
        continue;
      }
      const startsWord = NAME_CHAR.test(c) && (i === from || !NAME_CHAR.test(source[i - 1] as string));
      if (!startsWord) {
        i++;
        continue;
      }
      let end = i;
      while (end < to && NAME_CHAR.test(source[end] as string)) end++;
      const word = source.slice(i, end);
      // `steps` is the variable only when it is not itself a member (`x.steps`).
      const member = source.slice(from, i).trimEnd().endsWith('.');
      if (word === 'steps' && !member) {
        let k = end;
        while (k < to && /\s/.test(source[k] as string)) k++;
        if (source[k] === '.') {
          k++;
          while (k < to && /\s/.test(source[k] as string)) k++;
          let nameEnd = k;
          while (nameEnd < to && NAME_CHAR.test(source[nameEnd] as string)) nameEnd++;
          if (nameEnd > k && NAME.test(source.slice(k, nameEnd))) {
            found.push({ id: source.slice(k, nameEnd), start: k, end: nameEnd, form: 'dot' });
            i = nameEnd;
            continue;
          }
        } else if (source[k] === '[') {
          k++;
          while (k < to && /\s/.test(source[k] as string)) k++;
          const quote = source[k];
          if (quote === "'" || quote === '"') {
            let close = k + 1;
            while (close < to && source[close] !== quote) close += source[close] === '\\' ? 2 : 1;
            if (close < to) {
              found.push({ id: source.slice(k + 1, close), start: k, end: close + 1, form: 'index', quote });
              i = close + 1;
              continue;
            }
          }
        }
      }
      i = end;
    }
  }
  return found;
}

/** `source` with every read of the output of step `from` reading the output of step `to`. */
export function renameStepReference(source: string, from: string, to: string, template = false): string {
  let out = source;
  // From the end, so that the positions of the ones before stay right.
  for (const ref of stepReferences(source, template).filter((r) => r.id === from).reverse()) {
    let text: string;
    let start = ref.start;
    if (ref.form === 'index') text = `${ref.quote}${to}${ref.quote}`;
    else if (NAME.test(to)) text = to;
    else {
      // `steps.old` becomes `steps["new-id"]`: the dot goes, the index takes its place.
      start = out.lastIndexOf('.', ref.start);
      text = `["${to}"]`;
    }
    out = out.slice(0, start) + text + out.slice(ref.end);
  }
  return out;
}

/** The fields of a node that hold an expression, with whether each is a template. */
export function expressionFields(node: IrNode): Array<{ field: string; template: boolean; value: string }> {
  const spec = kindOf(node.type);
  if (!spec) return [];
  return spec.fields
    .filter((f) => f.kind === 'expression')
    .flatMap((f) => {
      const value = node[f.name];
      return typeof value === 'string' ? [{ field: f.name, template: f.name === 'idempotencyTemplate', value }] : [];
    });
}

/** The reads of a step that does not exist: `[node, field, id]`. */
export function danglingReferences(nodes: readonly IrNode[]): Array<{ nodeId: string; field: string; id: string }> {
  const ids = new Set(nodes.map((n) => n.id));
  const out: Array<{ nodeId: string; field: string; id: string }> = [];
  for (const node of nodes) {
    for (const { field, template, value } of expressionFields(node)) {
      for (const ref of stepReferences(value, template)) {
        if (!ids.has(ref.id)) out.push({ nodeId: node.id, field, id: ref.id });
      }
    }
  }
  return out;
}
