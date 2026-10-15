package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.worker.BlobDescriptor;
import java.io.FilterInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Instant;
import java.time.OffsetDateTime;
import java.time.format.DateTimeParseException;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.NoSuchElementException;
import java.util.UUID;
import java.util.concurrent.Callable;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.regex.Pattern;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * The client of the REST API (sdks/SPEC.md section 3). Every call has a blocking form and an {@code ...Async} form
 * that returns a {@link CompletableFuture}; the asynchronous form runs the blocking one on an executor, so cancelling
 * the future interrupts a call that is waiting. A client is thread-safe; {@link #close()} releases the executor it
 * owns.
 */
public final class SagatClient implements AutoCloseable {
    private static final Pattern UUID_TEXT = Pattern.compile("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", Pattern.CASE_INSENSITIVE);
    private static final Pattern KEY = Pattern.compile("^[\\x21-\\x7e]{8,160}$");
    private static final Pattern APPROVAL_DECISION_KEY = Pattern.compile("^[\\x21-\\x7e]{1,128}$");
    private static final Pattern APPROVAL_REASON_CODE = Pattern.compile("^[A-Za-z0-9_.:-]{1,64}$");
    private static final Pattern DECLARED_OPERATION_NAME = Pattern.compile("^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$");
    private static final Pattern UPDATE_KEY = Pattern.compile("^[\\x21-\\x7e]{1,128}$");
    private static final long NAME_CACHE_MILLIS = 60_000;

    private final ClientConfig config;
    private final RestClient http;
    private final Map<String, CachedId> names = new ConcurrentHashMap<>();
    private final ExecutorService executor;
    private final boolean ownsExecutor;

    private record CachedId(String id, long atNanos) {}

    public SagatClient(ClientConfig config) {
        this.config = config;
        this.http = new RestClient(config);
        if (config.asyncExecutor() != null) {
            this.executor = config.asyncExecutor();
            this.ownsExecutor = false;
        } else {
            AtomicInteger counter = new AtomicInteger();
            this.executor = Executors.newCachedThreadPool(runnable -> {
                Thread thread = new Thread(runnable, "sagat-client-" + counter.incrementAndGet());
                thread.setDaemon(true);
                return thread;
            });
            this.ownsExecutor = true;
        }
    }

    /** Never shows the token: only where the client talks to. */
    @Override
    public String toString() {
        return "SagatClient[" + config + "]";
    }

    /** Shortcut for {@code new SagatClient(config)}. */
    public static SagatClient create(ClientConfig config) {
        return new SagatClient(config);
    }

    @Override
    public void close() {
        if (ownsExecutor) {
            executor.shutdownNow();
        }
    }

    // ------------------------------------------------------------------------------------------ plumbing

    private String trace(String explicit) {
        String value = explicit;
        if (value == null && config.traceContext() != null) {
            try {
                value = config.traceContext().get();
            } catch (Throwable t) {
                // A tracer that fails is no reason to fail the call: no ambient context, the server makes a trace.
                Hooks.rethrowIfFatal(t);
                Hooks.log(config.logger(), System.Logger.Level.WARNING, "the trace context hook failed (" + t.getClass().getSimpleName() + "): no traceparent is sent");
                value = null;
            }
        }
        return value == null ? null : Traceparents.check(value);
    }

    private static final Pattern RUN_ID = UUID_TEXT;

    /** A run id is a UUID: anything else would be a path of its own ({@code ..}, {@code x/operations}) and is refused before a request. */
    private static String runId(String id) {
        if (id == null || !RUN_ID.matcher(id).matches()) {
            throw SagatException.invalid("a run id is a UUID");
        }
        return id.toLowerCase(Locale.ROOT);
    }

    /** A name in a URL path (a signal) is percent-encoded; empty, {@code .} and {@code ..} would change the path and are refused. */
    private static String pathName(String name, String what) {
        if (name == null || name.isEmpty() || name.equals(".") || name.equals("..")) {
            throw SagatException.invalid(what + " is not empty, \".\" or \"..\"");
        }
        return Endpoints.encodeComponent(name);
    }

    private static String key(String given) {
        String key = given != null ? given : UUID.randomUUID().toString();
        if (!KEY.matcher(key).matches()) {
            throw SagatException.invalid("an idempotency key is 8 to 160 printable ASCII characters");
        }
        return key;
    }

    private <T> CompletableFuture<T> async(Callable<T> body) {
        CompletableFuture<T> future = new CompletableFuture<>();
        Future<?> task = executor.submit(() -> {
            try {
                future.complete(body.call());
            } catch (Throwable t) {
                future.completeExceptionally(t);
            }
        });
        future.whenComplete((result, error) -> {
            if (future.isCancelled()) {
                task.cancel(true);
            }
        });
        return future;
    }

    // ------------------------------------------------------------------------------------------ workflows

    /** The id of a workflow given by id or by name (the active version of the workflow of that name). */
    public String resolveWorkflow(String workflow) {
        if (UUID_TEXT.matcher(workflow).matches()) {
            return workflow.toLowerCase(Locale.ROOT);
        }
        CachedId cached = names.get(workflow);
        if (cached != null && System.nanoTime() - cached.atNanos() < NAME_CACHE_MILLIS * 1_000_000L) {
            return cached.id();
        }
        String cursor = null;
        do {
            Map<String, String> query = new LinkedHashMap<>();
            query.put("limit", "200");
            query.put("cursor", cursor);
            RestClient.Response response = http.request(
                    new RestClient.Request("GET", "/workflows", query, null, null, trace(null), true));
            JsonNode page = object(response.body(), "a page");
            for (JsonNode item : page.path("items")) {
                JsonNode draft = object(item, "a workflow");
                if (draft.path("name").isTextual() && draft.path("name").textValue().equals(workflow)) {
                    String id = str(draft, "id");
                    names.put(workflow, new CachedId(id, System.nanoTime()));
                    return id;
                }
            }
            cursor = optStr(page, "nextCursor");
        } while (cursor != null && !cursor.isEmpty());
        throw SagatException.builder(ErrorCode.NOT_FOUND, "no workflow is named \"" + workflow + "\" in this namespace")
                .httpStatus(404)
                .reason("WORKFLOW_NOT_FOUND")
                .build();
    }

    // ------------------------------------------------------------------------------------------- runs

    /** Starts a run of a published workflow, given by id or by name. {@code input} is any JSON value (an object by convention). */
    public Run start(String workflow, Object input, StartOptions options) {
        StartOptions opts = options == null ? StartOptions.create() : options;
        if (workflow == null || workflow.isEmpty()) {
            throw SagatException.invalid("start needs the name or the id of a workflow");
        }
        // Validated before the name is looked up: a call that cannot be made does not cost a request.
        String idempotencyKey = key(opts.idempotencyKey());
        String traceparent = trace(opts.traceparent());
        String workflowId = resolveWorkflow(workflow);
        Instant deadline = opts.deadline();
        if (opts.deadlineIn() != null) {
            deadline = Instant.now().plus(opts.deadlineIn());
        }
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("workflowId", workflowId);
        body.set("input", input == null ? JsonNodeFactory.instance.objectNode() : Json.toTree(input));
        if (opts.versionId() != null && !opts.versionId().isEmpty()) {
            body.put("versionId", opts.versionId());
        }
        if (opts.correlationId() != null) {
            body.put("correlationId", opts.correlationId());
        }
        if (deadline != null) {
            body.put("deadline", Json.instant(deadline));
        }
        RestClient.Response response = http.request(new RestClient.Request(
                "POST", "/runs", null, body, idempotencyKey, traceparent, true));
        return toRun(response.body());
    }

    /** {@link #start(String, Object, StartOptions)} with the defaults. */
    public Run start(String workflow, Object input) {
        return start(workflow, input, null);
    }

    public CompletableFuture<Run> startAsync(String workflow, Object input, StartOptions options) {
        return async(() -> start(workflow, input, options));
    }

    /** Streams a staged run input once and verifies the server descriptor against the bytes read. */
    public BlobDescriptor uploadRunInput(InputStream source, RunInputBlobOptions options) {
        if (source == null) throw SagatException.invalid("a run-input source is required");
        RunInputBlobOptions opts = options == null ? RunInputBlobOptions.defaults() : options;
        if ((opts.expectedSize() != null && opts.expectedSize() < 0)
                || (opts.expectedSha256() != null && !opts.expectedSha256().matches("[0-9a-f]{64}"))) {
            throw SagatException.invalid("blob preconditions are malformed");
        }
        MeasuringInputStream measured = new MeasuringInputStream(source);
        Map<String, String> headers = new LinkedHashMap<>();
        headers.put("Content-Type", opts.contentType());
        if (opts.expectedSize() != null) headers.put("X-Octopus-Blob-Size", opts.expectedSize().toString());
        if (opts.expectedSha256() != null) headers.put("X-Octopus-Blob-Sha256", opts.expectedSha256());
        RestClient.Response response = http.request(new RestClient.Request(
                "POST", "/run-input-blobs", null, null, null, trace(null), false, measured, headers));
        JsonNode raw = object(response.body(), "a blob descriptor");
        JsonNode size = raw.get("size");
        if (size == null || !size.isIntegralNumber() || !size.canConvertToLong()) {
            throw SagatException.of(ErrorCode.PROTOCOL, "size is missing from the blob descriptor");
        }
        BlobDescriptor descriptor = new BlobDescriptor(str(raw, "id"), size.longValue(),
                str(raw, "sha256"), str(raw, "contentType"));
        if (!measured.eof || !UUID_TEXT.matcher(descriptor.id()).matches() || descriptor.size() != measured.size
                || !descriptor.sha256().equals(measured.sha256()) || descriptor.contentType().isBlank()) {
            throw SagatException.of(ErrorCode.PROTOCOL, "the run-input descriptor does not match the uploaded bytes");
        }
        return descriptor;
    }

    public BlobDescriptor uploadRunInput(InputStream source) {
        return uploadRunInput(source, null);
    }

    public CompletableFuture<BlobDescriptor> uploadRunInputAsync(InputStream source, RunInputBlobOptions options) {
        return async(() -> uploadRunInput(source, options));
    }

    /** Atomically starts a run with a descriptor returned by {@link #uploadRunInput}. */
    public Run startWithBlob(String workflow, BlobDescriptor descriptor, StartOptions options) {
        if (descriptor == null || descriptor.id() == null || descriptor.sha256() == null
                || descriptor.contentType() == null || !UUID_TEXT.matcher(descriptor.id()).matches()
                || descriptor.size() < 0 || !descriptor.sha256().matches("[0-9a-f]{64}")
                || descriptor.contentType().isBlank()) {
            throw SagatException.invalid("a run-input blob descriptor is malformed");
        }
        StartOptions opts = options == null ? StartOptions.create() : options;
        String idempotencyKey = key(opts.idempotencyKey());
        String traceparent = trace(opts.traceparent());
        String workflowId = resolveWorkflow(workflow);
        Instant deadline = opts.deadlineIn() == null ? opts.deadline() : Instant.now().plus(opts.deadlineIn());
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("workflowId", workflowId); body.put("inputBlobRef", descriptor.id());
        if (opts.versionId() != null && !opts.versionId().isEmpty()) body.put("versionId", opts.versionId());
        if (opts.correlationId() != null) body.put("correlationId", opts.correlationId());
        if (deadline != null) body.put("deadline", Json.instant(deadline));
        return toRun(http.request(new RestClient.Request("POST", "/runs", null, body, idempotencyKey, traceparent, true)).body());
    }

    public CompletableFuture<Run> startWithBlobAsync(String workflow, BlobDescriptor descriptor, StartOptions options) {
        return async(() -> startWithBlob(workflow, descriptor, options));
    }

    private static final class MeasuringInputStream extends FilterInputStream {
        private final MessageDigest digest;
        private long size;
        private boolean eof;

        MeasuringInputStream(InputStream source) {
            super(source);
            try { digest = MessageDigest.getInstance("SHA-256"); }
            catch (NoSuchAlgorithmException error) { throw new ExceptionInInitializerError(error); }
        }

        @Override public int read() throws IOException {
            int value = super.read();
            if (value >= 0) { digest.update((byte) value); size++; }
            else { eof = true; }
            return value;
        }

        @Override public int read(byte[] target, int offset, int length) throws IOException {
            int read = super.read(target, offset, length);
            if (read > 0) { digest.update(target, offset, read); size += read; }
            else if (read < 0) { eof = true; }
            return read;
        }

        String sha256() { return java.util.HexFormat.of().formatHex(digest.digest()); }
    }

    /** Reads a run. */
    public Run get(String runId) {
        RestClient.Response response = http.request(new RestClient.Request(
                "GET", "/runs/" + runId(runId), null, null, null, trace(null), true));
        return toRun(response.body());
    }

    public CompletableFuture<Run> getAsync(String runId) {
        return async(() -> get(runId));
    }

    /** One page of runs, newest first. */
    public RunPage list(ListOptions options) {
        ListOptions opts = options == null ? ListOptions.create() : options;
        Map<String, String> query = new LinkedHashMap<>();
        query.put("workflowId", opts.workflowId());
        query.put("status", opts.status() == null ? null : opts.status().name());
        query.put("limit", opts.limit() == null ? null : Integer.toString(opts.limit()));
        query.put("cursor", opts.cursor());
        RestClient.Response response = http.request(new RestClient.Request("GET", "/runs", query, null, null, trace(null), true));
        return toPage(response.body());
    }

    public CompletableFuture<RunPage> listAsync(ListOptions options) {
        return async(() -> list(options));
    }

    /**
     * Every run that matches, page after page, in the order the server gave them (newest first). The stream is lazy:
     * a page is read when the previous one has been consumed. A failure while paging is raised from the stream.
     */
    public Stream<Run> listAll(ListOptions options) {
        ListOptions first = options == null ? ListOptions.create() : options;
        Iterator<Run> iterator = new Iterator<>() {
            private Iterator<Run> page = List.<Run>of().iterator();
            private String cursor = null;
            private boolean started = false;
            private boolean done = false;

            private void fill() {
                while (!page.hasNext() && !done) {
                    ListOptions next = ListOptions.create()
                            .workflowId(first.workflowId())
                            .status(first.status())
                            .limit(first.limit())
                            .cursor(started ? cursor : null);
                    RunPage result = list(next);
                    started = true;
                    cursor = result.nextCursor();
                    page = result.items().iterator();
                    if (cursor == null || cursor.isEmpty()) {
                        done = true;
                    }
                }
            }

            @Override
            public boolean hasNext() {
                fill();
                return page.hasNext();
            }

            @Override
            public Run next() {
                if (!hasNext()) {
                    throw new NoSuchElementException();
                }
                return page.next();
            }
        };
        return StreamSupport.stream(java.util.Spliterators.spliteratorUnknownSize(iterator, java.util.Spliterator.ORDERED), false);
    }

    /** Every run that matches, collected. */
    public CompletableFuture<List<Run>> listAllAsync(ListOptions options) {
        return async(() -> {
            try (Stream<Run> runs = listAll(options)) {
                return runs.toList();
            }
        });
    }

    /** An operation on a run: pause, resume, cancel and the saga decisions. {@code reason} has 8 to 2000 characters. */
    public OperationResult operate(String runId, OperateAction action, String reason, OperateOptions options) {
        OperateOptions opts = options == null ? OperateOptions.create() : options;
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("action", action.name());
        body.put("reason", reason);
        if (opts.stepId() != null) {
            body.put("stepId", opts.stepId());
        }
        if (opts.outcome() != null) {
            body.put("outcome", opts.outcome().name());
        }
        if (opts.errorCode() != null) {
            body.put("errorCode", opts.errorCode());
        }
        if (opts.expectedRunVersion() != null) {
            body.put("expectedRunVersion", opts.expectedRunVersion().longValue());
        }
        RestClient.Response response = http.request(new RestClient.Request(
                "POST",
                "/runs/" + runId(runId) + "/operations",
                null,
                body,
                key(opts.idempotencyKey()),
                trace(opts.traceparent()),
                true));
        JsonNode raw = object(response.body(), "an operation result");
        return new OperationResult(
                str(raw, "operationId"),
                raw.path("accepted").isBoolean() && raw.path("accepted").booleanValue(),
                optStr(raw, "runStatus"),
                raw.path("runVersion").isIntegralNumber() ? Long.valueOf(raw.path("runVersion").longValue()) : null,
                raw);
    }

    public CompletableFuture<OperationResult> operateAsync(String runId, OperateAction action, String reason, OperateOptions options) {
        return async(() -> operate(runId, action, reason, options));
    }

    /** Cancels a run. */
    public OperationResult cancel(String runId, String reason, OperateOptions options) {
        return operate(runId, OperateAction.CANCEL, reason, options);
    }

    public CompletableFuture<OperationResult> cancelAsync(String runId, String reason, OperateOptions options) {
        return async(() -> cancel(runId, reason, options));
    }

    /** Records an eligible actor's durable decision for an approval request. */
    public ApprovalDecisionReceipt decideApproval(String approvalId, ApprovalDecision decision, ApprovalDecisionOptions options) {
        if (approvalId == null || !UUID_TEXT.matcher(approvalId).matches()) {
            throw SagatException.invalid("an approval id is a UUID");
        }
        if (decision == null) {
            throw SagatException.invalid("an approval decision is APPROVE or REJECT");
        }
        if (options == null || options.decisionKey() == null || !APPROVAL_DECISION_KEY.matcher(options.decisionKey()).matches()) {
            throw SagatException.invalid("an approval decision key is 1 to 128 printable ASCII characters");
        }
        if (options.reasonCode() != null && !APPROVAL_REASON_CODE.matcher(options.reasonCode()).matches()) {
            throw SagatException.invalid("an approval reason code is 1 to 64 letters, digits, dots, underscores, colons or hyphens");
        }
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("decisionKey", options.decisionKey());
        body.put("decision", decision.name());
        if (options.reasonCode() != null) {
            body.put("reasonCode", options.reasonCode());
        }
        RestClient.Response response = http.request(new RestClient.Request(
                "POST", "/approvals/" + approvalId.toLowerCase(Locale.ROOT) + "/decisions", null, body, null,
                trace(options.traceparent()), true));
        JsonNode raw = object(response.body(), "an approval decision receipt");
        ApprovalDecision received;
        try {
            received = ApprovalDecision.valueOf(str(raw, "decision"));
        } catch (IllegalArgumentException error) {
            throw SagatException.of(ErrorCode.PROTOCOL, "decision is invalid in the answer");
        }
        Instant decidedAt = optInstant(raw, "decidedAt");
        if (decidedAt == null) {
            throw SagatException.of(ErrorCode.PROTOCOL, "decidedAt is missing from the answer");
        }
        JsonNode sequence = raw.path("outcomeSequence");
        if (!sequence.isIntegralNumber() || sequence.longValue() < 1) {
            throw SagatException.of(ErrorCode.PROTOCOL, "outcomeSequence is invalid in the answer");
        }
        return new ApprovalDecisionReceipt(
                str(raw, "receiptId"), str(raw, "approvalId"), str(raw, "decisionKey"), received,
                optStr(raw, "reasonCode"), str(raw, "actor"), decidedAt, sequence.longValue(),
                raw.path("replayed").isBoolean() && raw.path("replayed").booleanValue(), raw);
    }

    public CompletableFuture<ApprovalDecisionReceipt> decideApprovalAsync(
            String approvalId, ApprovalDecision decision, ApprovalDecisionOptions options) {
        return async(() -> decideApproval(approvalId, decision, options));
    }

    /** Evaluates a declared side-effect-free query over one committed state snapshot. */
    public QueryResult query(String runId, String name, Object args, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.set("args", Json.toTree(args));
        JsonNode raw = object(http.request(new RestClient.Request(
                "POST", declaredOperationPath(runId, "queries", name), null, body, null,
                trace(opts.traceparent()), true)).body(), "a query result");
        if (!raw.has("result")) {
            throw SagatException.of(ErrorCode.PROTOCOL, "result is missing from the answer");
        }
        return new QueryResult(str(raw, "runId"), revision(raw, "runVersion", 0),
                revision(raw, "stateRevision", 0), raw.get("result"), raw);
    }

    public CompletableFuture<QueryResult> queryAsync(String runId, String name, Object args, CallOptions options) {
        return async(() -> query(runId, name, args, options));
    }

    /** Applies or replays a declared durable state update. */
    public UpdateReceipt update(String runId, String name, Object args, UpdateOptions options) {
        if (options == null || options.updateKey() == null || !UPDATE_KEY.matcher(options.updateKey()).matches()) {
            throw SagatException.invalid("an update key is 1 to 128 printable ASCII characters");
        }
        if (options.expectedStateRevision() != null && options.expectedStateRevision() < 0) {
            throw SagatException.invalid("expected state revision is non-negative");
        }
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("updateKey", options.updateKey());
        body.set("args", Json.toTree(args));
        if (options.expectedStateRevision() != null) {
            body.put("expectedStateRevision", options.expectedStateRevision());
        }
        JsonNode raw = object(http.request(new RestClient.Request(
                "POST", declaredOperationPath(runId, "updates", name), null, body, null,
                trace(options.traceparent()), true)).body(), "an update receipt");
        if (!"APPLIED".equals(str(raw, "status"))) {
            throw SagatException.of(ErrorCode.PROTOCOL, "status is invalid in the answer");
        }
        if (!raw.has("result")) {
            throw SagatException.of(ErrorCode.PROTOCOL, "result is missing from the answer");
        }
        return new UpdateReceipt(str(raw, "runId"), str(raw, "operation"), str(raw, "updateKey"), "APPLIED",
                raw.get("result"), revision(raw, "beforeStateRevision", 0), revision(raw, "afterStateRevision", 1),
                revision(raw, "runVersion", 1), raw.path("replayed").isBoolean() && raw.path("replayed").booleanValue(), raw);
    }

    public CompletableFuture<UpdateReceipt> updateAsync(String runId, String name, Object args, UpdateOptions options) {
        return async(() -> update(runId, name, args, options));
    }

    /** Sends a signal to the steps of a run that wait for it; a signal nobody waits for is a {@code CONFLICT}. */
    public SignalResult signal(String runId, String name, Object payload, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        JsonNode body = payload == null ? JsonNodeFactory.instance.objectNode() : Json.toTree(payload);
        RestClient.Response response = http.request(new RestClient.Request(
                "POST",
                "/runs/" + runId(runId) + "/signals/" + pathName(name, "a signal name"),
                null,
                body,
                key(opts.idempotencyKey()),
                trace(opts.traceparent()),
                true));
        JsonNode raw = object(response.body(), "a signal result");
        List<String> delivered = new ArrayList<>();
        for (JsonNode item : raw.path("deliveredTo")) {
            delivered.add(item.asText());
        }
        return new SignalResult(List.copyOf(delivered), raw);
    }

    public CompletableFuture<SignalResult> signalAsync(String runId, String name, Object payload, CallOptions options) {
        return async(() -> signal(runId, name, payload, options));
    }

    /** Reads one committed chain head and lineage snapshot. */
    public ChainInspection inspectChain(String namespaceId, String chainId, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        JsonNode raw = object(http.request(new RestClient.Request(
                "GET", chainPath(namespaceId, chainId), null, null, null, trace(opts.traceparent()), true)).body(),
                "a chain inspection");
        JsonNode values = raw.path("segments");
        if (!values.isArray() || values.isEmpty()) throw SagatException.of(ErrorCode.PROTOCOL, "segments is invalid in the answer");
        List<ChainSegment> segments = new ArrayList<>();
        for (JsonNode value : values) {
            long index = revision(value, "index", 0);
            if (index > 65535) throw SagatException.of(ErrorCode.PROTOCOL, "index is invalid in the answer");
            segments.add(new ChainSegment((int) index, str(value, "runId"), optStr(value, "predecessorRunId"), optStr(value, "successorRunId")));
        }
        return new ChainInspection(str(raw, "chainId"), str(raw, "currentRunId"), revision(raw, "revision", 0),
                raw.path("cancelRequested").asBoolean(false), optStr(raw, "finalOutcome"), List.copyOf(segments), raw);
    }

    public CompletableFuture<ChainInspection> inspectChainAsync(String namespaceId, String chainId, CallOptions options) {
        return async(() -> inspectChain(namespaceId, chainId, options));
    }

    /** Signals a root wait on the expected current chain head. */
    public SignalResult signalChain(String namespaceId, String chainId, String name, Object payload, ChainCommandOptions options) {
        requireChainOptions(options);
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("expectedRunId", runId(options.expectedRunId())); body.set("payload", Json.toTree(payload));
        JsonNode raw = object(http.request(new RestClient.Request(
                "POST", chainPath(namespaceId, chainId) + "/signals/" + pathName(name, "a signal name"), null, body,
                key(options.idempotencyKey()), trace(options.traceparent()), true)).body(), "a chain signal result");
        List<String> delivered = new ArrayList<>();
        for (JsonNode item : raw.path("deliveredTo")) delivered.add(item.asText());
        return new SignalResult(List.copyOf(delivered), raw);
    }

    public CompletableFuture<SignalResult> signalChainAsync(String namespaceId, String chainId, String name, Object payload, ChainCommandOptions options) {
        return async(() -> signalChain(namespaceId, chainId, name, payload, options));
    }

    /** Latches cancellation and cancels the expected current chain head. */
    public void cancelChain(String namespaceId, String chainId, String reason, ChainCommandOptions options) {
        requireChainOptions(options);
        if (reason == null || reason.isEmpty() || reason.codePointCount(0, reason.length()) > 512) throw SagatException.invalid("a chain cancellation reason is 1 to 512 characters");
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("expectedRunId", runId(options.expectedRunId())); body.put("reason", reason);
        http.request(new RestClient.Request("POST", chainPath(namespaceId, chainId) + "/cancel", null, body,
                key(options.idempotencyKey()), trace(options.traceparent()), true));
    }

    public CompletableFuture<Void> cancelChainAsync(String namespaceId, String chainId, String reason, ChainCommandOptions options) {
        return async(() -> { cancelChain(namespaceId, chainId, reason, options); return null; });
    }

    private static void requireChainOptions(ChainCommandOptions options) {
        if (options == null || options.expectedRunId() == null) throw SagatException.invalid("an expected run id is a UUID");
    }

    // ------------------------------------------------------------------------------------------- events

    /** Publishes an event to the inbox of the namespace, once per event id for the deduplication window. */
    public EventAccepted publishEvent(EventInput event, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        ObjectNode body = JsonNodeFactory.instance.objectNode();
        body.put("eventId", event.eventId() != null ? event.eventId() : UUID.randomUUID().toString());
        body.put("type", event.type());
        body.set("data", event.data() == null ? JsonNodeFactory.instance.objectNode() : Json.toTree(event.data()));
        if (event.subject() != null) {
            body.put("subject", event.subject());
        }
        if (event.occurredAt() != null) {
            body.put("occurredAt", Json.instant(event.occurredAt()));
        }
        if (event.correlationKey() != null) {
            body.put("correlationKey", event.correlationKey());
        }
        if (event.dedupeTtlSeconds() != null) {
            body.put("dedupeTtlSeconds", event.dedupeTtlSeconds().intValue());
        }
        // The event id is the key: the server answers a repetition DUPLICATE, so the call is safe to repeat.
        RestClient.Response response = http.request(new RestClient.Request(
                "POST", "/events", null, body, null, trace(opts.traceparent()), true));
        JsonNode raw = object(response.body(), "an event answer");
        String status = str(raw, "status");
        EventAccepted.Status parsed;
        try {
            parsed = EventAccepted.Status.valueOf(status);
        } catch (IllegalArgumentException e) {
            parsed = EventAccepted.Status.UNKNOWN;
        }
        return new EventAccepted(str(raw, "eventId"), str(raw, "source"), parsed, status, raw);
    }

    public CompletableFuture<EventAccepted> publishEventAsync(EventInput event, CallOptions options) {
        return async(() -> publishEvent(event, options));
    }

    /** The journal of a run, a page at a time. */
    public JournalPage events(String runId, String cursor, Integer limit) {
        Map<String, String> query = new LinkedHashMap<>();
        query.put("cursor", cursor);
        query.put("limit", limit == null ? null : Integer.toString(limit));
        RestClient.Response response = http.request(new RestClient.Request(
                "GET", "/runs/" + runId(runId) + "/events", query, null, null, trace(null), true));
        JsonNode raw = object(response.body(), "a page");
        List<JournalEvent> items = new ArrayList<>();
        for (JsonNode item : raw.path("items")) {
            items.add(new JournalEvent(object(item, "an event")));
        }
        return new JournalPage(List.copyOf(items), optStr(raw, "nextCursor"), raw);
    }

    public CompletableFuture<JournalPage> eventsAsync(String runId, String cursor, Integer limit) {
        return async(() -> events(runId, cursor, limit));
    }

    // ----------------------------------------------------------------------------------------- waiting

    /**
     * Polls {@link #get} until the run is in a terminal status or the timeout passes (a {@code TRANSPORT} error
     * with the reason {@code WAIT_TIMEOUT}). An interrupt of the waiting thread is a {@code CANCELLED} error.
     */
    public Run waitForCompletion(String runId, WaitOptions options) {
        WaitOptions opts = options == null ? WaitOptions.create() : options;
        runId(runId);
        long start = System.nanoTime();
        long timeoutNanos = opts.timeout().toNanos();
        long interval = opts.pollInterval().toMillis();
        for (;;) {
            Run run = get(runId);
            if (run.isTerminal()) {
                return run;
            }
            if (System.nanoTime() - start >= timeoutNanos) {
                throw SagatException.builder(ErrorCode.TRANSPORT, "the run did not finish within " + opts.timeout().toMillis() + " ms")
                        .reason("WAIT_TIMEOUT")
                        .build();
            }
            RestClient.pause(Math.round(interval * (0.8 + ThreadLocalRandom.current().nextDouble() * 0.4)));
        }
    }

    public CompletableFuture<Run> waitForCompletionAsync(String runId, WaitOptions options) {
        return async(() -> waitForCompletion(runId, options));
    }

    // ------------------------------------------------------------------------------------------ parsing

    private static JsonNode object(JsonNode value, String what) {
        if (value == null || !value.isObject()) {
            throw SagatException.of(ErrorCode.PROTOCOL, what + " is not an object");
        }
        return value;
    }

    private static String str(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        if (!value.isTextual()) {
            throw SagatException.of(ErrorCode.PROTOCOL, key + " is missing from the answer");
        }
        return value.textValue();
    }

    private static String optStr(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        return value.isTextual() ? value.textValue() : null;
    }

    private static Instant optInstant(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        if (!value.isTextual()) {
            return null;
        }
        try {
            return OffsetDateTime.parse(value.textValue()).toInstant();
        } catch (DateTimeParseException e) {
            return null;
        }
    }

    private static long num(JsonNode raw, String key, long fallback) {
        JsonNode value = raw.path(key);
        return value.isNumber() ? value.longValue() : fallback;
    }

    private static long revision(JsonNode raw, String key, long minimum) {
        JsonNode value = raw.path(key);
        if (!value.isIntegralNumber() || !value.canConvertToLong() || value.longValue() < minimum) {
            throw SagatException.of(ErrorCode.PROTOCOL, key + " is invalid in the answer");
        }
        return value.longValue();
    }

    private static String declaredOperationPath(String runId, String kind, String name) {
        if (name == null || !DECLARED_OPERATION_NAME.matcher(name).matches()) {
            throw SagatException.invalid("an operation name is 1 to 128 permitted characters and starts with a letter or digit");
        }
        return "/runs/" + runId(runId) + "/" + kind + "/" + Endpoints.encodeComponent(name);
    }

    private static String chainPath(String namespaceId, String chainId) {
        if (namespaceId == null || !UUID_TEXT.matcher(namespaceId).matches() || chainId == null || !UUID_TEXT.matcher(chainId).matches()) {
            throw SagatException.invalid("namespace and chain ids are UUIDs");
        }
        return "/namespaces/" + namespaceId.toLowerCase(Locale.ROOT) + "/chains/" + chainId.toLowerCase(Locale.ROOT);
    }

    static Run toRun(JsonNode value) {
        JsonNode raw = object(value, "a run");
        Instant created = optInstant(raw, "createdAt");
        if (created == null) {
            throw SagatException.of(ErrorCode.PROTOCOL, "createdAt is missing from the answer");
        }
        JsonNode parentNode = raw.path("parent");
        Run.Parent parent = parentNode.isObject()
                ? new Run.Parent(parentNode.path("runId").asText(""), parentNode.path("node").asText(""), parentNode.path("cancelPolicy").asText(""))
                : null;
        String status = str(raw, "status");
        return new Run(
                str(raw, "runId"),
                str(raw, "workflowId"),
                str(raw, "versionId"),
                RunStatus.of(status),
                status,
                num(raw, "version", 0),
                raw.path("paused").isBoolean() && raw.path("paused").booleanValue(),
                optStr(raw, "startedBy"),
                created,
                optInstant(raw, "updatedAt"),
                optInstant(raw, "finishedAt"),
                optInstant(raw, "deadline"),
                optStr(raw, "traceId"),
                optStr(raw, "correlationId"),
                (int) num(raw, "depth", 0),
                parent,
                raw);
    }

    private static RunPage toPage(JsonNode body) {
        JsonNode raw = object(body, "a page");
        JsonNode items = raw.path("items");
        if (!items.isArray()) {
            throw SagatException.of(ErrorCode.PROTOCOL, "items is missing from the page");
        }
        List<Run> runs = new ArrayList<>();
        for (JsonNode item : items) {
            runs.add(toRun(item));
        }
        return new RunPage(List.copyOf(runs), optStr(raw, "nextCursor"), raw);
    }
}
