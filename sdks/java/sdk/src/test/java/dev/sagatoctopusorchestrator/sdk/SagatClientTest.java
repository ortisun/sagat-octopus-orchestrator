package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/** The client against an in-process REST server (mirrors client.test.ts and adds what is specific to Java). */
@Timeout(60)
class SagatClientTest {
    private static final String WORKFLOW = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private static final String TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    private static FakeRestServer server;

    @BeforeAll
    static void startServer() throws IOException {
        server = new FakeRestServer();
    }

    @AfterAll
    static void stopServer() {
        server.close();
    }

    private static String run(String... overrides) {
        Map<String, String> fields = new java.util.LinkedHashMap<>();
        fields.put("runId", "\"0190aaaa-0000-7000-8000-000000000001\"");
        fields.put("workflowId", "\"" + WORKFLOW + "\"");
        fields.put("versionId", "\"0190aaaa-0000-7000-8000-000000000002\"");
        fields.put("status", "\"RUNNING\"");
        fields.put("version", "3");
        fields.put("paused", "false");
        fields.put("startedBy", "\"alice\"");
        fields.put("createdAt", "\"2026-10-10T10:00:00Z\"");
        fields.put("updatedAt", "\"2026-10-10T10:00:01+05:30\"");
        fields.put("finishedAt", "null");
        fields.put("deadline", "null");
        fields.put("traceId", "\"abcdefabcdefabcdefabcdefabcdefab\"");
        fields.put("correlationId", "null");
        fields.put("depth", "0");
        fields.put("parent", "null");
        fields.put("futureField", "7");
        for (int i = 0; i + 1 < overrides.length; i += 2) {
            fields.put(overrides[i], overrides[i + 1]);
        }
        StringBuilder out = new StringBuilder("{");
        fields.forEach((k, v) -> out.append(out.length() > 1 ? "," : "").append('"').append(k).append("\":").append(v));
        return out.append('}').toString();
    }

    private static SagatClient client(Consumer<ClientConfig.Builder> tweak) {
        ClientConfig.Builder builder = ClientConfig.builder()
                .apiUrl(server.base())
                .namespace("payments")
                .token("tok-1")
                .retry(new RetryPolicy(4, Duration.ofMillis(5), Duration.ofMillis(20)));
        tweak.accept(builder);
        return new SagatClient(builder.build());
    }

    private static SagatClient client() {
        return client(b -> {});
    }

    @Test
    void aStartSendsTheHeadersAndTheBodyTheApiExpectsAndReturnsATypedRunThatKeepsTheRawAnswer() {
        server.prepare((req, reply) -> reply.send(202, run()));
        Run result = client().start(
                WORKFLOW,
                Map.of("orderId", "o-1"),
                StartOptions.create()
                        .idempotencyKey("key-12345678")
                        .correlationId("corr-1")
                        .traceparent(TRACE)
                        .deadline(Instant.parse("2030-01-01T00:00:00Z")));
        FakeRestServer.Seen request = server.seen.get(0);
        assertEquals("POST", request.method());
        assertEquals("/v1/runs", request.path());
        assertEquals("Bearer tok-1", request.header("authorization"));
        assertEquals("payments", request.header("x-octopus-namespace"));
        assertEquals("key-12345678", request.header("idempotency-key"));
        assertEquals(TRACE, request.header("traceparent"));
        assertEquals("application/json", request.header("accept"));
        assertTrue(request.header("user-agent").matches("^sagat-sdk-java/\\d+\\.\\d+\\.\\d+.*"), request.header("user-agent"));
        assertEquals(
                "{\"workflowId\":\"" + WORKFLOW + "\",\"input\":{\"orderId\":\"o-1\"},\"correlationId\":\"corr-1\",\"deadline\":\"2030-01-01T00:00:00.000Z\"}",
                request.rawBody());
        assertEquals(RunStatus.RUNNING, result.status());
        assertEquals(Instant.parse("2026-10-10T10:00:00Z"), result.createdAt());
        assertEquals(Instant.parse("2026-10-10T04:30:01Z"), result.updatedAt(), "an offset in an answer is read into UTC");
        assertEquals(7, result.raw().get("futureField").intValue(), "a field of a newer server is reachable");
    }

