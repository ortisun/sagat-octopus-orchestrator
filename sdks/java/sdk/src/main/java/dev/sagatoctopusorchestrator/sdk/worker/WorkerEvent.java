package dev.sagatoctopusorchestrator.sdk.worker;

import java.time.Duration;

/** Something that happened in a worker host: the hook for metrics (SPEC section 6.8). */
public sealed interface WorkerEvent {
    /** The host registered and agreed a protocol version. */
    record Registered(String protocolVersion) implements WorkerEvent {}

    /** A poll answered with {@code tasks} tasks. */
    record Poll(int tasks) implements WorkerEvent {}

    record TaskTaken(String taskId, String activity, int attempt, String traceparent) implements WorkerEvent {}

    record TaskCompleted(String taskId, Duration duration) implements WorkerEvent {}

    record TaskFailed(String taskId, String code, boolean retryable, Duration duration) implements WorkerEvent {}

    /** The lease was lost or the report was answered {@code STALE_ATTEMPT}: nothing was (or will be) reported. */
    record TaskStale(String taskId) implements WorkerEvent {}

    /** A report could not be delivered. */
    record TaskDropped(String taskId, String reason) implements WorkerEvent {}

    record Heartbeat(String taskId) implements WorkerEvent {}

    /** A receive window failed; the code is bounded and carries no remote exception text. */
    record ReceiveFailed(String code) implements WorkerEvent {}

    /** The session was lost or the service was unreachable and the host goes on after a pause or a new registration. */
    record Reconnect() implements WorkerEvent {}
}
