package dev.sagatoctopusorchestrator.conformance;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.sun.net.httpserver.HttpServer;
import dev.sagatoctopusorchestrator.sdk.ClientConfig;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatClient;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * The schedule steps of the conformance runner, played with the typed SDK against a recording server: what the
 * {@code action} of a scenario's write becomes on the wire, and what the runner prints of a schedule. The real suite
 * needs the platform; this proves the mapping without it.
 */
@Timeout(60)
class OperationsTest {
    private static final ObjectMapper JSON = new ObjectMapper();
    private static final String NS = "0190aaaa-0000-7000-8000-00000000aaaa";
    private static final String WF = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private static final String SCHEDULE = "0190aaaa-0000-7000-8000-0000000000a1";
    private static final List<String> OLD_ACTION_KEYS =
            List.of("workflowId", "versionPolicy", "versionId", "inputDigest", "nominalTimeField", "runTimeoutMs");

    private HttpServer server;
    private final List<JsonNode> bodies = new CopyOnWriteArrayList<>();
    private final List<String> methods = new CopyOnWriteArrayList<>();
    private volatile String answer = "{}";
    private volatile int status = 200;

    @BeforeEach
    void start() throws IOException {
        server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/", exchange -> {
            byte[] request = exchange.getRequestBody().readAllBytes();
            bodies.add(request.length == 0 ? JSON.nullNode() : JSON.readTree(request));
            methods.add(exchange.getRequestMethod());
            byte[] reply = answer.getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            exchange.getResponseHeaders().add("X-Trace-Id", "f".repeat(32));
            exchange.sendResponseHeaders(status, reply.length);
            try (OutputStream out = exchange.getResponseBody()) {
                out.write(reply);
            }
        });
        server.start();
    }

    @AfterEach
    void stop() {
        server.stop(0);
    }

    private SagatClient client() {
        return SagatClient.create(ClientConfig.builder()
                .apiUrl("http://127.0.0.1:" + server.getAddress().getPort() + "/v1")
                .namespace("payments")
                .token("tok-1")
                .retry(new RetryPolicy(1, Duration.ofMillis(5), Duration.ofMillis(20)))
                .build());
    }

    private static JsonNode json(String text) throws IOException {
        return JSON.readTree(text.replace('\'', '"'));
    }

    private static JsonNode config() throws IOException {
        return json("{'namespaceId':'" + NS + "'}");
    }

    /** What the server answers for a schedule whose action is {@code action}. */
    private static String schedule(String action) {
        return ("{'scheduleId':'" + SCHEDULE + "','name':'nightly','origin':'API','state':'ACTIVE','revision':2,"
                + "'createdAt':'2026-10-10T10:00:00Z','updatedAt':'2026-10-10T10:00:00Z','nextActionAt':'2099-01-01T09:00:00Z',"
                + "'action':" + action + "}").replace('\'', '"');
    }

    private static String plainAction() {
        return "{'workflowId':'" + WF + "','versionPolicy':'ACTIVE','inputDigest':'" + "a".repeat(64) + "'}";
    }

    private static String prefixedAction() {
        return "{'workflowId':'" + WF + "','versionPolicy':'ACTIVE','inputDigest':'" + "a".repeat(64)
                + "','businessIdPrefix':'order','businessIdConflictPolicy':'USE_EXISTING','businessIdReusePolicy':'REJECT_DUPLICATE'}";
    }

    private static List<String> names(JsonNode object) {
        List<String> out = new ArrayList<>();
        object.fieldNames().forEachRemaining(out::add);
        return out;
    }

    private JsonNode play(String step) throws Exception {
        Map<String, JsonNode> results = new LinkedHashMap<>();
        results.put("created", json("{'scheduleId':'" + SCHEDULE + "'}"));
        try (SagatClient client = client()) {
            return Operations.play(json(step), client, config(), results);
        }
    }

    @Test
    void aScenarioActionThatNamesAPrefixAndItsPoliciesIsSentThroughTheTypedWriteAndPrintedBackByName() throws Exception {
        status = 201;
        answer = schedule(prefixedAction());
        JsonNode printed = play("{'op':'scheduleCreate','write':{'name':'nightly','spec':{'cron':['0 9 * * *']},"
                + "'action':{'workflowId':'" + WF + "','input':{},'businessIdPrefix':' Order/1 ',"
                + "'businessIdConflictPolicy':'USE_EXISTING','businessIdReusePolicy':'REJECT_DUPLICATE'}}}");
        JsonNode sent = bodies.get(0).path("action");
        assertEquals(" Order/1 ", sent.path("businessIdPrefix").textValue(), "the prefix goes as given: " + sent);
        assertEquals("USE_EXISTING", sent.path("businessIdConflictPolicy").textValue());
        assertEquals("REJECT_DUPLICATE", sent.path("businessIdReusePolicy").textValue());

        JsonNode action = printed.path("action");
        assertEquals("order", action.path("businessIdPrefix").textValue(), "what is printed is what the server sent");
        assertEquals("USE_EXISTING", action.path("businessIdConflictPolicy").textValue());
        assertEquals("REJECT_DUPLICATE", action.path("businessIdReusePolicy").textValue());
        assertEquals("a".repeat(64), action.path("inputDigest").textValue());
    }

    @Test
    void aPrefixAloneAndAnEmptyPrefixAreSentAsTheyAreAndNoPolicyIsInventedForThem() throws Exception {
        answer = schedule("{'workflowId':'" + WF + "','businessIdPrefix':'x','businessIdConflictPolicy':'FAIL','businessIdReusePolicy':'ALLOW_DUPLICATE'}");
        play("{'op':'scheduleCreate','write':{'name':'nightly','spec':{'cron':['0 9 * * *']},"
                + "'action':{'workflowId':'" + WF + "','input':{},'businessIdPrefix':''}}}");
        JsonNode sent = bodies.get(0).path("action");
        assertTrue(sent.has("businessIdPrefix") && sent.get("businessIdPrefix").isTextual(), sent.toString());
        assertEquals("", sent.get("businessIdPrefix").textValue(), "the empty string is sent as the empty string");
        assertFalse(sent.has("businessIdConflictPolicy"), sent.toString());
        assertFalse(sent.has("businessIdReusePolicy"), sent.toString());
    }

    @Test
    void anUpdateStepCarriesTheSameActionDocumentAndAnActionWithoutThemSendsAndPrintsNoneOfTheThreeKeys() throws Exception {
        answer = schedule(prefixedAction());
        JsonNode with = play("{'op':'scheduleUpdate','schedule':'created','expectedRevision':2,'write':{'name':'nightly',"
                + "'spec':{'cron':['0 9 * * *']},'action':{'workflowId':'" + WF + "','input':{},'businessIdPrefix':'order'}}}");
        assertEquals("PUT", methods.get(0));
        assertEquals("order", bodies.get(0).path("action").path("businessIdPrefix").textValue());
        assertEquals("order", with.path("action").path("businessIdPrefix").textValue());

        bodies.clear();
        answer = schedule(plainAction());
        JsonNode without = play("{'op':'scheduleUpdate','schedule':'created','expectedRevision':3,'write':{'name':'nightly',"
                + "'spec':{'cron':['0 9 * * *']},'action':{'workflowId':'" + WF + "','input':{}}}}");
        JsonNode sent = bodies.get(0).path("action");
        assertEquals(List.of("workflowId", "input"), names(sent), "none of the three is sent when none is set");
        assertEquals(OLD_ACTION_KEYS, names(without.path("action")),
                "an action without a prefix prints the keys it always printed, so every golden is what it was");
    }

    @Test
    void aGetPrintsTheThreeKeysOnlyWhenTheActionHasThemAndAnActionOfANewerServerKeepsItsPolicyStrings() throws Exception {
        answer = schedule("{'workflowId':'" + WF + "','businessIdPrefix':'order','businessIdConflictPolicy':'NEWER',"
                + "'businessIdReusePolicy':'NEWER_TOO'}").replaceAll("}$", ",\"nextActions\":[],\"recentActions\":[],\"openRuns\":[],"
                + "\"bufferedCount\":0,\"counters\":{}}");
        JsonNode described = play("{'op':'scheduleGet','schedule':'created'}");
        assertEquals("NEWER", described.path("action").path("businessIdConflictPolicy").textValue());
        assertEquals("NEWER_TOO", described.path("action").path("businessIdReusePolicy").textValue());
        assertEquals("order", described.path("action").path("businessIdPrefix").textValue());

        answer = schedule(plainAction()).replaceAll("}$", ",\"nextActions\":[],\"recentActions\":[],\"openRuns\":[],"
                + "\"bufferedCount\":0,\"counters\":{}}");
        JsonNode plain = play("{'op':'scheduleGet','schedule':'created'}");
        assertEquals(OLD_ACTION_KEYS, names(plain.path("action")), plain.path("action").toString());
    }

    @Test
    void aMemberOfTheActionTheRunnerDoesNotMapIsStillRefusedSoARequestIsNeverSilentlyNarrowed() throws Exception {
        IllegalArgumentException refused = assertThrows(IllegalArgumentException.class, () -> play(
                "{'op':'scheduleCreate','write':{'name':'nightly','spec':{'cron':['0 9 * * *']},"
                        + "'action':{'workflowId':'" + WF + "','input':{},'businessIdSuffix':'x'}}}"));
        assertTrue(refused.getMessage().contains("businessIdSuffix"), refused.getMessage());
        assertTrue(bodies.isEmpty(), "nothing was sent");
    }
}