    @Test
    void aCallWithoutAKeyGetsAFreshOnePerCallAndKeepsItAcrossItsRetries() {
        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 3) {
                reply.send(503, "{\"code\":\"UNAVAILABLE\",\"message\":\"busy\",\"retryable\":true,\"traceId\":\"" + "e".repeat(32) + "\"}");
            } else {
                reply.send(202, run());
            }
        });
        client().start(WORKFLOW, Map.of());
        assertEquals(3, server.seen.size());
        Set<String> keys = new java.util.HashSet<>();
        server.seen.forEach(s -> keys.add(s.header("idempotency-key")));
        assertEquals(1, keys.size(), "the same key on every attempt");
        String key = keys.iterator().next();
        assertTrue(key.matches("^[0-9a-f-]{36}$"), key);
        attempts.set(0);
        server.seen.clear();
        client().start(WORKFLOW, Map.of());
        assertNotEquals(key, server.seen.get(0).header("idempotency-key"), "another call, another key");
    }

    @Test
    void anEventWithoutAnIdGetsOnePerCallKeptAcrossRetriesSoARepetitionIsADuplicateAndNotASecondEvent() {
        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 2) {
                reply.send(503, "upstream down");
            } else {
                reply.send(202, "{\"eventId\":\"e\",\"source\":\"api\",\"status\":\"ACCEPTED\"}");
            }
        });
        EventAccepted accepted = client().publishEvent(EventInput.of("order.created", Map.of("id", 1)), null);
        assertEquals(EventAccepted.Status.ACCEPTED, accepted.status());
        assertEquals(2, server.seen.size());
        String first = server.seen.get(0).body().get("eventId").textValue();
        assertEquals(first, server.seen.get(1).body().get("eventId").textValue());
        assertTrue(first.matches("^[0-9a-f-]{36}$"), first);
    }

    @Test
    void getIsRetriedOnUnavailabilityAndOnRateLimitingHonouringRetryAfterAndGivesUpAfterTheAttempts() {
        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() == 1) {
                reply.send(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow\",\"retryable\":true,\"traceId\":\"" + "a".repeat(32) + "\"}", "Retry-After", "0");
            } else {
                reply.send(200, run());
            }
        });
        Run got = client().get("r-1");
        assertEquals("0190aaaa-0000-7000-8000-000000000001", got.runId());
        assertEquals(2, server.seen.size());

        server.prepare((req, reply) -> reply.send(503, "<html>bad gateway</html>"));
        SagatException error = assertThrows(SagatException.class, () -> client().get("r-1"));
        assertEquals(ErrorCode.UNAVAILABLE, error.code());
        assertEquals(503, error.httpStatus().getAsInt());
        assertEquals(4, server.seen.size(), "four attempts in all");
    }

    @Test
    void aRetryAfterLongerThanTheBackoffIsHonoured() {
        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() == 1) {
                reply.send(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow\",\"retryable\":true}", "Retry-After", "1");
            } else {
                reply.send(200, run());
            }
        });
        long began = System.nanoTime();
        client().get("r-1");
        assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - began) >= 950, "waited for Retry-After: 1");
    }

    @Test
    void a4xxOtherThan429IsNeverRetriedAndKeepsWhatTheServerSaid() {
        server.prepare((req, reply) -> reply.send(409, "{\"code\":\"CONFLICT\",\"message\":\"the idempotency key was used for another request\",\"retryable\":false,\"traceId\":\""
                + "c".repeat(32) + "\",\"details\":{\"reason\":\"IDEMPOTENCY_KEY_REUSED\"}}"));
        SagatException error = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of(), StartOptions.create().idempotencyKey("key-12345678")));
        assertEquals(ErrorCode.CONFLICT, error.code());
        assertEquals("IDEMPOTENCY_KEY_REUSED", error.reason().orElseThrow());
        assertEquals("c".repeat(32), error.traceId());
        assertFalse(error.retryable());
        assertFalse(error.remediation().isEmpty());
        assertEquals(1, server.seen.size());
    }

    @Test
    void onlyTransportUnavailableRateLimitedAndInternalAreRepeatedWhateverTheBodyClaims() {
        // A 4xx whose envelope says retryable is still not repeated; a 5xx that says so is.
        Object[][] refused = {
            {400, "BAD_REQUEST"}, {401, "UNAUTHENTICATED"}, {403, "FORBIDDEN"}, {404, "NOT_FOUND"}, {409, "CONFLICT"}, {422, "VALIDATION_FAILED"}};
        for (Object[] c : refused) {
            server.prepare((req, reply) -> reply.send((Integer) c[0], "{\"code\":\"" + c[1] + "\",\"message\":\"no\",\"retryable\":true}"));
            SagatException error = assertThrows(SagatException.class, () -> client().get("x"));
            assertTrue(error.retryable(), "the server said so: " + c[1]);
            assertFalse(error.isRetriable(), String.valueOf(c[1]));
            assertEquals(1, server.seen.size(), c[0] + " is not repeated");
        }
        server.prepare((req, reply) -> reply.send(500, "{\"code\":\"INTERNAL\",\"message\":\"oops\",\"retryable\":true}"));
        assertThrows(SagatException.class, () -> client().get("x"));
        assertEquals(4, server.seen.size(), "an INTERNAL that says it may be repeated is");
        server.prepare((req, reply) -> reply.send(500, "{\"code\":\"INTERNAL\",\"message\":\"oops\",\"retryable\":false}"));
        assertThrows(SagatException.class, () -> client().get("x"));
        assertEquals(1, server.seen.size(), "and one that does not say so is not");
        server.prepare((req, reply) -> reply.send(429, "{\"code\":\"QUOTA_EXCEEDED\",\"message\":\"full\",\"retryable\":false}"));
        assertThrows(SagatException.class, () -> client().get("x"));
        assertEquals(1, server.seen.size(), "a quota is not a rate limit");
    }

    @Test
    void aConfigurationThatCannotWorkIsRefusedWhenItIsBuilt() {
        for (String url : new String[] {"", "api.example/v1", "ftp://api.example/v1", "http://", "not a url"}) {
            SagatException error = assertThrows(SagatException.class, () -> ClientConfig.builder().apiUrl(url).namespace("p").token("t").build(), url);
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        }
        assertThrows(SagatException.class, () -> ClientConfig.builder().apiUrl("https://a.example/v1").token("t").build());
        assertThrows(SagatException.class, () -> ClientConfig.builder().apiUrl("https://a.example/v1").namespace("p").build());
        assertThrows(SagatException.class, () -> ClientConfig.builder().apiUrl("https://a.example/v1").namespace("p").token("t").timeout(Duration.ZERO).build());
    }

    @Test
    void aRefusedTokenAMissingResourceAndAValidationErrorAreToldApart() {
        Object[][] cases = {{401, "UNAUTHENTICATED"}, {403, "FORBIDDEN"}, {404, "NOT_FOUND"}, {422, "VALIDATION_FAILED"}};
        for (Object[] c : cases) {
            int status = (Integer) c[0];
            String code = (String) c[1];
            server.prepare((req, reply) -> reply.send(status, "{\"code\":\"" + code + "\",\"message\":\"no\",\"retryable\":false,\"traceId\":\"" + "b".repeat(32) + "\"}"));
            SagatException error = assertThrows(SagatException.class, () -> client().get("x"));
            assertEquals(ErrorCode.valueOf(code), error.code());
            assertEquals(status, error.httpStatus().getAsInt());
            assertEquals(1, server.seen.size(), String.valueOf(status));
        }
    }

    @Test
    void theTokenIsAskedForBeforeEveryRequestSoARotatedOneIsUsedAtOnce() {
        AtomicInteger n = new AtomicInteger();
        server.prepare((req, reply) -> reply.send(200, run()));
        SagatClient c = client(b -> b.tokenProvider(() -> "tok-" + n.incrementAndGet()));
        c.get("a");
        c.get("b");
        assertEquals(List.of("Bearer tok-1", "Bearer tok-2"), server.seen.stream().map(s -> s.header("authorization")).toList());
    }

    @Test
    void aFailingTokenProviderIsACanonicalErrorThatDoesNotEchoItsMessage() {
        SagatClient c = client(b -> b.tokenProvider(() -> {
            throw new IllegalStateException("vault said: password=hunter2");
        }));
        server.prepare((req, reply) -> reply.send(200, run()));
        SagatException error = assertThrows(SagatException.class, () -> c.get("a"));
        assertEquals(ErrorCode.UNAUTHENTICATED, error.code());
        assertFalse(error.getMessage().contains("hunter2"));
        assertEquals(0, server.seen.size());
    }

    @Test
    void aNameIsResolvedToTheIdOfItsWorkflowThroughEveryPageAndRemembered() {
        server.prepare((req, reply) -> {
            if (req.path().startsWith("/v1/workflows")) {
                boolean second = req.uri().contains("cursor=c2");
                reply.send(200, second
                        ? "{\"items\":[{\"id\":\"" + WORKFLOW + "\",\"name\":\"order-flow\"}],\"nextCursor\":null}"
                        : "{\"items\":[{\"id\":\"0190aaaa-0000-7000-8000-00000000ffff\",\"name\":\"other\"}],\"nextCursor\":\"c2\"}");
            } else {
                reply.send(202, run());
            }
        });
        SagatClient c = client();
        c.start("order-flow", Map.of());
        c.start("order-flow", Map.of());
        long lookups = server.seen.stream().filter(s -> s.path().startsWith("/v1/workflows")).count();
        assertEquals(2, lookups, "two pages, once");
        FakeRestServer.Seen post = server.seen.stream().filter(s -> s.method().equals("POST")).findFirst().orElseThrow();
        assertEquals(WORKFLOW, post.body().get("workflowId").textValue());

        server.prepare((req, reply) -> reply.send(200, "{\"items\":[],\"nextCursor\":null}"));
        SagatException missing = assertThrows(SagatException.class, () -> client().start("nothing-here", Map.of()));
        assertEquals(ErrorCode.NOT_FOUND, missing.code());
        assertEquals("WORKFLOW_NOT_FOUND", missing.reason().orElseThrow());
    }

    @Test
    void aRedirectIsNeverFollowedItIsAProtocolErrorAndTheOtherServerSeesNothing() throws Exception {
        try (FakeRestServer elsewhere = new FakeRestServer()) {
            elsewhere.prepare((req, reply) -> reply.send(200, run()));
            for (int status : new int[] {301, 302, 303, 307, 308}) {
                server.prepare((req, reply) -> reply.send(status, "", "Location", elsewhere.base() + "/runs/r"));
                SagatException get = assertThrows(SagatException.class, () -> client().get("r"), "GET " + status);
                assertEquals(ErrorCode.PROTOCOL, get.code());
                assertEquals(status, get.httpStatus().getAsInt());
                assertEquals("the server redirected the call", get.getMessage());
                assertFalse(get.retryable());
                assertFalse(get.getMessage().contains("127.0.0.1"), "the Location is not repeated");
                assertEquals(1, server.seen.size(), "a redirect is not repeated");

                SagatException post = assertThrows(SagatException.class,
                        () -> client().start(WORKFLOW, Map.of("a", 1), StartOptions.create().idempotencyKey("key-12345678")), "POST " + status);
                assertEquals(ErrorCode.PROTOCOL, post.code());
                assertEquals(2, server.seen.size());
            }
            assertEquals(0, elsewhere.seen.size(), "the second server never saw a request: no token, no key, nothing");
        }
    }

    @Test
    void aHttpClientTheCallerSuppliesMustNotFollowRedirectsEither() throws Exception {
        java.net.http.HttpClient following = java.net.http.HttpClient.newBuilder().followRedirects(java.net.http.HttpClient.Redirect.ALWAYS).build();
        SagatException refused = assertThrows(SagatException.class, () -> JdkHttpTransport.using(following));
        assertEquals(ErrorCode.VALIDATION_FAILED, refused.code());
        java.net.http.HttpClient normally = java.net.http.HttpClient.newBuilder().followRedirects(java.net.http.HttpClient.Redirect.NORMAL).build();
        assertThrows(SagatException.class, () -> JdkHttpTransport.using(normally));

        try (FakeRestServer elsewhere = new FakeRestServer()) {
            elsewhere.prepare((req, reply) -> reply.send(200, run()));
            server.prepare((req, reply) -> reply.send(307, "", "Location", elsewhere.base() + "/runs/r"));
            HttpTransport supplied = JdkHttpTransport.using(java.net.http.HttpClient.newBuilder().followRedirects(java.net.http.HttpClient.Redirect.NEVER).build());
            SagatException error = assertThrows(SagatException.class, () -> client(b -> b.transport(supplied)).get("r"));
            assertEquals(ErrorCode.PROTOCOL, error.code());
            assertEquals(0, elsewhere.seen.size());
        }
        // The JDK's own default is also "never".
        JdkHttpTransport.using(java.net.http.HttpClient.newHttpClient());
    }

    @Test
    void theDeadlineOfACallCoversTheWholeExchangeEvenWhenTheBodyStalls() throws Exception {
        com.sun.net.httpserver.HttpServer stalling = com.sun.net.httpserver.HttpServer.create(new java.net.InetSocketAddress("127.0.0.1", 0), 0);
        java.util.concurrent.CountDownLatch release = new java.util.concurrent.CountDownLatch(1);
        stalling.createContext("/", exchange -> {
            exchange.sendResponseHeaders(200, 1000); // promises 1000 bytes
            exchange.getResponseBody().write('{');
            exchange.getResponseBody().flush();
            try {
                release.await(10, TimeUnit.SECONDS);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            exchange.close();
        });
        stalling.setExecutor(java.util.concurrent.Executors.newCachedThreadPool());
        stalling.start();
        try {
            SagatClient slow = new SagatClient(ClientConfig.builder()
                    .apiUrl("http://127.0.0.1:" + stalling.getAddress().getPort() + "/v1")
                    .namespace("p")
                    .token("t")
                    .timeout(Duration.ofMillis(400))
                    .retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1)))
                    .build());
            long began = System.nanoTime();
            SagatException error = assertThrows(SagatException.class, () -> slow.get("r"));
            long took = TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - began);
            assertEquals(ErrorCode.TRANSPORT, error.code());
            assertEquals("the request timed out", error.getMessage());
            assertTrue(error.retryable());
            assertTrue(took < 3000, "gave up after " + took + " ms, not when the server let go");
        } finally {
            release.countDown();
            stalling.stop(0);
            ((java.util.concurrent.ExecutorService) stalling.getExecutor()).shutdownNow();
        }
    }

    @Test
    void aMalformedTraceparentOrKeyIsRefusedBeforeAnythingIsSent() {
        server.prepare((req, reply) -> reply.send(202, run()));
        SagatException trace = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of(), StartOptions.create().traceparent("not-a-trace")));
        assertEquals(ErrorCode.BAD_REQUEST, trace.code());
        SagatException key = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of(), StartOptions.create().idempotencyKey("short")));
        assertEquals(ErrorCode.VALIDATION_FAILED, key.code());
        assertEquals(0, server.seen.size());
    }

    @Test
    void anIdempotencyKeyIs8To160VisibleAsciiCharacters() {
        server.prepare((req, reply) -> reply.send(202, run()));
        String[] valid = {"12345678", "a".repeat(160), "!~!~!~!~", "key-with.dots_and:colons/1"};
        for (String key : valid) {
            client().start(WORKFLOW, Map.of(), StartOptions.create().idempotencyKey(key));
        }
        assertEquals(valid.length, server.seen.size());
        String[] invalid = {"1234567", "a".repeat(161), "has space", "tab\there", "caf\u00e9-key-1", "del\u007f-key-1", "new\nline-1", ""};
        for (String key : invalid) {
            SagatException error = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of(), StartOptions.create().idempotencyKey(key)), key);
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
            assertFalse(error.getMessage().contains(key.isEmpty() ? "\u0000" : key), "a key is not repeated in a message");
        }
        assertEquals(valid.length, server.seen.size(), "nothing invalid was sent");
    }

    @Test
    void plainTextOutsideLoopbackLogsOneWarningAtStartAndNothingElseDoes() {
        HttpTransport dead = call -> {
            throw new IOException("down");
        };
        for (String loopbackOrSecure : List.of("http://127.0.0.1:8080/v1", "http://localhost/v1", "http://[::1]:8080/v1", "https://api.example.com/v1")) {
            CapturingLogger quiet = new CapturingLogger();
            new SagatClient(ClientConfig.builder().apiUrl(loopbackOrSecure).namespace("p").token("t").logger(quiet).transport(dead).build());
            assertEquals(0, quiet.warnings("plain text").size(), loopbackOrSecure);
        }
        CapturingLogger log = new CapturingLogger();
        SagatClient plain = new SagatClient(ClientConfig.builder()
                .apiUrl("http://api.example.com/v1").namespace("p").token("tok-secret-1").logger(log).transport(dead)
                .retry(new RetryPolicy(2, Duration.ofMillis(1), Duration.ofMillis(1))).build());
        assertEquals(1, log.warnings("plain text").size(), "one warning, at start");
        assertThrows(SagatException.class, () -> plain.get("r"));
        assertThrows(SagatException.class, () -> plain.get("r"));
        assertEquals(1, log.warnings("plain text").size(), "not repeated by the calls");
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("tok-secret")), "no log line holds the token");
    }

    @Test
    void theAmbientTraceContextIsUsedWhenTheCallHasNone() {
        server.prepare((req, reply) -> reply.send(200, run()));
        client(b -> b.traceContext(() -> TRACE)).get("r");
        assertEquals(TRACE, server.seen.get(0).header("traceparent"));
        server.prepare((req, reply) -> reply.send(200, run()));
        client(b -> b.traceContext(() -> null)).get("r");
        assertEquals(null, server.seen.get(0).header("traceparent"), "no context, no header");
    }

    @Test
    void aListFollowsItsCursorADeadlineGivenAsADurationIsComputedOnceAndAnUnwritableNumberIsRefused() {
        server.prepare((req, reply) -> reply.send(200, req.uri().contains("cursor=n2")
                ? "{\"items\":[" + run("runId", "\"r-2\"") + "],\"nextCursor\":null}"
                : "{\"items\":[" + run("runId", "\"r-1\"") + "],\"nextCursor\":\"n2\"}"));
        List<String> ids = new ArrayList<>();
        client().listAll(ListOptions.create().status(RunStatus.RUNNING).limit(1)).forEach(r -> ids.add(r.runId()));
        assertEquals(List.of("r-1", "r-2"), ids);
        assertTrue(server.seen.get(0).uri().contains("status=RUNNING") && server.seen.get(0).uri().contains("limit=1"));
        assertFalse(server.seen.get(0).uri().contains("cursor="));

        server.prepare((req, reply) -> reply.send(202, run()));
        Instant before = Instant.now();
        client().start(WORKFLOW, Map.of(), StartOptions.create().deadlineIn(Duration.ofSeconds(60)));
        Instant sent = Instant.parse(server.seen.get(0).body().get("deadline").textValue());
        assertTrue(!sent.isBefore(before.plusSeconds(60).minusMillis(1)) && sent.isBefore(before.plusSeconds(61)), sent.toString());
        assertTrue(server.seen.get(0).body().get("deadline").textValue().endsWith("Z"));

        SagatException nan = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of("nan", Double.NaN)));
        assertEquals(ErrorCode.VALIDATION_FAILED, nan.code());
        SagatException infinite = assertThrows(SagatException.class, () -> client().start(WORKFLOW, Map.of("inf", Double.POSITIVE_INFINITY)));
        assertEquals(ErrorCode.VALIDATION_FAILED, infinite.code());
    }

    @Test
    void anAnswerListedLazilyReadsOnePageAtATime() {
        AtomicInteger pages = new AtomicInteger();
        server.prepare((req, reply) -> {
            int n = pages.incrementAndGet();
            reply.send(200, "{\"items\":[" + run("runId", "\"p" + n + "\"") + "],\"nextCursor\":\"c" + n + "\"}");
        });
        try (var stream = client().listAll(ListOptions.create().limit(1))) {
            assertEquals("p1", stream.findFirst().orElseThrow().runId());
        }
        assertEquals(1, pages.get(), "the second page is not read until it is needed");
    }

    @Test
    void integersBigDecimalsAndStringsGoOutAsTheyWereGiven() {
        server.prepare((req, reply) -> reply.send(202, run()));
        JsonNode input = Json.parse("{\"id\":9007199254740993,\"max\":9223372036854775807,\"n\":600000,\"half\":0.5,\"amount\":\"19.99\",\"scaled\":1.50,\"big\":123456789012345678901234567890}");
        client().start(WORKFLOW, input);
        assertEquals(
                "{\"id\":9007199254740993,\"max\":9223372036854775807,\"n\":600000,\"half\":0.5,\"amount\":\"19.99\",\"scaled\":1.50,\"big\":123456789012345678901234567890}",
                extractInput(server.seen.get(0).rawBody()));
    }

    private static String extractInput(String body) {
        int start = body.indexOf("\"input\":") + "\"input\":".length();
        return body.substring(start, body.length() - 1);
    }

    @Test
    void noAnswerAtAllIsARetryableTransportErrorAndALostAnswerIsRetriedOnlyBecauseTheCallCarriesAKey() {
        AtomicInteger calls = new AtomicInteger();
        HttpTransport real = new JdkHttpTransport(Duration.ofSeconds(5));
        HttpTransport lossy = call -> {
            if (calls.incrementAndGet() == 1) {
                // The request reached the server (it is recorded) and the answer was lost.
                real.send(call);
                throw new IOException("connection reset");
            }
            return real.send(call);
        };
        server.prepare((req, reply) -> reply.send(202, run()));
        Run result = client(b -> b.transport(lossy)).start(WORKFLOW, Map.of(), StartOptions.create().idempotencyKey("key-12345678"));
        assertEquals(RunStatus.RUNNING, result.status());
        assertEquals(2, server.seen.size());
        assertEquals(server.seen.get(0).header("idempotency-key"), server.seen.get(1).header("idempotency-key"));

        SagatClient dead = new SagatClient(ClientConfig.builder()
                .apiUrl("http://127.0.0.1:1/v1")
                .namespace("p")
                .token("t")
                .retry(new RetryPolicy(2, Duration.ofMillis(1), Duration.ofMillis(2)))
                .build());
        SagatException error = assertThrows(SagatException.class, () -> dead.get("x"));
        assertEquals(ErrorCode.TRANSPORT, error.code());
        assertTrue(error.retryable());
        assertTrue(error.httpStatus().isEmpty());
        assertTrue(error.status().isEmpty());
        assertEquals("", error.traceId());
    }

    @Test
    void cancelAndSignalAreOperationsWithAReasonAndAPayloadAndTheResultIsTyped() {
        server.prepare((req, reply) -> reply.send(202, req.path().endsWith("/operations")
                ? "{\"operationId\":\"op-1\",\"accepted\":true,\"runStatus\":\"CANCELLING\",\"runVersion\":9}"
                : "{\"deliveredTo\":[\"wait\"]}"));
        OperationResult cancelled = client().cancel("r-1", "no longer needed", OperateOptions.create().expectedRunVersion(3L));
        assertEquals("op-1", cancelled.operationId());
        assertTrue(cancelled.accepted());
        assertEquals("CANCELLING", cancelled.runStatus());
        assertEquals(9L, cancelled.runVersion());
        assertEquals("{\"action\":\"CANCEL\",\"reason\":\"no longer needed\",\"expectedRunVersion\":3}", server.seen.get(0).rawBody());
        SignalResult signalled = client().signal("r-1", "payment-confirmed", Map.of("paymentId", "p-1"), null);
        assertEquals(List.of("wait"), signalled.deliveredTo());
        assertEquals("/v1/runs/r-1/signals/payment-confirmed", server.seen.get(1).path());
        assertEquals("{\"paymentId\":\"p-1\"}", server.seen.get(1).rawBody());
    }

    @Test
    void pathSegmentsAreEncodedLikeEncodeURIComponent() {
        server.prepare((req, reply) -> reply.send(202, "{\"deliveredTo\":[]}"));
        client().signal("a b/c", "x y", Map.of(), null);
        assertEquals("/v1/runs/a%20b%2Fc/signals/x%20y", server.seen.get(0).uri());
    }

    @Test
    void waitingForCompletionPollsUntilTheRunIsTerminal() {
        AtomicInteger polls = new AtomicInteger();
        server.prepare((req, reply) -> reply.send(200, run("status", polls.incrementAndGet() < 3 ? "\"RUNNING\"" : "\"SUCCEEDED\"")));
        Run done = client().waitForCompletion("r", WaitOptions.create().pollInterval(Duration.ofMillis(5)).timeout(Duration.ofSeconds(5)));
        assertEquals(RunStatus.SUCCEEDED, done.status());
        assertTrue(done.isTerminal());
        assertEquals(3, polls.get());

        server.prepare((req, reply) -> reply.send(200, run()));
        SagatException timeout = assertThrows(SagatException.class,
                () -> client().waitForCompletion("r", WaitOptions.create().pollInterval(Duration.ofMillis(5)).timeout(Duration.ofMillis(30))));
        assertEquals("WAIT_TIMEOUT", timeout.reason().orElseThrow());
    }

    @Test
    void aCallTheOlderServerDoesNotHaveIsNotFoundWithTheReasonUnsupportedByServer() {
        server.prepare((req, reply) -> reply.send(404, ""));
        SagatException routing = assertThrows(SagatException.class, () -> client().get("r"));
        assertEquals(ErrorCode.NOT_FOUND, routing.code());
        assertEquals("UNSUPPORTED_BY_SERVER", routing.reason().orElseThrow());
        assertEquals(1, server.seen.size(), "a 404 is not repeated");

        server.prepare((req, reply) -> reply.send(501, "<html>not implemented</html>"));
        SagatException notImplemented = assertThrows(SagatException.class, () -> client().get("r"));
        assertEquals(ErrorCode.NOT_FOUND, notImplemented.code());
        assertEquals("UNSUPPORTED_BY_SERVER", notImplemented.reason().orElseThrow());
        assertEquals(501, notImplemented.httpStatus().getAsInt());
        assertFalse(notImplemented.retryable());

        server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"no such run\",\"retryable\":false}"));
        SagatException missing = assertThrows(SagatException.class, () -> client().get("r"));
        assertEquals(ErrorCode.NOT_FOUND, missing.code());
        assertTrue(missing.reason().isEmpty(), "a missing run is not an unsupported call");
    }

    @Test
    void everyAnswerKeepsTheRawJson() {
        server.prepare((req, reply) -> reply.send(200, "{\"items\":[" + run() + "],\"nextCursor\":null,\"extra\":1}"));
        assertEquals(1, client().list(null).raw().get("extra").intValue());
        server.prepare((req, reply) -> reply.send(200, "{\"items\":[],\"nextCursor\":null,\"extra\":2}"));
        assertEquals(2, client().events("r", null, null).raw().get("extra").intValue());
        server.prepare((req, reply) -> reply.send(202, "{\"deliveredTo\":[],\"extra\":3}"));
        assertEquals(3, client().signal("r", "n", null, null).raw().get("extra").intValue());
        server.prepare((req, reply) -> reply.send(202, "{\"eventId\":\"e\",\"source\":\"api\",\"status\":\"DUPLICATE\",\"extra\":4}"));
        EventAccepted duplicate = client().publishEvent(EventInput.of("t", null), null);
        assertEquals(EventAccepted.Status.DUPLICATE, duplicate.status());
        assertEquals(4, duplicate.raw().get("extra").intValue());
    }

    @Test
    void aStatusANewerServerAddedIsUnknownButItsTextIsKept() {
        server.prepare((req, reply) -> reply.send(200, run("status", "\"HIBERNATING\"")));
        Run got = client().get("r");
        assertEquals(RunStatus.UNKNOWN, got.status());
        assertEquals("HIBERNATING", got.statusText());
        assertFalse(got.isTerminal());
    }

    @Test
    void asyncCallsReturnFuturesAndCancellingOneInterruptsAWait() throws Exception {
        server.prepare((req, reply) -> reply.send(200, run()));
        try (SagatClient c = client()) {
            Run viaFuture = c.getAsync("r").get(5, TimeUnit.SECONDS);
            assertEquals(RunStatus.RUNNING, viaFuture.status());

            server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"no\",\"retryable\":false}"));
            CompletableFuture<Run> failing = c.getAsync("missing");
            CompletionException wrapped = assertThrows(CompletionException.class, failing::join);
            assertTrue(wrapped.getCause() instanceof SagatException e && e.code() == ErrorCode.NOT_FOUND);

            server.prepare((req, reply) -> reply.send(200, run()));
            CompletableFuture<Run> waiting = c.waitForCompletionAsync("r", WaitOptions.create().pollInterval(Duration.ofMillis(20)).timeout(Duration.ofMinutes(5)));
            Thread.sleep(100);
            long before = server.seen.size();
            assertTrue(waiting.cancel(true));
            Thread.sleep(200);
            long after = server.seen.size();
            Thread.sleep(200);
            assertEquals(after, server.seen.size(), "the wait stopped polling once cancelled (" + before + " -> " + after + ")");
        }
    }

    @Test
    void theTransportHookSeesEveryCallAndCanBeWrapped() {
        List<String> seenByHook = new ArrayList<>();
        HttpTransport real = new JdkHttpTransport(Duration.ofSeconds(5));
        server.prepare((req, reply) -> reply.send(200, run()));
        client(b -> b.transport(call -> {
            seenByHook.add(call.method() + " " + call.uri().getPath());
            return real.send(call);
        })).get("r-9");
        assertEquals(List.of("GET /v1/runs/r-9"), seenByHook);
    }

    @Test
    void everyInstantOfARunIsUtcWhateverTheZoneOfTheProcess() {
        assertEquals("Asia/Kolkata", java.util.TimeZone.getDefault().getID(), "the build runs the tests in a zone that is not UTC");
        server.prepare((req, reply) -> reply.send(200, run("deadline", "\"2030-01-01T05:30:00+05:30\"")));
        Run got = client().get("r");
        assertEquals("2030-01-01T00:00:00Z", got.deadline().toString());
        assertEquals("2026-10-10T10:00:00Z", got.createdAt().toString());
        assertNotNull(Json.instant(got.createdAt()));
        assertEquals("2026-10-10T10:00:00.000Z", Json.instant(got.createdAt()));
    }
}
