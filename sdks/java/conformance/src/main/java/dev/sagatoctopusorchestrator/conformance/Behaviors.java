package dev.sagatoctopusorchestrator.conformance;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityContext;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityFailure;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityHandler;
import dev.sagatoctopusorchestrator.sdk.worker.BlobDescriptor;
import dev.sagatoctopusorchestrator.sdk.worker.BlobOutput;
import dev.sagatoctopusorchestrator.sdk.worker.BlobWriteOptions;
import dev.sagatoctopusorchestrator.sdk.worker.Secrets;
import dev.sagatoctopusorchestrator.sdk.worker.Verdict;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.security.MessageDigest;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/** The activity behaviours of a scenario (tests/contract/sdk-conformance/README.md, "Worker activities"). */
final class Behaviors {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private Behaviors() {}

    private static ActivityFailure failure(JsonNode fails) {
        return new ActivityFailure(fails.path("code").asText(), fails.path("message").asText(), fails.path("retryable").asBoolean(true));
    }

    private static JsonNode orNull(JsonNode node) {
        return node == null ? F.nullNode() : node;
    }

    /** The handler of one activity; every attempt appends an entry to {@code log} when the handler ends. */
    static ActivityHandler<JsonNode, Object> handler(
            String name, String kind, JsonNode behavior, List<ObjectNode> log, Measurements measurements) {
        return (input, ctx) -> {
            measurements.enter(ctx);
            String outcome = "returned";
            try {
                if (behavior.has("failsUntilAttempt") && ctx.attempt() < behavior.get("failsUntilAttempt").asInt()) {
                    outcome = "failed";
                    throw failure(behavior.get("fails"));
                }
                if (behavior.has("fails") && !behavior.has("failsUntilAttempt")) {
                    outcome = "failed";
                    throw failure(behavior.get("fails"));
                }
                if (behavior.has("throws")) {
                    outcome = "threw";
                    throw new IllegalStateException(behavior.get("throws").asText());
                }
                if (behavior.has("outcomeUnknown")) {
                    outcome = "failed";
                    throw ActivityFailure.outcomeUnknown(behavior.get("outcomeUnknown").asText());
                }
                if (behavior.path("hang").asBoolean(false)) {
                    ctx.cancellation().awaitCancelled();
                    outcome = "cancelled";
                    // onCancel.raises: the handler ends by raising the SDK's cancellation error.
                    if (behavior.path("onCancel").path("raises").asBoolean(false)) {
                        ctx.cancellation().throwIfCancelled();
                    }
                    return F.nullNode();
                }
                if (behavior.path("ignoresCancel").asBoolean(false)) {
                    // A handler that never looks at its token: it runs to its end and what it returns is reported.
                    Thread.sleep(behavior.path("sleepMs").asLong(0));
                    return orNull(behavior.get("returns"));
                }
                if (behavior.has("sleepMs")) {
                    boolean cancelled = ctx.cancellation().await(Duration.ofMillis(behavior.get("sleepMs").asLong()));
                    if (cancelled) {
                        outcome = "cancelled";
                        if (behavior.path("onCancel").path("raises").asBoolean(false)) {
                            ctx.cancellation().throwIfCancelled();
                        }
                        return orNull(behavior.path("onCancel").get("returns"));
                    }
                }
                if (behavior.has("readsSecret")) {
                    ObjectNode out = F.objectNode();
                    if (behavior.has("returns") && behavior.get("returns").isObject()) {
                        out.setAll((ObjectNode) behavior.get("returns").deepCopy());
                    }
                    final Secrets secrets;
                    try {
                        secrets = ctx.secrets();
                    } catch (SagatException refusal) {
                        // `reportsSecretError`: the canonical code of the refusal is the output, so the scenario can compare it.
                        if (behavior.path("reportsSecretError").asBoolean(false)) {
                            out.put("secretError", refusal.code().name());
                            return out;
                        }
                        throw refusal;
                    }
                    out.put("secretLength", secrets.get(behavior.get("readsSecret").asText()).length);
                    return out;
                }
                if (behavior.has("blobRoundTripBytes")) {
                    byte[] body = new byte[behavior.get("blobRoundTripBytes").asInt()];
                    Arrays.fill(body, (byte) 'x');
                    String sha256 = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(body));
                    var descriptor = ctx.writeBlob(new ByteArrayInputStream(body),
                            new BlobWriteOptions("application/octet-stream", body.length, sha256));
                    ByteArrayOutputStream read = new ByteArrayOutputStream();
                    try (var stream = ctx.readBlob(descriptor)) {
                        stream.transferTo(read);
                    }
                    if (!Arrays.equals(read.toByteArray(), body)) {
                        throw new IllegalStateException("blob round trip failed");
                    }
                    ObjectNode result = F.objectNode();
                    result.put("blobSize", descriptor.size());
                    result.put("blobSha256", descriptor.sha256());
                    result.put("roundTrip", true);
                    return result;
                }
                if (behavior.has("blobInputBytes")) {
                    JsonNode raw = input.get("$blob");
                    var descriptor = new BlobDescriptor(raw.get("id").asText(), raw.get("size").asLong(),
                            raw.get("sha256").asText(), raw.get("contentType").asText());
                    ByteArrayOutputStream read = new ByteArrayOutputStream();
                    try (var stream = ctx.readBlob(descriptor)) { stream.transferTo(read); }
                    byte[] expected = new byte[behavior.get("blobInputBytes").asInt()];
                    Arrays.fill(expected, (byte) 'i');
                    if (!Arrays.equals(read.toByteArray(), expected)) throw new IllegalStateException("run-input blob did not match");
                    ObjectNode result = F.objectNode();
                    result.put("inputBlobSize", descriptor.size()); result.put("inputBlobSha256", descriptor.sha256());
                    return result;
                }
                if (behavior.has("blobOutputBytes")) {
                    byte[] body = new byte[behavior.get("blobOutputBytes").asInt()];
                    Arrays.fill(body, (byte) 'y');
                    var descriptor = ctx.writeBlob(new ByteArrayInputStream(body),
                            new BlobWriteOptions("application/octet-stream", 0, ""));
                    return new BlobOutput(descriptor);
                }
                if (behavior.has("blobInlineBytes")) {
                    byte[] body = new byte[behavior.get("blobInlineBytes").asInt()];
                    Arrays.fill(body, (byte) 'z');
                    var descriptor = ctx.writeBlob(new ByteArrayInputStream(body),
                            new BlobWriteOptions("application/octet-stream", 0, ""));
                    ObjectNode blob = F.objectNode();
                    blob.put("id", descriptor.id());
                    blob.put("size", descriptor.size());
                    blob.put("sha256", descriptor.sha256());
                    blob.put("contentType", descriptor.contentType());
                    ObjectNode result = F.objectNode();
                    result.set("$blob", blob);
                    return result;
                }
                if (behavior.has("verdict")) {
                    JsonNode output = behavior.get("output");
                    return switch (behavior.get("verdict").asText()) {
                        case "applied" -> output == null ? Verdict.applied() : Verdict.applied(output);
                        case "not_applied" -> Verdict.notApplied();
                        default -> Verdict.unknown();
                    };
                }
                if (behavior.path("echo").asBoolean(false)) {
                    ObjectNode out = F.objectNode();
                    out.set("input", input);
                    out.set("steps", stepsOf(ctx));
                    return out;
                }
                if (behavior.path("echoInput").asBoolean(false)) {
                    return input;
                }
                if (behavior.has("returnsText")) {
                    return "x".repeat(behavior.get("returnsText").asInt());
                }
                return orNull(behavior.get("returns"));
            } finally {
                try {
                    log.add(entry(name, kind, input, ctx, outcome));
                } finally {
                    measurements.leave();
                }
            }
        };
    }

    /** Benchmark-only handler admission evidence; excluded from conformance golden normalization. */
    static final class Measurements {
        private final AtomicInteger active = new AtomicInteger();
        private final AtomicInteger peak = new AtomicInteger();
        private final List<ObjectNode> deliveries = new ArrayList<>();

        void enter(ActivityContext context) {
            int now = active.incrementAndGet();
            peak.accumulateAndGet(now, Math::max);
            ObjectNode delivery = F.objectNode();
            delivery.put("runId", context.runId());
            delivery.put("node", context.node());
            delivery.put("takenAtUnixMs", System.currentTimeMillis());
            synchronized (deliveries) {
                deliveries.add(delivery);
            }
        }

        void leave() {
            active.decrementAndGet();
        }

        ObjectNode snapshot() {
            ObjectNode out = F.objectNode();
            out.put("peakInflight", peak.get());
            var values = out.putArray("deliveries");
            synchronized (deliveries) {
                deliveries.forEach(values::add);
            }
            return out;
        }
    }

    private static ObjectNode stepsOf(ActivityContext ctx) {
        ObjectNode steps = F.objectNode();
        for (Map.Entry<String, JsonNode> e : ctx.steps().entrySet()) {
            steps.set(e.getKey(), e.getValue());
        }
        return steps;
    }

    private static ObjectNode entry(String name, String kind, JsonNode input, ActivityContext ctx, String outcome) {
        ObjectNode e = F.objectNode();
        e.put("activity", name);
        e.put("kind", kind);
        e.put("node", ctx.node());
        e.put("attempt", ctx.attempt());
        e.set("input", input);
        e.put("taskKind", ctx.taskKind().name());
        e.put("targetNode", ctx.targetNode());
        e.put("targetIdempotencyKey", ctx.targetIdempotencyKey());
        e.put("idempotencyKey", ctx.idempotencyKey());
        e.put("traceparentPresent", !ctx.traceparent().isEmpty());
        String[] parts = ctx.traceparent().split("-");
        e.put("traceId", parts.length > 1 ? parts[1] : "");
        e.set("steps", stepsOf(ctx));
        e.put("deadlineAhead", ctx.deadline().isAfter(Instant.now()));
        if (ctx.cancellation().isCancelled()) {
            e.put("abortedReason", ctx.cancellation().reason().orElseThrow().wire());
        } else {
            e.putNull("abortedReason");
        }
        e.put("outcome", outcome);
        return e;
    }
}
