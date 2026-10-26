package dev.sagatoctopusorchestrator.sdk.worker;

import java.util.List;

/** The run-local identity supplied for a task by worker protocol 1.5. */
public record ExecutionInstanceIdentity(List<ExecutionScope> instancePath, String definitionNodeId) {
    public ExecutionInstanceIdentity {
        instancePath = List.copyOf(instancePath);
    }
}
