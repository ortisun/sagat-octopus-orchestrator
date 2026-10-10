import { randomBytes } from 'node:crypto';
import { SagatError } from './errors.js';

const PATTERN = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/;

/** True for a W3C `traceparent` of version 00 with a non-zero trace and parent id. */
export function isTraceparent(value: string): boolean {
  const match = PATTERN.exec(value);
  return !!match && match[1] !== '0'.repeat(32) && match[2] !== '0'.repeat(16);
}

/** Raises `BAD_REQUEST` for a malformed `traceparent`, before it is sent. */
export function checkTraceparent(value: string): string {
  if (!isTraceparent(value)) {
    throw new SagatError({ code: 'BAD_REQUEST', message: 'traceparent is not a W3C trace context (00-<32 hex>-<16 hex>-<2 hex>)' });
  }
  return value;
}

/** The trace id of a valid `traceparent`; undefined otherwise. */
export function traceIdOf(value: string | undefined | null): string | undefined {
  if (!value) return undefined;
  const match = PATTERN.exec(value);
  return match ? match[1] : undefined;
}

/** A fresh sampled trace context, for an application that has none. */
export function newTraceparent(): string {
  return `00-${randomBytes(16).toString('hex')}-${randomBytes(8).toString('hex')}-01`;
}
