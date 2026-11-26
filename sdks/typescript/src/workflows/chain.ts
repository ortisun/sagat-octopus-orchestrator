/*
 * The command chain and the body digests of design 044, Technical Plan 6.2, computed by the worker from the commands its
 * replay makes. The server keeps the same chain over the bodies it received and refuses a completion whose chain differs.
 */
import { createHash } from 'node:crypto';
import { DESCRIPTOR_FIELD, rfc8785, type WorkflowCommand } from './context.js';

/** The chain of a run with no command: 32 zero bytes. */
export const EMPTY_CHAIN: Uint8Array = new Uint8Array(32);

export function sha256(bytes: Uint8Array | string): Uint8Array {
  return new Uint8Array(createHash('sha256').update(bytes).digest());
}

export function hex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('hex');
}

/** A value's descriptor: the SHA-256 and the size of its RFC 8785 bytes, whatever carries it (inline or a blob). */
export interface ValueDescriptor { sha256: string; size: number }

export function descriptorOf(value: unknown): ValueDescriptor {
  const bytes = Buffer.from(rfc8785(value), 'utf8');
  return { sha256: hex(sha256(bytes)), size: bytes.length };
}

/** The body that is hashed: the value field of the kind enters it as its descriptor, whatever carries the value. */
export function hashedBody(command: Pick<WorkflowCommand, 'kind' | 'body'>): Record<string, unknown> {
  const field = DESCRIPTOR_FIELD[command.kind];
  if (!field || !(field in command.body)) return command.body;
  return { ...command.body, [field]: descriptorOf(command.body[field]) };
}

/** The SHA-256 of a command's canonical body (with descriptors). */
export function bodyDigest(command: Pick<WorkflowCommand, 'kind' | 'body'>): Uint8Array {
  return sha256(Buffer.from(rfc8785(hashedBody(command)), 'utf8'));
}

/** `d(k)`: the RFC 8785 bytes of `{"seq", "kind", "target", "body"}` for command k. */
export function commandDigestInput(seq: number, kind: string, target: string, body: Uint8Array): Uint8Array {
  return Buffer.from(rfc8785({ seq, kind, target, body: hex(body) }), 'utf8');
}

/** `H(k) = SHA-256(H(k − 1) ‖ uint32 big-endian length of d(k) ‖ d(k))`. */
export function extendChain(previous: Uint8Array, seq: number, kind: string, target: string, body: Uint8Array): Uint8Array {
  if (previous.length !== 32) throw new RangeError('a chain is 32 bytes');
  const input = commandDigestInput(seq, kind, target, body);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(input.length, 0);
  return sha256(Buffer.concat([previous, length, input]));
}
