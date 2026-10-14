package dev.sagatoctopusorchestrator.sdk.worker;

/** How a worker receives tasks. */
public enum WorkerTransport {
    /** Stream on protocol 1.4, poll older servers, and fall back only when StreamTasks is unimplemented. */
    AUTO,
    /** Always use unary PollTasks. */
    POLL,
    /** Require protocol 1.4 StreamTasks. */
    STREAM
}
