package dev.sagatoctopusorchestrator.sdk;

import java.util.concurrent.Callable;
import java.util.concurrent.CompletableFuture;

/**
 * What the groups of the client ({@link Schedules}, {@link BatchOperations}, {@link BusinessIds}) and the run
 * operations need of it. The client supplies it, so the token stays in the client and its transport: a group holds
 * nothing it could show.
 */
interface Calls {
    /** Sends a request. Every retry of it sends the same body and the same idempotency key. */
    RestClient.Response send(RestClient.Request request);

    /** The {@code traceparent} to send: the explicit one, else the ambient context; null when there is none. */
    String trace(String explicit);

    /** The id of a workflow given by id or by name. */
    String resolveWorkflow(String workflow);

    /** Runs a blocking call on the client's executor; cancelling the future interrupts the call. */
    <T> CompletableFuture<T> async(Callable<T> body);
}
