import { isSafeNumber, parse } from 'lossless-json';

export class UnsafeJsonNumber extends Error { constructor() { super('Use strings for exact integers and decimals.'); this.name = 'UnsafeJsonNumber'; } }

/** Reject numeric text before conversion can round a value that will be sent to a database. */
export function parseAuthoringJson(text: string): unknown {
  return parse(text, undefined, { parseNumber: raw => {
    const value = Number(raw);
    if (!isSafeNumber(raw) || Number.isInteger(value) && !Number.isSafeInteger(value)) throw new UnsafeJsonNumber();
    return value;
  } });
}
