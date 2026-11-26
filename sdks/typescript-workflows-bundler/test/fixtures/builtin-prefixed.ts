import { randomUUID } from 'node:crypto';

export async function leak(): Promise<string> {
  return randomUUID();
}
