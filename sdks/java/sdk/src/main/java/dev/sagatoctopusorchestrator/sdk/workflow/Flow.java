package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;

/**
 * A chain of nodes: the body of a workflow, of a parallel branch or of a saga. Nodes are linked in the order they
 * are added (SPEC section 7): node <em>i</em> gets {@code next = id of node i+1}, except a {@code condition} (it
 * names its two branches) and an {@code end} (it ends); the last node of a flow has no {@code next}.
 */
public final class Flow {
    /** How a parallel node joins its branches. */
    public enum Join {
        /** Waits for every branch. */
        ALL,
        /** Continues with the first branch to finish. */
        ANY;

        String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    private static final Pattern SECRET = Pattern.compile("^[a-z][a-z0-9._-]{0,63}$");
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final List<ObjectNode> nodes = new ArrayList<>();
    private final List<Flow> embedded = new ArrayList<>();

    /** An empty flow, for a parallel branch or a saga body. */
    public static Flow create() {
        return new Flow();
    }

    Flow() {}

    /** Adds the nodes of another flow, in order, after the ones this flow has. */
    void appendAll(Flow other) {
        nodes.addAll(other.nodes);
        embedded.addAll(other.embedded);
    }

    boolean isEmpty() {
        return nodes.isEmpty();
    }

    String firstId() {
        return nodes.get(0).path("id").textValue();
    }

    private Flow push(ObjectNode node) {
        JsonNode id = node.get("id");
        if (id == null || !id.isTextual() || id.textValue().isEmpty()) {
            throw SagatException.invalid("a node needs an id");
        }
        nodes.add(node);
        return this;
    }

    private static String quoted(String text) {
        return "\"" + text + "\"";
    }

    /** An activity executed by a worker. */
    public Flow step(String id, String activity) {
        return step(id, activity, null);
    }

    /** An activity executed by a worker. */
    public Flow step(String id, String activity, StepOptions options) {
        if (activity == null || activity.isEmpty()) {
            throw SagatException.invalid("step " + id + " needs an activity name");
        }
        StepOptions o = options == null ? StepOptions.create() : options;
        if (Boolean.TRUE.equals(o.irreversible) && o.compensateWith != null) {
            throw SagatException.invalid("step " + id + " cannot be irreversible and have a compensation");
        }
        if (o.secrets != null) {
            for (String secret : o.secrets) {
                if (secret == null || !SECRET.matcher(secret).matches()) {
                    throw SagatException.invalid("step " + id + " names a secret that is not a name: " + quoted(String.valueOf(secret)));
                }
            }
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "activity");
        node.put("activity", activity);
        if (o.retry != null) {
            node.set("retry", o.retry.toJson());
        }
        if (o.timeoutMs != null) {
            node.put("timeoutMs", o.timeoutMs.longValue());
        }
        if (o.compensateWith != null) {
            node.put("compensateWith", o.compensateWith);
        }
        if (o.reconcileWith != null) {
            node.put("reconcileWith", o.reconcileWith);
        }
        if (o.idempotencyTemplate != null) {
            node.put("idempotencyTemplate", o.idempotencyTemplate);
        }
        if (o.secrets != null) {
            ArrayNode secrets = node.putArray("secrets");
            o.secrets.forEach(secrets::add);
        }
        if (o.compensationRetry != null) {
            node.set("compensationRetry", o.compensationRetry.toJson());
        }
        if (o.compensationTimeoutMs != null) {
            node.put("compensationTimeoutMs", o.compensationTimeoutMs.longValue());
        }
        if (o.irreversible != null) {
            node.put("irreversible", o.irreversible.booleanValue());
        }
        if (o.highRisk != null) {
            node.put("highRisk", o.highRisk.booleanValue());
        }
        if (o.onError != null) {
            node.put("onError", o.onError);
        }
        return push(node);
    }

    /** Waits for a duration (a whole number of milliseconds, at least 1). A timer, not a process: nothing is kept alive. */
    public Flow sleep(String id, Duration duration) {
        if (duration.toNanos() % 1_000_000L != 0) {
            throw SagatException.invalid("sleep " + id + " needs a whole number of milliseconds");
        }
        return sleepMs(id, duration.toMillis());
    }

    /** Waits for {@code durationMs} milliseconds (at least 1). */
    public Flow sleepMs(String id, long durationMs) {
        if (durationMs < 1) {
            throw SagatException.invalid("sleep " + id + " needs a duration of at least 1 ms");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "timer");
        node.put("durationMs", durationMs);
        return push(node);
    }

