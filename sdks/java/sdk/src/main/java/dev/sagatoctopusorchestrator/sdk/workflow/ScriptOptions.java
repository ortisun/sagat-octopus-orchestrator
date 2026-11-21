package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.util.ArrayList;
import java.util.List;

/** Options of a pinned script artifact node. */
public final class ScriptOptions {
    String language;
    RetryOptions retry;
    String compensateWith;
    List<String> secrets;
    String onError;
    JsonNode outputSchema;
    ConcurrencyPolicy concurrency;
    RateLimitPolicy rateLimit;

    public static ScriptOptions create() { return new ScriptOptions(); }
    public ScriptOptions language(String value) { language = value; return this; }
    public ScriptOptions retry(RetryOptions value) { retry = value; return this; }
    public ScriptOptions compensateWith(String value) { compensateWith = value; return this; }
    public ScriptOptions secrets(List<String> value) { secrets = value == null ? null : new ArrayList<>(value); return this; }
    public ScriptOptions onError(String value) { onError = value; return this; }
    /** How many attempts of one key may run at once (feature 022). Read when the script is added. */
    public ScriptOptions concurrency(ConcurrencyPolicy value) { concurrency = value; return this; }
    /** How many leases of one key the script may have per window (feature 022). Read when the script is added. */
    public ScriptOptions rateLimit(RateLimitPolicy value) { rateLimit = value; return this; }
    public ScriptOptions outputSchema(Object value) {
        JsonNode node = Json.toTree(value);
        if (!node.isObject()) throw SagatException.invalid("outputSchema is a JSON object");
        outputSchema = node.deepCopy();
        return this;
    }
}
