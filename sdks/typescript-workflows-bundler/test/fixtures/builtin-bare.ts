import { readFileSync } from 'fs';

export async function leak(): Promise<string> {
  return readFileSync('/etc/hostname', 'utf8');
}
