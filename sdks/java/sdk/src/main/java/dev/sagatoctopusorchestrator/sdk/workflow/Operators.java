package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.ArrayNode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * The operators of a collection, in order: the stages applied once, when the collection is entered, to the list
 * {@code items} yields (design 042). Pass it to {@link Flow#forEach(String, String, String, String, CollectionOptions,
 * Operators)} or {@code map}; an empty chain writes no {@code operators}. The server validates the chain.
 */
public final class Operators {
    private final List<Operator> stages;

    private Operators(List<Operator> stages) {
        this.stages = stages;
    }

    /** The stages, in the order given. */
    public static Operators of(Operator... stages) {
        return of(java.util.Arrays.asList(stages));
    }

    /** The stages, in the order given; the list is copied. */
    public static Operators of(List<Operator> stages) {
        if (stages == null) {
            throw SagatException.invalid("the operators need a list of stages");
        }
        List<Operator> copied = new ArrayList<>(stages.size());
        for (int position = 0; position < stages.size(); position++) {
            Operator stage = stages.get(position);
            if (stage == null) {
                throw SagatException.invalid("operator " + position + " is null: build it with Ops");
            }
            copied.add(stage);
        }
        return new Operators(Collections.unmodifiableList(copied));
    }

    /** The stages, in order, unmodifiable. */
    public List<Operator> stages() {
        return stages;
    }

    boolean isEmpty() {
        return stages.isEmpty();
    }

    ArrayNode toArray() {
        ArrayNode out = Ops.F.arrayNode();
        for (Operator stage : stages) {
            out.add(stage.toNode());
        }
        return out;
    }
}
