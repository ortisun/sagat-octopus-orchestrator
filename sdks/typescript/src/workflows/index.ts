/*
 * `@sagat-octopus-orchestrator/sdk/workflows`: workflow as code for TypeScript (design 044).
 *
 * In a workflow bundle this import resolves, through the package export condition `sagat-workflow`, to the runtime of
 * `context.ts` alone. In the host process it resolves here: the same workflow API (whose calls throw outside a workflow
 * context), the replayer, the test environment and the pieces a worker uses. Loading it checks that Node is 22.8 or
 * later, which `vm.constants.DONT_CONTEXTIFY` needs; the package's other entry points keep Node 20.
 */
import { checkNodeVersion } from './preconditions.js';

checkNodeVersion();

export {
  activity, approval, cancellation, child, continueAsNew, continueAsNewSuggested, deprecatePatch, historyLimitReached, now,
  onSignal, onUpdate, patched, random, randomInt, requestCancellation, saga, script, setState, sideEffect, signal, sleep, uuid,
  waitForEvent, acknowledgeCancellation,
  ActivityError, ChildWorkflowError, CommandBatchLimitExceeded, InlineValuesExceeded, NotInWorkflow, SagaCompensated,
  SagaCompensationFailed, SagaPartial, TimerCancelled, WorkflowFailure,
  BRIDGE_VERSION, MAX_BLOBS, MAX_BLOB_CARRIED_BYTES, MAX_FUNCTION_COMMANDS, MAX_FUNCTION_METADATA_BYTES, MAX_INLINE_BYTES,
  MAX_SIDE_EFFECT_BYTES, Xoshiro256, rfc8785,
} from './context.js';
export type {
  ActivityOptions, ActivityRetryPolicy, Cancellation, ChildOptions, CommandKind, CommandPromise, ContinueAsNewOptions,
  HistoryEvent, SagaOptions, SagaScope, ScriptOptions, WaitOptions, WorkflowCommand, WorkflowContext, WorkflowFunction,
  WorkflowTaskFailure,
} from './context.js';
export { DEFAULT_CACHE_SIZE, WorkflowRunCache } from './cache.js';
export { bodyDigest, descriptorOf, extendChain, EMPTY_CHAIN, hex } from './chain.js';
export { ActivationCutter } from './history.js';
export { MANIFEST_FORMAT, ManifestMismatch, SDK_NAME, WORKFLOW_TYPE, codeDigest, contentOf, manifestOf, runtimeOf, verifyManifest } from './manifest.js';
export type { ManifestWorkflow, WorkflowManifest } from './manifest.js';
export { SDK_VERSION } from '../config.js';
export type { OpenWindow, ReplayActivation } from './history.js';
export { checkWorkflowRuntime, WorkflowRuntimeUnavailable } from './preconditions.js';
export { MalformedHistory, Replayer, UnsupportedExportFormat, readExport, replayEvents } from './replayer.js';
export type { ExportHeader, ReplayOptions, ReplayVerdict } from './replayer.js';
export { CodeRun, prepare } from './run.js';
export type { CodeRunOptions, Completion, CompletionAnswer, Failure, PreparedCommand, ReplayStep } from './run.js';
export { ActivationDeadlineExceeded, InvalidWorkflowBundle, WorkflowBundle } from './sandbox.js';
export { StubFailure, WorkflowTestEnvironment } from './testing.js';
export type { BundleText, Stub, TestEnvironmentOptions, TestOutcome, TestRunStatus } from './testing.js';
export { ThreadedRun, WorkflowThread, WorkflowThreadError } from './threaded.js';
export type { LoadedBundle, RunHandle } from './threaded.js';
