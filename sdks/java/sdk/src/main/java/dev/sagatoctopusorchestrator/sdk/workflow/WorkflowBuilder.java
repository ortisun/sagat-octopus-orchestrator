package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * Builds the workflow IR ({@code workflow.schema.json}) without any side effect (SPEC section 7). {@link #build()}
 * returns the document and {@link #toJson()} its canonical text (UTF-8, keys sorted, no insignificant whitespace),
 * which is byte for byte the same in the five SDKs for the same program.
 *
 * <p>Mistakes the builder can see (a duplicate id, an empty flow, a bad name, a reference to a node that does not
 * exist) are {@code VALIDATION_FAILED} errors raised when the node is added or when {@link #build()} runs, not when the
 * server refuses the document.
 */
public final class WorkflowBuilder {
    private static final Pattern NAME = Pattern.compile("^[a-z][a-z0-9-]{2,63}$");
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final String name;
    private final Flow body = new Flow();
    private final Map<String, String> labels = new LinkedHashMap<>();
    private final List<Trigger> triggers = new ArrayList<>();
    private String description;
    private JsonNode inputSchema;
    private JsonNode outputSchema;

    private WorkflowBuilder(String name) {
        if (name == null || !NAME.matcher(name).matches()) {
            throw SagatException.invalid("a workflow name is lower case letters, digits and dashes, 3 to 64: \"" + name + "\"");
        }
        this.name = name;
    }

    /** Starts a workflow definition. */
    public static WorkflowBuilder named(String name) {
        return new WorkflowBuilder(name);
    }

    public WorkflowBuilder description(String text) {
        this.description = text;
        return this;
    }

    public WorkflowBuilder label(String key, String value) {
        labels.put(key, value);
        return this;
    }

    public WorkflowBuilder trigger(Trigger trigger) {
        triggers.add(trigger);
        return this;
    }

    /** A JSON Schema for the run's input (a map or a {@code JsonNode}). */
    public WorkflowBuilder inputSchema(Object schema) {
        this.inputSchema = objectOrThrow(schema, "inputSchema");
        return this;
    }

    public WorkflowBuilder outputSchema(Object schema) {
        this.outputSchema = objectOrThrow(schema, "outputSchema");
        return this;
    }

    private static JsonNode objectOrThrow(Object schema, String what) {
        JsonNode node = Json.toTree(schema);
        if (!node.isObject()) {
            throw SagatException.invalid(what + " is a JSON object");
        }
        return node;
    }

    /** Adds the nodes of a flow, in order, after the ones the workflow has; the flow is not changed. */
    public WorkflowBuilder append(Flow flow) {
        body.appendAll(flow);
        return this;
    }

    // The flow methods, on the top flow of the workflow.

    public WorkflowBuilder step(String id, String activity) {
        body.step(id, activity);
        return this;
    }

    public WorkflowBuilder step(String id, String activity, StepOptions options) {
        body.step(id, activity, options);
        return this;
    }

    public WorkflowBuilder sleep(String id, Duration duration) {
        body.sleep(id, duration);
        return this;
    }

    public WorkflowBuilder sleepMs(String id, long durationMs) {
        body.sleepMs(id, durationMs);
        return this;
    }

    public WorkflowBuilder waitForEvent(String id, WaitForEventOptions options) {
        body.waitForEvent(id, options);
        return this;
    }

    public WorkflowBuilder invokeChild(String id, String workflow) {
        body.invokeChild(id, workflow);
        return this;
    }

    public WorkflowBuilder invokeChild(String id, String workflow, ChildOptions options) {
        body.invokeChild(id, workflow, options);
        return this;
    }

    public WorkflowBuilder condition(String id, String expression, String ifTrue, String ifFalse) {
        body.condition(id, expression, ifTrue, ifFalse);
        return this;
    }

    public WorkflowBuilder parallel(String id, Flow.Join join, List<Flow> branches) {
        body.parallel(id, join, branches);
        return this;
    }

    public WorkflowBuilder parallel(String id, Flow.Join join, Flow... branches) {
        body.parallel(id, join, branches);
        return this;
    }

    public WorkflowBuilder saga(String id, SagaOptions options, Flow sagaBody) {
        body.saga(id, options, sagaBody);
        return this;
    }

    public WorkflowBuilder end(String id) {
        body.end(id);
        return this;
    }

    public WorkflowBuilder end(String id, String outputExpr) {
        body.end(id, outputExpr);
        return this;
    }

    /** The document. Raises {@code VALIDATION_FAILED} for what the server would refuse and the builder can see. */
    public ObjectNode build() {
        if (body.isEmpty()) {
            throw SagatException.invalid("a workflow needs at least one node");
        }
        List<ObjectNode> nodes = body.flatten();
        Set<String> ids = new HashSet<>();
        for (ObjectNode node : nodes) {
            String id = node.path("id").textValue();
            if (!ids.add(id)) {
                throw SagatException.invalid("the node id \"" + id + "\" is used twice");
            }
        }
        for (ObjectNode node : nodes) {
            String id = node.path("id").textValue();
            for (String field : new String[] {"next", "onError", "onTimeout", "ifTrue", "ifFalse", "begin"}) {
                reference(ids, id, field, node.get(field));
            }
            JsonNode branches = node.get("branches");
            if (branches != null && branches.isArray()) {
                branches.forEach(branch -> reference(ids, id, "branches", branch));
            }
        }
        ObjectNode metadata = F.objectNode();
        metadata.put("name", name);
        if (description != null) {
            metadata.put("description", description);
        }
        if (!labels.isEmpty()) {
            ObjectNode labelNode = metadata.putObject("labels");
            labels.forEach(labelNode::put);
        }
        ObjectNode document = F.objectNode();
        document.put("apiVersion", "sagat-octopus-orchestrator.io/v1");
        document.put("kind", "Workflow");
        document.set("metadata", metadata);
        document.put("entrypoint", body.firstId());
        ArrayNode nodeArray = document.putArray("nodes");
        nodes.forEach(nodeArray::add);
        if (!triggers.isEmpty()) {
            ArrayNode triggerArray = document.putArray("triggers");
            triggers.forEach(trigger -> triggerArray.add(trigger.toJson()));
        }
        if (inputSchema != null) {
            document.set("inputSchema", inputSchema.deepCopy());
        }
        if (outputSchema != null) {
            document.set("outputSchema", outputSchema.deepCopy());
        }
        return document;
    }

    private static void reference(Set<String> ids, String from, String field, JsonNode target) {
        if (target != null && target.isTextual() && !ids.contains(target.textValue())) {
            throw SagatException.invalid(from + "." + field + " points at \"" + target.textValue() + "\", which is not a node");
        }
    }

    /** The canonical text of {@link #build()}: keys sorted, no whitespace. */
    public String toJson() {
        return Json.canonical(build());
    }
}
