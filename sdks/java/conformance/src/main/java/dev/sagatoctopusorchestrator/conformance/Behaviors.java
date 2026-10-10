package dev.sagatoctopusorchestrator.conformance;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityContext;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityFailure;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityHandler;
import dev.sagatoctopusorchestrator.sdk.worker.Verdict;
import java.time.Duration;
import java.time.Instant;
import java.util.List;
import java.util.Map;

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
    static ActivityHandler<JsonNode, Object> handler(String name, String kind, JsonNode behavior, List<ObjectNode> log) {
        return (input, ctx) -> {
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
                    out.put("secretLength", ctx.secrets().get(behavior.get("readsSecret").asText()).length);
                    return out;
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
                log.add(entry(name, kind, input, ctx, outcome));
            }
        };
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
