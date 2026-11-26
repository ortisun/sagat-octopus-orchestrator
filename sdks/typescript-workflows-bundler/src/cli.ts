#!/usr/bin/env node
/*
 * sagat-workflows bundle <entry> [--out <directory>] [--refuse <module prefix>]...
 *
 * Writes workflows.bundle.js and workflows.manifest.json (the code digest of each workflow type) to the directory,
 * `dist/workflows` by default. Run it with the Node the workers run: the digest names Node's version and its ICU data.
 */
import { bundleWorkflows, WorkflowImportRefused } from './bundle.js';

const USAGE = 'usage: sagat-workflows bundle <entry> [--out <directory>] [--refuse <module prefix>]...';

async function main(argv: string[]): Promise<number> {
  const [command, ...rest] = argv;
  if (command !== 'bundle') {
    process.stderr.write(`${USAGE}\n`);
    return 2;
  }
  let entry: string | undefined;
  let outDir = 'dist/workflows';
  const refuse: string[] = [];
  for (let i = 0; i < rest.length; i++) {
    const argument = rest[i]!;
    if (argument === '--out' && rest[i + 1]) outDir = rest[++i]!;
    else if (argument === '--refuse' && rest[i + 1]) refuse.push(rest[++i]!);
    else if (!argument.startsWith('--') && entry === undefined) entry = argument;
    else {
      process.stderr.write(`${USAGE}\n`);
      return 2;
    }
  }
  if (!entry) {
    process.stderr.write(`${USAGE}\n`);
    return 2;
  }
  try {
    const result = await bundleWorkflows({ entry, outDir, refuse });
    for (const workflow of result.manifest.workflows) process.stdout.write(`${workflow.workflowType} ${workflow.digest}\n`);
    process.stdout.write(`wrote ${result.bundlePath} and ${result.manifestPath} (${result.manifest.runtime})\n`);
    return 0;
  } catch (error) {
    const message = error instanceof WorkflowImportRefused ? error.message : error instanceof Error ? error.message : String(error);
    process.stderr.write(`sagat-workflows: ${message}\n`);
    return 1;
  }
}

process.exitCode = await main(process.argv.slice(2));
