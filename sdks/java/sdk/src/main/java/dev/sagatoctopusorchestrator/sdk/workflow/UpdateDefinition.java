package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * An atomic declarative validator, state reducer and result projection.
 *
 * <p>{@code reapplyOnReset} keeps the arguments of every applied instance so a reset can validate and apply them
 * again. It is written into the document only when true, and a server publishes it only when its capability is
 * on.
 */
public record UpdateDefinition(
        JsonNode argsSchema,
        String validate,
        String state,
        String result,
        JsonNode resultSchema,
        String disclosure,
        boolean reapplyOnReset) {

    /** An Update that does not keep its arguments for a reset, which is the default. */
    public UpdateDefinition(
            JsonNode argsSchema,
            String validate,
            String state,
            String result,
            JsonNode resultSchema,
            String disclosure) {
        this(argsSchema, validate, state, result, resultSchema, disclosure, false);
    }
}
