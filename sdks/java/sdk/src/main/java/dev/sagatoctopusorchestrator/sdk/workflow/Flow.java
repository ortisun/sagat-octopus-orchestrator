package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * A chain of nodes: the body of a workflow, of a parallel branch or of a saga. Nodes are linked in the order they
 * are added: node <em>i</em> gets {@code next = id of node i+1}, except a branching node and an
 * {@code end}; the last node of a flow has no {@code next}.
 *
 * <p>A flow joined into another ({@link #parallel(String, Join, List)}, {@link #saga(String, SagaOptions, Flow)}) is joined
 * <em>by reference</em> and read when {@code build()} is called: a step added to it afterwards is part of the document.
 * Node ids are unique across the whole document, nested flows included; each is checked when the node is added, against
 * every flow the node's flow is joined into, and again by {@code build()}.
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
    /**
     * The flows this one is joined into (a flow is read by reference, SPEC section 7, so it can be joined to more than
     * one workflow); it is how a node added later is checked against the whole document it belongs to.
     */
    private final List<Flow> parents = new ArrayList<>();
    /** The ids of this flow's nodes and, below it, of every flow joined into it. */
    private final Set<String> ids = new HashSet<>();

    /** An empty flow, for a parallel branch or a saga body. */
    public static Flow create() {
        return new Flow();
    }

    /** A closed body for a runtime-sized collection or loop. */
    public static Flow body() {
        return new Flow();
    }

    Flow() {}

    private static SagatException duplicate(String id) {
        return SagatException.invalid("the node id \"" + id + "\" is used twice");
    }

    /** This flow and every flow it is joined into, directly or not, each once. */
    private Set<Flow> lineage() {
        Set<Flow> seen = java.util.Collections.newSetFromMap(new java.util.IdentityHashMap<>());
        java.util.ArrayDeque<Flow> queue = new java.util.ArrayDeque<>();
        queue.add(this);
        while (!queue.isEmpty()) {
            Flow flow = queue.poll();
            if (seen.add(flow)) {
                queue.addAll(flow.parents);
            }
        }
        return seen;
    }

    /**
     * Raises when one of these ids is already in the document this flow belongs to (this flow, every flow it is joined
     * into, and so everything below them): an id is unique across the whole document, nested flows included.
     */
    private void requireFree(Collection<String> incoming) {
        for (Flow flow : lineage()) {
            for (String id : incoming) {
                if (flow.ids.contains(id)) {
                    throw duplicate(id);
                }
            }
        }
    }

    private void take(Collection<String> incoming) {
        for (Flow flow : lineage()) {
            flow.ids.addAll(incoming);
        }
    }

    /** The ids that joining these flows (and a node) would add, each once; raises on the first that two of them share. */
    private static Set<String> incoming(String nodeId, List<Flow> joining) {
        Set<String> incoming = new HashSet<>();
        if (nodeId != null && !incoming.add(nodeId)) {
            throw duplicate(nodeId);
        }
        for (Flow flow : joining) {
            for (String id : flow.ids) {
                if (!incoming.add(id)) {
                    throw duplicate(id);
                }
            }
        }
        return incoming;
    }

    /** A copy of the flow as it is now, for {@link WorkflowBuilder#append(Flow)}: the nodes are not shared. */
    private Flow copy() {
        Flow flow = new Flow();
        for (ObjectNode node : nodes) {
            flow.nodes.add(node.deepCopy());
        }
        flow.ids.addAll(ids);
        for (Flow inner : embedded) {
            Flow innerCopy = inner.copy();
            innerCopy.parents.add(flow);
            flow.embedded.add(innerCopy);
        }
        return flow;
    }

    /** Adds the nodes of another flow (a copy of it as it is now), in order, after the ones this flow has. */
    void appendAll(Flow other) {
        Flow copy = other.copy();
        requireFree(copy.ids);
        nodes.addAll(copy.nodes);
        for (Flow inner : copy.embedded) {
            inner.parents.remove(copy);
            inner.parents.add(this);
            embedded.add(inner);
        }
        take(copy.ids);
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
        // Raised when the node is added (SPEC section 7), against every node of the document this flow belongs to.
        requireFree(List.of(id.textValue()));
        nodes.add(node);
        take(List.of(id.textValue()));
        return this;
    }

    private static void requireId(ObjectNode node) {
        JsonNode id = node.get("id");
        if (id == null || !id.isTextual() || id.textValue().isEmpty()) {
            throw SagatException.invalid("a node needs an id");
        }
    }

    /** A flow cannot be joined into itself or into something below it (that would be a loop, not a document). */
    private void joinable(List<Flow> joining) {
        Set<Flow> mine = lineage();
        for (Flow flow : joining) {
            if (mine.contains(flow)) {
                throw SagatException.invalid("a flow cannot be joined into itself");
            }
        }
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

    /** Tries cases in order and follows the first matching target, or the default. It has no next node. */
    public Flow switchCase(String id, List<SwitchCase> cases, String defaultTarget) {
        if (cases == null || cases.isEmpty() || cases.size() > 32) {
            throw SagatException.invalid("switch " + id + " needs between 1 and 32 cases");
        }
        if (defaultTarget == null || defaultTarget.isEmpty()) {
            throw SagatException.invalid("switch " + id + " needs a default target");
        }
        Set<String> caseIds = new HashSet<>();
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "switch");
        ArrayNode copied = node.putArray("cases");
        for (SwitchCase item : cases) {
            if (item == null || item.id() == null || item.id().isEmpty()) {
                throw SagatException.invalid("switch " + id + " has a case without an id");
            }
            if (!caseIds.add(item.id())) {
                throw SagatException.invalid("switch " + id + " uses case id " + quoted(item.id()) + " twice");
            }
            if (item.when() == null || item.when().isEmpty()) {
                throw SagatException.invalid("switch " + id + " case " + quoted(item.id()) + " needs a when expression");
            }
            if (item.target() == null || item.target().isEmpty()) {
                throw SagatException.invalid("switch " + id + " case " + quoted(item.id()) + " needs a target");
            }
            ObjectNode copiedCase = copied.addObject();
            copiedCase.put("id", item.id());
            copiedCase.put("when", item.when());
            copiedCase.put("target", item.target());
        }
        node.put("default", defaultTarget);
        return push(node);
    }

    /** Invokes a named body once for every item captured from {@code items}. */
    public Flow forEach(String id, String items, String body, String result, CollectionOptions options) {
        if (items == null || items.isEmpty() || body == null || body.isEmpty() || result == null || result.isEmpty()) {
            throw SagatException.invalid("forEach " + id + " needs items, body and result expressions");
        }
        if (options == null || options.maxItems() < 1 || options.maxItems() > 10_000) {
            throw SagatException.invalid("forEach " + id + " maxItems must be between 1 and 10000");
        }
        if (options.maxConcurrency() < 1 || options.maxConcurrency() > 256) {
            throw SagatException.invalid("forEach " + id + " maxConcurrency must be between 1 and 256");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "forEach");
        node.put("items", items);
        node.put("body", body);
        node.put("result", result);
        node.put("maxItems", options.maxItems());
        node.put("maxConcurrency", options.maxConcurrency());
        return push(node);
    }

    /** Value-returning collection authoring; emits the same durable {@code forEach} node. */
    public Flow map(String id, String items, String body, String result, CollectionOptions options) {
        return forEach(id, items, body, result, options);
    }

    /** Runs a named body sequentially while a pre-test condition is true. */
    public Flow whileLoop(String id, String condition, String body, String initial, String carry, LoopOptions options) {
        return loop("while", id, condition, body, initial, carry, options);
    }

    /** Runs a named body once, then repeats it until a post-test condition is true. */
    public Flow repeatUntil(String id, String condition, String body, String initial, String carry, LoopOptions options) {
        return loop("repeatUntil", id, condition, body, initial, carry, options);
    }

    private Flow loop(String kind, String id, String condition, String body, String initial, String carry, LoopOptions options) {
        if (condition == null || condition.isEmpty() || body == null || body.isEmpty()
                || initial == null || initial.isEmpty() || carry == null || carry.isEmpty()) {
            throw SagatException.invalid(kind + " " + id + " needs condition, body, initial and carry expressions");
        }
        if (options == null || options.maxIterations() < 1 || options.maxIterations() > 10_000) {
            throw SagatException.invalid(kind + " " + id + " maxIterations must be between 1 and 10000");
        }
        long timeoutMs = loopMillis(options.timeout(), kind + " " + id + " timeoutMs");
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", kind);
        node.put("condition", condition);
        node.put("body", body);
        node.put("initial", initial);
        node.put("carry", carry);
        node.put("maxIterations", options.maxIterations());
        node.put("timeoutMs", timeoutMs);
        if (options.delay() != null) {
            node.put("delayMs", loopMillis(options.delay(), kind + " " + id + " delayMs"));
        }
        if (options.onError() != null) {
            node.put("onError", options.onError());
        }
        return push(node);
    }

    /** Waits for one authorized approval decision or for its deadline. */
    public Flow approval(String id, ApprovalOptions options) {
        if (options == null || options.policy() == null) {
            throw SagatException.invalid("approval " + id + " policy needs between 1 and 32 subjects and roles");
        }
        List<String> subjects = options.policy().subjects() == null ? List.of() : options.policy().subjects();
        List<ApprovalRole> roles = options.policy().roles() == null ? List.of() : options.policy().roles();
        int principals = subjects.size() + roles.size();
        if (principals < 1 || principals > 32) {
            throw SagatException.invalid("approval " + id + " policy needs between 1 and 32 subjects and roles");
        }
        if (new HashSet<>(subjects).size() != subjects.size()) {
            throw SagatException.invalid("approval " + id + " policy has a duplicate subject");
        }
        if (new HashSet<>(roles).size() != roles.size()) {
            throw SagatException.invalid("approval " + id + " policy has a duplicate role");
        }
        for (String subject : subjects) {
            if (subject == null || subject.isBlank()
                    || subject.getBytes(StandardCharsets.UTF_8).length > 255
                    || subject.chars().anyMatch(Character::isISOControl)
                    || subject.regionMatches(true, 0, "system:", 0, 7)) {
                throw SagatException.invalid("approval " + id + " policy has an invalid subject");
            }
        }
        if (roles.stream().anyMatch(role -> role == null)) {
            throw SagatException.invalid("approval " + id + " policy has an unsupported role");
        }
        long timeoutMs = loopMillis(options.timeout(), "approval " + id + " timeoutMs");
        if (options.onApprove() == null || options.onApprove().isEmpty()) {
            throw SagatException.invalid("approval " + id + " needs onApprove");
        }
        if (options.onReject() == null || options.onReject().isEmpty()) {
            throw SagatException.invalid("approval " + id + " needs onReject");
        }
        if (options.onExpire() == null || options.onExpire().isEmpty()) {
            throw SagatException.invalid("approval " + id + " needs onExpire");
        }
        ObjectNode policy = F.objectNode();
        if (options.policy().subjects() != null) {
            ArrayNode values = policy.putArray("subjects");
            subjects.forEach(values::add);
        }
        if (options.policy().roles() != null) {
            ArrayNode values = policy.putArray("roles");
            roles.forEach(role -> values.add(switch (role) {
                case ADMIN -> "admin";
                case DEVELOPER -> "developer";
                case OPERATOR -> "operator";
            }));
        }
        if (options.policy().excludeInitiator() != null) {
            policy.put("excludeInitiator", options.policy().excludeInitiator());
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "approval");
        node.set("policy", policy);
        node.put("timeoutMs", timeoutMs);
        node.put("onApprove", options.onApprove());
        node.put("onReject", options.onReject());
        node.put("onExpire", options.onExpire());
        return push(node);
    }

    private static long loopMillis(Duration duration, String subject) {
        if (duration == null || duration.isNegative() || duration.isZero()
                || duration.getNano() % 1_000_000 != 0
                || duration.compareTo(Duration.ofMillis(31_622_400_000L)) > 0) {
            throw SagatException.invalid(subject + " must be between 1 and 31622400000");
        }
        return duration.toMillis();
    }

    /** Runs branches at once; {@code ALL} waits for every one, {@code ANY} for the first. At least two, none empty. */
    public Flow parallel(String id, Join join, List<Flow> branches) {
        if (branches.size() < 2) {
            throw SagatException.invalid("parallel " + id + " needs at least two branches");
        }
        if (branches.stream().anyMatch(Flow::isEmpty)) {
            throw SagatException.invalid("parallel " + id + " has an empty branch");
        }
        // The branches are joined by reference: they are read when build() is called, so what is added to them later is
        // part of the document. Their ids, and the node's own, are checked here.
        Set<String> incoming = incoming(id, branches);
        joinable(branches);
        requireFree(incoming);
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "parallel");
        ArrayNode first = node.putArray("branches");
        branches.forEach(branch -> first.add(branch.firstId()));
        node.put("join", join.wire());
        requireId(node);
        nodes.add(node);
        for (Flow branch : branches) {
            branch.parents.add(this);
            embedded.add(branch);
        }
        take(incoming);
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
        Set<String> incoming = incoming(id, List.of(body));
        joinable(List.of(body));
        requireFree(incoming);
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
        requireId(node);
        nodes.add(node);
        body.parents.add(this);
        embedded.add(body);
        take(incoming);
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

    /** Returns a value from a named body; it is not a workflow terminal. */
    public Flow returnValue(String id, String result) {
        if (result == null || result.isEmpty()) {
            throw SagatException.invalid("return " + id + " needs a result expression");
        }
        ObjectNode node = F.objectNode();
        node.put("id", id);
        node.put("type", "return");
        node.put("result", result);
        return push(node);
    }

    /** Every node of the flow and of the flows inside it, in order, with {@code next} set. */
    List<ObjectNode> flatten() {
        List<ObjectNode> out = new ArrayList<>();
        for (int i = 0; i < nodes.size(); i++) {
            ObjectNode linked = nodes.get(i).deepCopy();
            String type = linked.path("type").textValue();
            // A branching node names its routes and an end ends: neither has a next node.
            boolean terminal = "condition".equals(type) || "switch".equals(type) || "approval".equals(type)
                    || "end".equals(type) || "return".equals(type);
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
