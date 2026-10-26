package dev.sagatoctopusorchestrator.sdk.workflow;

/** Optional workflow-wide execution ceilings. A null member keeps the platform default. */
public record WorkflowLimits(Integer maxInstances, Integer maxDepth) {}
