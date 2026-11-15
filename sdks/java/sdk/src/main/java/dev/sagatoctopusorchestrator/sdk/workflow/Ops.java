package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * The operator factory of a collection (design 042, FR-042-028). Each method returns one stage; gather them with
 * {@link Operators#of(Operator...)}. The factory only assembles a stage; the server validates the chain (its length, the
 * ranges of the counts and the variables an expression reads). Stage expressions read {@code element}, {@code position},
 * {@code input}, {@code steps}, {@code run} and {@code state}.
 */
public final class Ops {
    static final JsonNodeFactory F = JsonNodeFactory.instance;

    private Ops() {
    }

    private static Operator stage(String name, ObjectNode parameters) {
        return new Operator(name, parameters);
    }

    /** Keeps the elements for which {@code where} is true. */
    public static Operator filter(String where) {
        return stage("filter", F.objectNode().put("where", where));
    }

    /** Replaces each element by the value of {@code select}. */
    public static Operator map(String select) {
        return stage("map", F.objectNode().put("select", select));
    }

    /** {@code select} yields a list for each element; the lists are concatenated in order, one level deep. */
    public static Operator flatMap(String select) {
        return stage("flatMap", F.objectNode().put("select", select));
    }

    /** Drops the first {@code count} elements. */
    public static Operator skip(int count) {
        return stage("skip", F.objectNode().put("count", count));
    }

    /** Keeps the first {@code count} elements. */
    public static Operator take(int count) {
        return stage("take", F.objectNode().put("count", count));
    }

    /** Keeps the first occurrence of each element. */
    public static Operator distinct() {
        return stage("distinct", F.objectNode());
    }

    /** Keeps the first element of each value of {@code by}. */
    public static Operator distinct(String by) {
        return stage("distinct", F.objectNode().put("by", by));
    }

    /** Replaces the list by consecutive lists of {@code count} elements, the last one possibly shorter. */
    public static Operator window(int count) {
        return stage("window", F.objectNode().put("count", count));
    }
}
