package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/**
 * The answer to {@link SagatClient#signalWithStart}.
 *
 * @param runId       the run that holds the business identifier now
 * @param started     whether this request created the run
 * @param deliveredTo the nodes the signal woke
 * @param raw         the answer as sent
 */
public record SignalWithStartResult(String runId, boolean started, List<String> deliveredTo, JsonNode raw) {

    static SignalWithStartResult parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a signal-with-start result");
        return new SignalWithStartResult(
                Wire.str(raw, "runId"), raw.path("started").isBoolean() && raw.path("started").booleanValue(),
                Wire.strings(raw.path("deliveredTo")), raw);
    }
}
