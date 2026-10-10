import type { components } from './schema';

/** Names for the shapes of the contract the Studio works with. */
type Schemas = components['schemas'];

export type Workflow = Schemas['WorkflowDraft'];
export type WorkflowSummary = Schemas['WorkflowDraft'];
export type VersionSummary = Schemas['VersionSummary'];
export type WorkflowVersion = Schemas['WorkflowVersion'];
export type Validation = Schemas['Validation'];
export type DefinitionProblem = Schemas['DefinitionProblem'];
export type DefinitionDiff = Schemas['DefinitionDiff'];
export type DiffFinding = Schemas['DiffFinding'];
export type Run = Schemas['Run'];
export type RuntimeGraph = Schemas['RuntimeGraph'];
export type RuntimeNode = Schemas['RuntimeNode'];
export type RuntimeEdge = Schemas['RuntimeEdge'];
export type GraphChanges = Schemas['GraphChanges'];
export type GraphEvent = Schemas['GraphEvent'];
export type JournalEvent = Schemas['JournalEvent'];
export type OperationRequest = Schemas['OperationRequest'];
export type OperationResult = Schemas['OperationResult'];
export type ApiErrorBody = Schemas['Error'];

export type NodeStatus = RuntimeNode['status'];

/** A page of a listing: the items and where the next page starts. */
export interface Page<T> {
  items: T[];
  nextCursor?: string | null;
}
