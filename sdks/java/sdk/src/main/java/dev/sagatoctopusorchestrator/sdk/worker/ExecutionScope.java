package dev.sagatoctopusorchestrator.sdk.worker;

/** One repeated scope that addresses a runtime activation of a definition node. */
public record ExecutionScope(String scopeNodeId, int iterationIndex) {}
