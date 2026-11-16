/**
 * The operator factory of a collection (design 042, FR-042-028): each function returns one stage of the
 * `operators` of a `forEach`, an object with exactly one key, the operator's name. The factory only assembles
 * the stage; the server validates the chain (its length, the ranges and the variables an expression reads).
 * Stage expressions read `element`, `position`, `input`, `steps`, `run` and `state`.
 */

/** One stage of a collection's operator chain, as `workflow.schema.json` names it. */
export type Operator =
  | { readonly filter: { readonly where: string } }
  | { readonly map: { readonly select: string } }
  | { readonly flatMap: { readonly select: string } }
  | { readonly skip: { readonly count: number } }
  | { readonly take: { readonly count: number } }
  | { readonly distinct: { readonly by?: string } }
  | { readonly window: { readonly count: number } };

/** Keeps the elements for which `where` is `true`. */
function filter(where: string): Operator {
  return { filter: { where } };
}

/** Replaces each element by the value of `select`. */
function map(select: string): Operator {
  return { map: { select } };
}

/** `select` yields a list for each element; the lists are concatenated in order, one level deep. */
function flatMap(select: string): Operator {
  return { flatMap: { select } };
}

/** Drops the first `count` elements. */
function skip(count: number): Operator {
  return { skip: { count } };
}

/** Keeps the first `count` elements. */
function take(count: number): Operator {
  return { take: { count } };
}

/** Keeps the first element of each value of `by`, or of the element itself when `by` is absent. */
function distinct(by?: string): Operator {
  return { distinct: by === undefined ? {} : { by } };
}

/** Replaces the list by consecutive lists of `count` elements, the last one possibly shorter. */
function window(count: number): Operator {
  return { window: { count } };
}

/** The operator factory: `ops.filter(where)`, `ops.map(select)`, `ops.flatMap(select)`, `ops.skip(n)`, `ops.take(n)`, `ops.distinct(by?)`, `ops.window(count)`. */
export const ops = Object.freeze({ filter, map, flatMap, skip, take, distinct, window });
