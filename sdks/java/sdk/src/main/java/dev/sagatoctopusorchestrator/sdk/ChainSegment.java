package dev.sagatoctopusorchestrator.sdk;

/** One immutable run link in an execution chain. */
public record ChainSegment(int index, String runId, String predecessorRunId, String successorRunId) {}
