package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import java.util.Map;
import org.junit.jupiter.api.Test;

/** {@code reapplyOnReset} on an Update declaration (feature 018): an optional boolean, written only when true. */
class ReapplyOnResetTest {
    private static final JsonNode INTEGER = Json.parse("{\"type\":\"integer\"}");

    private static JsonNode declared(UpdateDefinition update) {
        JsonNode schema = Json.parse(
                "{\"type\":\"object\",\"properties\":{\"balance\":{\"type\":\"integer\"}},"
                        + "\"required\":[\"balance\"],\"additionalProperties\":false}");
        JsonNode document = WorkflowBuilder.named("account-state")
                .state(new StateDefinition(schema, "input.account", Map.of(), Map.of("balance.credit", update)))
                .end("done")
                .build();
        return document.path("state").path("updates").path("balance.credit");
    }

    @Test
    void anUpdateThatOptsInWritesTheField() {
        JsonNode entry = declared(new UpdateDefinition(
                INTEGER, null, "state", "newState.balance", INTEGER, "declared-business-result-v1", true));
        assertTrue(entry.path("reapplyOnReset").isBoolean());
        assertTrue(entry.path("reapplyOnReset").booleanValue());
    }

    @Test
    void theDefaultIsNotWrittenSoADocumentThatDoesNotOptInIsTheOneMadeBeforeTheField() {
        JsonNode sixArguments = declared(new UpdateDefinition(
                INTEGER, null, "state", "newState.balance", INTEGER, "declared-business-result-v1"));
        JsonNode explicitFalse = declared(new UpdateDefinition(
                INTEGER, null, "state", "newState.balance", INTEGER, "declared-business-result-v1", false));
        assertFalse(sixArguments.has("reapplyOnReset"));
        assertFalse(explicitFalse.has("reapplyOnReset"));
        assertEquals(Json.canonical(sixArguments), Json.canonical(explicitFalse));
    }
}
