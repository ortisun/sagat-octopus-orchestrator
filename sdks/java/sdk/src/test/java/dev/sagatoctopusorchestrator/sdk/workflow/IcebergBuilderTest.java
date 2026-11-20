package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.workflow.DatabaseOptions.ConnectionRef;
import dev.sagatoctopusorchestrator.sdk.workflow.IcebergWriteOptions.Limits;
import dev.sagatoctopusorchestrator.sdk.workflow.IcebergWriteOptions.Mapping;
import dev.sagatoctopusorchestrator.sdk.workflow.IcebergWriteOptions.Records;
import dev.sagatoctopusorchestrator.sdk.workflow.IcebergWriteOptions.Table;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;
import org.junit.jupiter.api.Test;

class IcebergBuilderTest {
    private static IcebergWriteOptions base() {
        return IcebergWriteOptions.create(new ConnectionRef("lake-main", 3), new Table(List.of("sales"), "orders"), Records.inline("steps.extract.output.rows"));
    }

    private static JsonNode nodeOf(IcebergWriteOptions options) {
        return WorkflowBuilder.named("lake").icebergWrite("w", options).end("done").build().path("nodes").get(0);
    }

    private static String repeat(String text, int times) {
        return text.repeat(times);
    }

    @Test
    void matchesTheSharedValidDocument() throws Exception {
        JsonNode fixture = Json.parse(Files.readString(Path.of("../../../tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json")));
        String built = WorkflowBuilder.named(fixture.path("metadata").path("name").asText())
            .step("extract", "orders.extract").icebergWrite("load", base()).end("done").toJson();
        assertEquals(Json.canonical(fixture), built);
    }

    @Test
    void theModeIsAlwaysAppendAndAMinimalWriteCarriesNothingItWasNotGiven() {
        JsonNode node = nodeOf(base());
        assertEquals("append", node.path("mode").asText());
        assertEquals("icebergWrite", node.path("type").asText());
        for (String absent : List.of("ref", "mapping", "limits", "compensation", "ignoreUnknownFields", "retry", "secrets", "overwrite")) {
            assertFalse(node.has(absent), absent);
        }
    }