    /** Waits for an event of a type with a correlation key, or for the timeout. */
    public Flow waitForEvent(String id, WaitForEventOptions options) {
        if (options.eventType == null || options.eventType.isEmpty() || options.correlationExpr == null || options.correlationExpr.isEmpty()) {
            throw SagatException.invalid("waitForEvent " + id + " needs an eventType and a correlationExpr");
        }
        if (options.timeoutMs < 1) {
            throw SagatException.invalid("waitForEvent " + id + " needs a timeout of at least 1 ms");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "waitEvent");
        node.put("eventType", options.eventType);
        node.put("correlationExpr", options.correlationExpr);
        node.put("timeoutMs", options.timeoutMs);
        if (options.onTimeout != null) {
            node.put("onTimeout", options.onTimeout);
        }
        if (options.lookbackMs != null) {
            node.put("lookbackMs", options.lookbackMs.longValue());
        }
        return push(node);
    }

    /** Starts another workflow of the namespace (by name) and waits for it. */
    public Flow invokeChild(String id, String workflow) {
        return invokeChild(id, workflow, null);
    }

    /** Starts another workflow of the namespace (by name) and waits for it. */
    public Flow invokeChild(String id, String workflow, ChildOptions options) {
        if (workflow == null || workflow.isEmpty()) {
            throw SagatException.invalid("invokeChild " + id + " needs the name of a workflow");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "child");
        node.put("workflowId", workflow);
        if (options != null) {
            if (options.version != null) {
                node.put("version", options.version.intValue());
            }
            if (options.input != null) {
                node.put("input", options.input);
            }
            if (options.cancelPolicy != null) {
                node.put("cancelPolicy", options.cancelPolicy.wire());
            }
        }
        return push(node);
    }

    /** A decision over the run's data: goes to {@code ifTrue} or to {@code ifFalse}. */
    public Flow condition(String id, String expression, String ifTrue, String ifFalse) {
        if (expression == null || expression.isEmpty()) {
            throw SagatException.invalid("condition " + id + " needs an expression");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "condition");
        node.put("expression", expression);
        node.put("ifTrue", ifTrue);
        node.put("ifFalse", ifFalse);
        return push(node);
    }

    /** Runs branches at once; {@code ALL} waits for every one, {@code ANY} for the first. At least two, none empty. */
    public Flow parallel(String id, Join join, List<Flow> branches) {
        if (branches.size() < 2) {
            throw SagatException.invalid("parallel " + id + " needs at least two branches");
        }
        if (branches.stream().anyMatch(Flow::isEmpty)) {
            throw SagatException.invalid("parallel " + id + " has an empty branch");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "parallel");
        ArrayNode first = node.putArray("branches");
        branches.forEach(branch -> first.add(branch.firstId()));
        node.put("join", join.wire());
        push(node);
        embedded.addAll(branches);
        return this;
    }

    /** {@link #parallel(String, Join, List)} with the branches as arguments. */
    public Flow parallel(String id, Join join, Flow... branches) {
        return parallel(id, join, List.of(branches));
    }

    /**
     * The body is compensated, in reverse order of completion, when it fails or the run is cancelled; a step in it
     * names its undo with {@code compensateWith}. The body holds no {@code end}.
     */
    public Flow saga(String id, SagaOptions options, Flow body) {
        if (body.isEmpty()) {
            throw SagatException.invalid("saga " + id + " has an empty body");
        }
        if (body.nodes.stream().anyMatch(node -> "end".equals(node.path("type").textValue()))) {
            throw SagatException.invalid("saga " + id + " cannot hold an end node");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "saga");
        node.put("begin", body.firstId());
        if (options != null) {
            if (options.onCompensationFailure != null) {
                node.put("onCompensationFailure", options.onCompensationFailure.wire());
            }
            if (options.cancelPolicy != null) {
                node.put("cancelPolicy", options.cancelPolicy.wire());
            }
        }
        push(node);
        embedded.add(body);
        return this;
    }

    /** An explicit end of the flow. */
    public Flow end(String id) {
        return end(id, null);
    }

    /** An explicit end of the flow, with the run's output expression. */
    public Flow end(String id, String outputExpr) {
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "end");
        if (outputExpr != null) {
            node.put("outputExpr", outputExpr);
        }
        return push(node);
    }

    /** Every node of the flow and of the flows inside it, in order, with {@code next} set. */
    List<ObjectNode> flatten() {
        List<ObjectNode> out = new ArrayList<>();
        for (int i = 0; i < nodes.size(); i++) {
            ObjectNode linked = nodes.get(i).deepCopy();
            String type = linked.path("type").textValue();
            // A decision names both of its branches and an end ends: neither has a next node.
            boolean terminal = "condition".equals(type) || "end".equals(type);
            if (i + 1 < nodes.size() && !terminal && !linked.has("next")) {
                linked.put("next", nodes.get(i + 1).path("id").textValue());
            }
            out.add(linked);
        }
        for (Flow inner : embedded) {
            out.addAll(inner.flatten());
        }
        return out;
    }
}
