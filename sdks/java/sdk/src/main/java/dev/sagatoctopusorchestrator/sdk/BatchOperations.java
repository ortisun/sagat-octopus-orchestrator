package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

/**
 * Batch jobs over many runs: preview, create, inspect, cancel and list the items (reached from
 * {@link SagatClient#batch()}). Every call that changes something carries one {@code Idempotency-Key}: the caller's
 * ({@link CallOptions#idempotencyKey}) or one generated once for the call and reused by every retry of it. Every call
 * has a blocking form and an {@code ...Async} form.
 */
public final class BatchOperations {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final Calls calls;

    BatchOperations(Calls calls) {
        this.calls = calls;
    }

    @Override
    public String toString() {
        return "BatchOperations";
    }

    private static String path(String namespaceId) {
        return "/namespaces/" + Wire.uuid(namespaceId, "a namespace id") + "/batch-operations";
    }

    private static String path(String namespaceId, String batchId, String tail) {
        return path(namespaceId) + "/" + Wire.uuid(batchId, "a batch id") + tail;
    }

    private RestClient.Response send(
            String method, String path, Map<String, String> query, JsonNode body, String traceparent, String key) {
        return calls.send(new RestClient.Request(method, path, query, body, key, calls.trace(traceparent), true));
    }

    /**
     * Counts what a selector matches, up to {@code maxItems}; changes nothing.
     *
     * @param maxItems the cap of the preview; null leaves the server's default
     */
    public BatchPreview preview(String namespaceId, BatchSelector selector, Integer maxItems, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = path(namespaceId) + "/preview";
        if (selector == null) {
            throw SagatException.invalid("a selector is an object");
        }
        ObjectNode body = F.objectNode();
        body.set("selector", selector.toJson());
        if (maxItems != null) {
            body.put("maxItems", maxItems.intValue());
        }
        return BatchPreview.parse(send("POST", path, null, body, opts.traceparent(), null).body());
    }

    public CompletableFuture<BatchPreview> previewAsync(
            String namespaceId, BatchSelector selector, Integer maxItems, CallOptions options) {
        return calls.async(() -> preview(namespaceId, selector, maxItems, options));
    }

    /** Creates a durable job. A request sent again with the same key and body is answered with the first job. */
    public BatchJob create(String namespaceId, BatchRequest request, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = path(namespaceId);
        if (request == null) {
            throw SagatException.invalid("a batch request is an object");
        }
        JsonNode body = request.toJson();
        return BatchJob.parse(send("POST", path, null, body, opts.traceparent(), Wire.key(opts.idempotencyKey())).body());
    }

    public CompletableFuture<BatchJob> createAsync(String namespaceId, BatchRequest request, CallOptions options) {
        return calls.async(() -> create(namespaceId, request, options));
    }

    /** A job with its counts. */
    public BatchJob get(String namespaceId, String batchId, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        return BatchJob.parse(send("GET", path(namespaceId, batchId, ""), null, null, opts.traceparent(), null).body());
    }

    public CompletableFuture<BatchJob> getAsync(String namespaceId, String batchId, CallOptions options) {
        return calls.async(() -> get(namespaceId, batchId, options));
    }

    /** One page of jobs, optionally only those in a state. */
    public Page<BatchJob> list(String namespaceId, BatchListOptions options) {
        BatchListOptions opts = options == null ? BatchListOptions.create() : options;
        return Wire.page(send("GET", path(namespaceId), query(opts), null, opts.traceparent(), null).body(),
                "a page of batch jobs", BatchJob::parse);
    }

    public CompletableFuture<Page<BatchJob>> listAsync(String namespaceId, BatchListOptions options) {
        return calls.async(() -> list(namespaceId, options));
    }

    /** Stops admitting items; what was applied stays applied. A job that already ended is {@code CONFLICT}. */
    public BatchJob cancel(String namespaceId, String batchId, String reason, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = path(namespaceId, batchId, "/cancel");
        ObjectNode body = F.objectNode();
        body.put("reason", Wire.text(reason, "a reason", false));
        return BatchJob.parse(send("POST", path, null, body, opts.traceparent(), Wire.key(opts.idempotencyKey())).body());
    }

    public CompletableFuture<BatchJob> cancelAsync(String namespaceId, String batchId, String reason, CallOptions options) {
        return calls.async(() -> cancel(namespaceId, batchId, reason, options));
    }

    /** One page of what the job did to each run, optionally only the items in a state. */
    public Page<BatchItem> listItems(String namespaceId, String batchId, BatchListOptions options) {
        BatchListOptions opts = options == null ? BatchListOptions.create() : options;
        return Wire.page(
                send("GET", path(namespaceId, batchId, "/items"), query(opts), null, opts.traceparent(), null).body(),
                "a page of batch items", BatchItem::parse);
    }

    public CompletableFuture<Page<BatchItem>> listItemsAsync(String namespaceId, String batchId, BatchListOptions options) {
        return calls.async(() -> listItems(namespaceId, batchId, options));
    }

    private static Map<String, String> query(BatchListOptions options) {
        Map<String, String> query = new LinkedHashMap<>();
        query.put("cursor", options.cursor());
        query.put("limit", options.limit() == null ? null : Integer.toString(options.limit()));
        query.put("state", options.state());
        return query;
    }
}
