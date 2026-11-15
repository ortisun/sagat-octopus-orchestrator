package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;

/**
 * One stage of a collection's operator chain (design 042): an object with exactly one key, the operator's name, whose
 * value is that operator's parameters. Built with {@link Ops}; the server validates the chain.
 */
public final class Operator {
    private final String name;
    private final ObjectNode parameters;

    Operator(String name, ObjectNode parameters) {
        this.name = name;
        this.parameters = parameters;
    }

    /** The operator's name as the IR writes it: filter, map, flatMap, skip, take, distinct or window. */
    public String name() {
        return name;
    }

    /** The stage as the IR writes it, {@code {name: parameters}}; a fresh copy on every call. */
    ObjectNode toNode() {
        ObjectNode stage = Ops.F.objectNode();
        stage.set(name, parameters.deepCopy());
        return stage;
    }

    /** The canonical text of the stage. */
    public String toJson() {
        return Json.canonical(toNode());
    }

    @Override
    public String toString() {
        return toJson();
    }
}
