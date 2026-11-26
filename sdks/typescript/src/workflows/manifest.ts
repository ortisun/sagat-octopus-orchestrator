/*
 * The code digest and the manifest of a TypeScript workflow bundle (design 044, Technical Plan 3.2). The digest is the
 * SHA-256 of the RFC 8785 JSON of `{"language", "runtime", "sdkName", "sdkVersion", "workflowType", "content"}`, where
 * `content` is the SHA-256 of the bundle file the worker loads and `runtime` names Node and its ICU data. The bundler
 * writes the manifest; a worker computes the same digests from the file it loads, under its own Node, and refuses to
 * start when they differ, so that a run is never served by code its version did not name.
 */
import { createHash } from 'node:crypto';
import { SDK_VERSION } from '../config.js';
import { rfc8785 } from './context.js';

export const SDK_NAME = '@sagat-octopus-orchestrator/sdk';
export const MANIFEST_FORMAT = 1;
/** Requirement 044-001: a workflow type is 1 to 48 characters. */
export const WORKFLOW_TYPE = /^[A-Za-z][A-Za-z0-9_-]{0,47}$/;

export interface ManifestWorkflow { workflowType: string; digest: string }

export interface WorkflowManifest {
  manifestFormat: number;
  language: 'typescript';
  runtime: string;
  sdkName: string;
  sdkVersion: string;
  /** The bundle's file name, beside the manifest. */
  bundle: string;
  /** The SHA-256 of the bundle file, 64 lowercase hexadecimal characters. */
  content: string;
  workflows: ManifestWorkflow[];
}

export class ManifestMismatch extends Error {
  readonly reason: 'FORMAT' | 'CONTENT' | 'RUNTIME' | 'SDK' | 'DIGEST' | 'WORKFLOW_TYPE';
  constructor(reason: ManifestMismatch['reason'], message: string) {
    super(message);
    this.name = 'ManifestMismatch';
    this.reason = reason;
  }
}

/** The runtime's name and full version with its locale data, for example `node 22.11.0 icu 75.1`. */
export function runtimeOf(versions: { node: string; icu?: string | undefined } = process.versions): string {
  return versions.icu ? `node ${versions.node} icu ${versions.icu}` : `node ${versions.node}`;
}

export function contentOf(bundle: Uint8Array | string): string {
  return createHash('sha256').update(bundle).digest('hex');
}

/** The code digest of one workflow type: `sha256:` and 64 lowercase hexadecimal characters. */
export function codeDigest(input: { runtime: string; sdkVersion: string; workflowType: string; content: string; sdkName?: string }): string {
  const canonical = rfc8785({
    language: 'typescript', runtime: input.runtime, sdkName: input.sdkName ?? SDK_NAME, sdkVersion: input.sdkVersion,
    workflowType: input.workflowType, content: input.content,
  });
  return `sha256:${createHash('sha256').update(canonical, 'utf8').digest('hex')}`;
}

/** The manifest of a bundle under this process's runtime and SDK. */
export function manifestOf(bundle: Uint8Array | string, bundleFile: string, workflowTypes: readonly string[],
  runtime = runtimeOf()): WorkflowManifest {
  const bad = workflowTypes.filter((type) => !WORKFLOW_TYPE.test(type));
  if (bad.length > 0) {
    throw new ManifestMismatch('WORKFLOW_TYPE', `a workflow type is 1 to 48 characters of ^[A-Za-z][A-Za-z0-9_-]{0,47}$: ${bad.join(', ')}`);
  }
  const content = contentOf(bundle);
  return {
    manifestFormat: MANIFEST_FORMAT, language: 'typescript', runtime, sdkName: SDK_NAME, sdkVersion: SDK_VERSION, bundle: bundleFile, content,
    workflows: [...workflowTypes].sort().map((workflowType) => ({ workflowType, digest: codeDigest({ runtime, sdkVersion: SDK_VERSION, workflowType, content }) })),
  };
}

/**
 * Checks a manifest against the bundle a worker loads, under this process's runtime and SDK: the content, the runtime,
 * the SDK and every digest. Answers the (workflow type, digest) pairs the worker registers.
 */
export function verifyManifest(manifest: WorkflowManifest, bundle: Uint8Array | string, runtime = runtimeOf()): ManifestWorkflow[] {
  if (manifest.manifestFormat !== MANIFEST_FORMAT || manifest.language !== 'typescript') {
    throw new ManifestMismatch('FORMAT', `this SDK reads manifest format ${MANIFEST_FORMAT} of TypeScript bundles`);
  }
  const content = contentOf(bundle);
  if (manifest.content !== content) throw new ManifestMismatch('CONTENT', 'the bundle file is not the one the manifest names');
  if (manifest.runtime !== runtime) {
    throw new ManifestMismatch('RUNTIME', `the manifest was made under ${manifest.runtime}; this worker runs ${runtime}: bundle again with the worker's Node`);
  }
  if (manifest.sdkName !== SDK_NAME || manifest.sdkVersion !== SDK_VERSION) {
    throw new ManifestMismatch('SDK', `the manifest names ${manifest.sdkName} ${manifest.sdkVersion}; this worker runs ${SDK_NAME} ${SDK_VERSION}`);
  }
  for (const workflow of manifest.workflows) {
    if (!WORKFLOW_TYPE.test(workflow.workflowType)) throw new ManifestMismatch('WORKFLOW_TYPE', `${workflow.workflowType} is not a workflow type`);
    const digest = codeDigest({ runtime, sdkVersion: SDK_VERSION, workflowType: workflow.workflowType, content });
    if (workflow.digest !== digest) throw new ManifestMismatch('DIGEST', `the digest of ${workflow.workflowType} does not follow from the bundle`);
  }
  return manifest.workflows.map((workflow) => ({ ...workflow }));
}
