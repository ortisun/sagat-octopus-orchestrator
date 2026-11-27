/*
 * `sagat-workflows bundle` (design 044, Technical Plan 1.2 and 3.2, Decision 044-012): one script holding the workflow
 * code, every module it imports and the SDK's determinism runtime, which the worker compiles once per digest and
 * evaluates in each run's context; and the manifest with each workflow type's code digest.
 *
 * The SDK's import `@sagat-octopus-orchestrator/sdk/workflows` resolves, under the export condition `sagat-workflow`,
 * to the runtime alone; it is imported first, so it replaces the context's globals before any workflow module runs.
 * Imports of Node's built-in modules (with or without `node:`), of the SDK's activity side and of the patterns the
 * caller names (its activity modules) are refused here, before any code runs.
 */
import { mkdir, writeFile } from 'node:fs/promises';
import { builtinModules } from 'node:module';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { existsSync } from 'node:fs';
import { build, type Plugin } from 'esbuild';
import { WorkflowThread, manifestOf, type WorkflowManifest } from '@sagat-octopus-orchestrator/sdk/workflows';

export const BUNDLE_FILE = 'workflows.bundle.js';
export const MANIFEST_FILE = 'workflows.manifest.json';
const SDK = '@sagat-octopus-orchestrator/sdk';
const SDK_WORKFLOWS = `${SDK}/workflows`;
const BUILTINS = new Set(builtinModules.flatMap((name) => [name, name.split('/')[0]!]));

export interface BundleOptions {
  /** The module whose exported functions are the workflows: each export's name is its workflow type. */
  entry: string;
  /** The directory the bundle and the manifest are written to; nothing is written without it. */
  outDir?: string;
  /** Module specifiers refused besides the built-ins, as prefixes (for example `./activities`). */
  refuse?: string[];
}

export interface BundleResult { code: string; manifest: WorkflowManifest; bundlePath?: string; manifestPath?: string }

export class WorkflowImportRefused extends Error {
  readonly specifiers: string[];
  constructor(specifiers: string[], message: string) {
    super(message);
    this.name = 'WorkflowImportRefused';
    this.specifiers = specifiers;
  }
}

function refusal(path: string, refuse: readonly string[]): string | undefined {
  const bare = path.startsWith('node:') ? path.slice('node:'.length) : path;
  if (path.startsWith('node:') || BUILTINS.has(bare) || BUILTINS.has(bare.split('/')[0]!)) {
    return `workflow code may not import the built-in module ${path}: it would reach the host's input and output`;
  }
  if (path === SDK || (path.startsWith(`${SDK}/`) && path !== SDK_WORKFLOWS)) {
    return `workflow code may not import ${path}: only ${SDK_WORKFLOWS} runs inside a workflow`;
  }
  const refused = refuse.find((prefix) => path === prefix || path.startsWith(prefix.endsWith('/') ? prefix : `${prefix}/`) || path.startsWith(`${prefix}.`));
  return refused === undefined ? undefined : `workflow code may not import ${path}: it is refused by the bundle's options (${refused})`;
}

function refusing(refuse: readonly string[]): Plugin {
  return {
    name: 'sagat-workflows-refuse',
    setup(context) {
      context.onResolve({ filter: /.*/ }, (args) => {
        const reason = refusal(args.path, refuse);
        return reason === undefined ? undefined : { errors: [{ text: reason, detail: args.path }] };
      });
    },
  };
}

/** The directory of the nearest `package.json` above a file: module paths in the bundle are relative to it. */
function projectRoot(file: string): string {
  let directory = dirname(file);
  for (;;) {
    if (existsSync(join(directory, 'package.json'))) return directory;
    const parent = dirname(directory);
    if (parent === directory) return dirname(file);
    directory = parent;
  }
}

export async function bundleWorkflows(options: BundleOptions): Promise<BundleResult> {
  const entry = resolve(options.entry);
  const root = projectRoot(entry);
  const importPath = `./${relative(dirname(entry), entry).split(sep).join('/')}`;
  let result;
  try {
    result = await build({
      stdin: {
        // The runtime first, so that it replaces the globals before the workflow modules are evaluated.
        contents: `import { __sagatInstall } from ${JSON.stringify(SDK_WORKFLOWS)};\nimport * as workflows from ${JSON.stringify(importPath)};\n__sagatInstall(workflows);\n`,
        resolveDir: dirname(entry),
        sourcefile: 'sagat-workflows-entry.js',
        loader: 'js',
      },
      absWorkingDir: root,
      bundle: true,
      write: false,
      format: 'iife',
      platform: 'neutral',
      target: 'es2022',
      conditions: ['sagat-workflow'],
      mainFields: ['module', 'main'],
      keepNames: true,
      // A dynamic import() becomes a call of esbuild's own require shim, which fails inside the context at the same point
      // live and in replay; left as syntax, Node's module loader would settle it outside the activation.
      supported: { 'dynamic-import': false },
      legalComments: 'none',
      charset: 'utf8',
      logLevel: 'silent',
      plugins: [refusing(options.refuse ?? [])],
    });
  } catch (error) {
    const errors = (error as { errors?: { text: string; detail?: unknown }[] }).errors ?? [];
    const refused = errors.filter((item) => typeof item.detail === 'string').map((item) => item.detail as string);
    if (refused.length > 0) throw new WorkflowImportRefused(refused, errors.map((item) => item.text).join('\n'));
    throw error;
  }
  const output = result.outputFiles[0];
  if (!output) throw new Error('esbuild wrote no bundle');
  const code = output.text;
  // The workflow types are the bundle's own list, read on a workflow thread: the bundle's top level runs in a context.
  const thread = new WorkflowThread();
  let workflowTypes: readonly string[];
  try {
    workflowTypes = (await thread.loadBundle(code, BUNDLE_FILE)).workflowTypes;
  } catch (error) {
    if (error instanceof Error && /\(NotInWorkflow\)/.test(error.message)) {
      throw new Error('a workflow module calls the clock, the random generator or a workflow function at its top level, '
        + 'where no activation runs; move the call into a workflow function (NotInWorkflow)');
    }
    throw error;
  } finally {
    await thread.terminate();
  }
  if (workflowTypes.length === 0) throw new Error(`${basename(entry)} exports no workflow function`);
  const manifest = manifestOf(code, BUNDLE_FILE, workflowTypes);
  if (!options.outDir) return { code, manifest };
  await mkdir(options.outDir, { recursive: true });
  const bundlePath = join(options.outDir, BUNDLE_FILE);
  const manifestPath = join(options.outDir, MANIFEST_FILE);
  await writeFile(bundlePath, code);
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  return { code, manifest, bundlePath, manifestPath };
}
