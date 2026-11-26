/*
 * Test bundles for the SDK's own tests. A real bundle is made by `sagat-workflows bundle` (esbuild, its own package);
 * the SDK's tests make one without esbuild: the compiled runtime (`context.js`, one module that imports nothing) with
 * its `export` keywords removed, followed by the workflow functions' source text, in one function scope, which is what
 * the bundler's output amounts to. Workflow functions written in a test may therefore name the runtime's classes
 * (`WorkflowFailure`, `SagaCompensated`...) directly.
 */
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { WorkflowBundle, type BundleText, type WorkflowContext } from '../../src/workflows/index.js';

const RUNTIME = fileURLToPath(new URL('../../src/workflows/context.js', import.meta.url));

export type TestWorkflow = (ctx: WorkflowContext, input: any) => Promise<unknown>;

/** The source of a test bundle: the runtime, an optional prelude (a workflow module's top level), the workflows. */
export function bundleSource(workflows: Record<string, TestWorkflow | string>, prelude = ''): string {
  const runtime = readFileSync(RUNTIME, 'utf8').replace(/^export (?=(?:async )?function |class |const |let )/gm, '');
  assert.doesNotMatch(runtime, /^\s*(?:export|import)\b/m, 'the runtime module must import and re-export nothing');
  const entries = Object.entries(workflows).map(([name, fn]) => `${JSON.stringify(name)}: (${typeof fn === 'string' ? fn : fn.toString()})`);
  return `(() => {\n${runtime}\n${prelude}\n__sagatInstall({\n${entries.join(',\n')}\n});\n})();\n`;
}

/** A test bundle's text, for the workflow thread (the test environment and the replayer). */
export function bundleOf(workflows: Record<string, TestWorkflow | string>, prelude = ''): BundleText {
  return { code: bundleSource(workflows, prelude), filename: 'test-bundle.js' };
}

/** A test bundle compiled in the test's own thread, for the tests that drive a `CodeRun` directly. */
export function localBundle(workflows: Record<string, TestWorkflow | string>, prelude = ''): WorkflowBundle {
  return new WorkflowBundle(bundleSource(workflows, prelude), 'test-bundle.js');
}

export const SEED = '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f';