    @Test
    void everyOptionIsCarriedAndTheThreeRecordSourcesAreAccepted() {
        WorkflowBuilder builder = WorkflowBuilder.named("lake").icebergWrite("w", base()
            .ref("staging")
            .mapping(List.of(new Mapping("/amount", "amount"), new Mapping("/customer/name", "customer_name")))
            .ignoreUnknownFields(true)
            .limits(new Limits().maxRecordBytes(1024).maxRecords(100000000).timeoutMs(3600000))
            .compensation(IcebergWriteOptions.Compensation.REVERT)
            .onError("recover")).step("recover", "orders.recover").end("done");
        JsonNode node = builder.build().path("nodes").get(0);
        assertEquals("staging", node.path("ref").asText());
        assertEquals(1024, node.path("limits").path("maxRecordBytes").asLong());
        assertEquals(100000000, node.path("limits").path("maxRecords").asLong());
        assertEquals(3600000, node.path("limits").path("timeoutMs").asLong());
        assertEquals("revert", node.path("compensation").asText());
        assertEquals("recover", node.path("onError").asText());
        assertTrue(node.path("ignoreUnknownFields").asBoolean());
        for (Records records : List.of(Records.inline("input.rows"), Records.blobs("steps.a.output.parts"), Records.databaseResult("read"))) {
            nodeOf(IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of("sales"), "orders"), records));
        }
    }

    @Test
    void theNodeIsASnapshotLaterChangesToTheOptionsDoNotReachIt() {
        List<Mapping> mapping = new ArrayList<>(List.of(new Mapping("/a", "a")));
        List<String> namespace = new ArrayList<>(List.of("sales"));
        WorkflowBuilder builder = WorkflowBuilder.named("lake")
            .icebergWrite("w", IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(namespace, "orders"), Records.inline("x")).mapping(mapping))
            .end("done");
        mapping.set(0, new Mapping("/changed", "changed"));
        namespace.set(0, "changed");
        String text = builder.toJson();
        assertFalse(text.contains("changed"), text);
    }

    private static void refused(String name, Supplier<IcebergWriteOptions> options) {
        SagatException error = assertThrows(SagatException.class, () -> WorkflowBuilder.named("lake").icebergWrite("w", options.get()), name);
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code(), name);
        assertTrue(error.getMessage().contains("invalid iceberg write"), name);
        assertFalse(error.toString().contains("credential-canary"), name);
    }

    @Test
    void whatTheServerWouldRefuseIsRefusedHereWithOneFixedMessageThatShowsNoContent() {
        Table table = new Table(List.of("sales"), "orders");
        Records inline = Records.inline("rows");
        Map<String, Supplier<IcebergWriteOptions>> cases = new java.util.LinkedHashMap<>();
        cases.put("connection is a url", () -> IcebergWriteOptions.create(new ConnectionRef("postgres://credential-canary@host/db"), table, inline));
        cases.put("revision zero", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main", 0), table, inline));
        cases.put("double dash", () -> IcebergWriteOptions.create(new ConnectionRef("lake--main"), table, inline));
        cases.put("no connection", () -> IcebergWriteOptions.create(null, table, inline));
        cases.put("no namespace", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of(), "orders"), inline));
        cases.put("null namespace", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(null, "orders"), inline));
        cases.put("nine levels", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of("a", "b", "c", "d", "e", "f", "g", "h", "i"), "orders"), inline));
        cases.put("empty name", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of("sales"), ""), inline));
        cases.put("dot dot name", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of("sales"), ".."), inline));
        cases.put("control in a level", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), new Table(List.of("sales\0"), "orders"), inline));
        cases.put("empty ref", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, inline).ref(""));
        cases.put("long ref", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, inline).ref(repeat("r", 256)));
        cases.put("no records", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, null));
        cases.put("null expression", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, Records.inline(null)));
        cases.put("empty expression", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, Records.inline("")));
        cases.put("expression of 1025 bytes", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, Records.inline(repeat("x", 1025))));
        cases.put("513 two-byte letters", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, Records.blobs(repeat("é", 513))));
        cases.put("empty database result", () -> IcebergWriteOptions.create(new ConnectionRef("lake-main"), table, Records.databaseResult("")));
        cases.put("pointer without slash", () -> base().mapping(List.of(new Mapping("amount", "amount"))));
        cases.put("pointer with empty segment", () -> base().mapping(List.of(new Mapping("/a//b", "a"))));
        cases.put("pointer with trailing slash", () -> base().mapping(List.of(new Mapping("/a/", "a"))));
        cases.put("pointer with a bad escape", () -> base().mapping(List.of(new Mapping("/a~2", "a"))));
        cases.put("pointer with 17 segments", () -> base().mapping(List.of(new Mapping("/" + repeat("a/", 16) + "a", "a"))));
        cases.put("empty column", () -> base().mapping(List.of(new Mapping("/a", ""))));
        cases.put("column twice", () -> base().mapping(List.of(new Mapping("/a", "c"), new Mapping("/b", "c"))));
        cases.put("field twice", () -> base().mapping(List.of(new Mapping("/a", "c"), new Mapping("/a", "d"))));
        cases.put("null row", () -> base().mapping(java.util.Arrays.asList((Mapping) null)));
        cases.put("zero records", () -> base().limits(new Limits().maxRecords(0)));
        cases.put("too many records", () -> base().limits(new Limits().maxRecords(100000001)));
        cases.put("record bytes below the edge", () -> base().limits(new Limits().maxRecordBytes(1023)));
        cases.put("record bytes above the edge", () -> base().limits(new Limits().maxRecordBytes(2147483649L)));
        cases.put("timeout below the edge", () -> base().limits(new Limits().timeoutMs(999)));
        cases.put("timeout above the edge", () -> base().limits(new Limits().timeoutMs(3600001)));
        cases.put("empty on error", () -> base().onError(""));
        cases.forEach(IcebergBuilderTest::refused);
        assertThrows(SagatException.class, () -> WorkflowBuilder.named("lake").icebergWrite("w", null));
    }

    @Test
    void theEdgesThatAreAcceptedAreAccepted() {
        ConnectionRef ref = new ConnectionRef("lake-main");
        Records inline = Records.inline("rows");
        Map<String, IcebergWriteOptions> cases = new java.util.LinkedHashMap<>();
        cases.put("eight levels and a 255 byte name", IcebergWriteOptions.create(ref, new Table(List.of("a", "b", "c", "d", "e", "f", "g", "h"), repeat("n", 255)), inline));
        cases.put("255 byte ref", base().ref(repeat("r", 255)));
        cases.put("1024 byte expression", IcebergWriteOptions.create(ref, new Table(List.of("sales"), "orders"), Records.inline(repeat("x", 1024))));
        cases.put("512 two-byte letters", IcebergWriteOptions.create(ref, new Table(List.of("sales"), "orders"), Records.inline(repeat("é", 512))));
        cases.put("16 segment pointer", base().mapping(List.of(new Mapping("/" + repeat("a/", 15) + "a", repeat("c", 255)))));
        cases.put("escapes", base().mapping(List.of(new Mapping("/a~0b/c~1d", "c"))));
        cases.put("lowest and highest limits", base().limits(new Limits().maxRecordBytes(2147483648L).maxRecords(1).timeoutMs(1000)));
        cases.put("shortest connection name", IcebergWriteOptions.create(new ConnectionRef("abc"), new Table(List.of("sales"), "orders"), inline));
        cases.forEach((name, options) -> assertDoesNotThrow(() -> nodeOf(options), name));
        JsonNode empty = nodeOf(base().mapping(List.of()).limits(new Limits()));
        assertTrue(empty.path("mapping").isArray() && empty.path("mapping").isEmpty(), "a supplied empty mapping must stay");
        assertTrue(empty.path("limits").isObject() && empty.path("limits").isEmpty(), "supplied empty limits must stay");
    }

    @Test
    void aNodeChainsLikeAnyOtherNextIsLinkedByTheFlow() {
        JsonNode nodes = WorkflowBuilder.named("lake").step("extract", "orders.extract").icebergWrite("load", base())
            .step("notify", "notify.done").end("done").build().path("nodes");
        assertEquals("load", nodes.get(0).path("next").asText());
        assertEquals("notify", nodes.get(1).path("next").asText());
    }
}
