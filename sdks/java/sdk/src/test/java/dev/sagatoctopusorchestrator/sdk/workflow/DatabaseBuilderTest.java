package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.Test;

class DatabaseBuilderTest {
    @Test
    void sharedTypedFixtureAndCanonicalBytesArePreserved() throws Exception {
        JsonNode fixture = Json.parse(Files.readString(Path.of("../../../tests/contract/database-connections/typed-parameters.workflow.json")));
        JsonNode node = fixture.path("nodes").get(0);
        List<DatabaseOptions.Parameter> parameters = new ArrayList<>();
        for (JsonNode p : node.path("parameters")) {
            JsonNode literal = p.get("literal");
            parameters.add(literal != null
                ? DatabaseOptions.Parameter.literal(DatabaseOptions.Type.valueOf(literal.path("type").asText().toUpperCase(java.util.Locale.ROOT)), literal.get("value"))
                : DatabaseOptions.Parameter.expression(DatabaseOptions.Type.valueOf(p.path("type").asText().toUpperCase(java.util.Locale.ROOT)), p.path("expression").asText()));
        }
        DatabaseOptions options = DatabaseOptions.create(new DatabaseOptions.ConnectionRef("orders-db", 2), DatabaseOptions.Operation.QUERY, node.path("statement").asText()).parameters(parameters);
        assertEquals(Json.canonical(fixture), WorkflowBuilder.named(fixture.path("metadata").path("name").asText()).database(node.path("id").asText(), options).toJson());
    }

    @Test
    void snapshotsAndRejectsBadShapesWithoutPrintingValues() {
        ObjectNode value = (ObjectNode) Json.parse("{\"amount\":\"9223372036854775807\"}");
        DatabaseOptions options = query("orders-db", "SELECT $1").parameters(List.of(DatabaseOptions.Parameter.literal(DatabaseOptions.Type.JSON, value)));
        WorkflowBuilder builder = WorkflowBuilder.named("database-chain").database("q", options).end("done");
        value.put("amount", "changed");
        JsonNode node = builder.build().path("nodes").get(0);
        assertEquals("9223372036854775807", node.path("parameters").get(0).path("literal").path("value").path("amount").asText());
        assertEquals("done", node.path("next").asText());
        assertFalse(node.has("retry") || node.has("secrets"));
        for (DatabaseOptions bad : List.of(
            query("postgres://credential-canary@host/db", "SELECT 1"), query("orders-db", ""),
            query("orders-db", "SELECT $1").parameters(List.of(DatabaseOptions.Parameter.literal(DatabaseOptions.Type.INTEGER, 123))),
            query("orders-db", "SELECT $1").parameters(List.of(DatabaseOptions.Parameter.literal(DatabaseOptions.Type.DECIMAL, "credential-canary"))),
            DatabaseOptions.create(new DatabaseOptions.ConnectionRef("orders-db", 0), DatabaseOptions.Operation.QUERY, "SELECT 1")
        )) {
            SagatException error = assertThrows(SagatException.class, () -> WorkflowBuilder.named("database-invalid").database("q", bad));
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
            assertFalse(error.toString().contains("credential-canary"));
        }
    }

    private static DatabaseOptions query(String name, String statement) {
        return DatabaseOptions.create(new DatabaseOptions.ConnectionRef(name), DatabaseOptions.Operation.QUERY, statement);
    }
}
