import { WorkerHost } from '@sagat-octopus-orchestrator/sdk';

export async function leak(): Promise<string> {
  return typeof WorkerHost;
}
