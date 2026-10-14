package dev.sagatoctopusorchestrator.sdk.worker;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;
import java.io.InputStream;
import java.util.Map;

/** What an activity knows about the attempt it is running (SPEC section 6.3). */
public interface ActivityContext {
    /** The activity's name, as registered. */
    String activityName();

    String taskId();

    String runId();

    /** The node id of the step in the workflow definition (for example {@code charge}), from the step document. */
    String node();

    /** The platform's id of the step ({@code step_id} of the lease, a UUID), which the reports carry. */
    String stepId();

    int attempt();

    long generation();

    /** Stable across the attempts of the step: send it to the provider of an external effect. */
    String idempotencyKey();

    /** The W3C trace context of the run; empty when the lease carried none. */
    String traceparent();

    TaskKind taskKind();

    /** For a compensation or a reconciliation: the forward node it is about; empty otherwise. */
    String targetNode();

    /** The key that identifies the effect of that node at the provider; send it to the provider. */
    String targetIdempotencyKey();

    /** The outputs of the steps that completed before this one, by node. */
    Map<String, JsonNode> steps();

    /** The end of the lease; moves forward with every heartbeat. */
    Instant deadline();

    /** Cancelled when the run is cancelled, the lease is lost or the host stops. */
    CancellationSignal cancellation();

    ActivityLogger logger();

    /**
     * The secrets the step declares, resolved once per attempt, on first use (protocol 1.2; against protocol 1 this
     * raises {@code UNSUPPORTED_PROTOCOL}). The values are dropped when the handler returns.
     */
    Secrets secrets();

    /** Streams a one-pass source and verifies the returned descriptor against the bytes sent. */
    BlobDescriptor writeBlob(InputStream source, BlobWriteOptions options);

    default BlobDescriptor writeBlob(InputStream source) {
        return writeBlob(source, BlobWriteOptions.defaults());
    }

    /** A verified stream. A clean EOF means the size and SHA-256 matched the supplied descriptor. */
    InputStream readBlob(BlobDescriptor descriptor);
}
