package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.io.StringWriter;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * What the client does with a hostile or broken server and a careless caller (SPEC sections 5, 8, 9 and 10): waits,
 * sizes, names, ids, credentials and the rule of what is repeated.
 */
@Timeout(120)
class RestHardeningTest {
    private static final String RUN = "0190aaaa-0000-7000-8000-000000000001";
    private static final String RUN_JSON = "{\"runId\":\"" + RUN + "\",\"workflowId\":\"0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee\",\"versionId\":\"0190aaaa-0000-7000-8000-000000000002\","
            + "\"status\":\"RUNNING\",\"createdAt\":\"2026-10-10T10:00:00Z\"}";
    private static FakeRestServer server;

    @BeforeAll
    static void start() throws IOException {
        server = new FakeRestServer();
    }

    @AfterAll
    static void stop() {
        server.close();
    }

    private static ClientConfig.Builder config(String apiUrl) {
        return ClientConfig.builder()
                .apiUrl(apiUrl)
                .namespace("payments")
                .token("tok")
                .retry(new RetryPolicy(4, Duration.ofMillis(2), Duration.ofMillis(10)));
    }

    private static SagatClient client(Consumer<ClientConfig.Builder> tweak) {
        ClientConfig.Builder builder = config(server.base());
        tweak.accept(builder);
        return new SagatClient(builder.build());
    }

    private static SagatClient client() {
        return client(b -> {});
    }

    private static String stackTraceOf(Throwable t) {
        StringWriter out = new StringWriter();
        t.printStackTrace(new PrintWriter(out));
        return out.toString();
    }

    // ------------------------------------------------------------------------------- waits from the server

    @Test
    void aWaitTheServerAsksForIsAMinuteAtMost() {
        SagatException hour = SagatException.builder(ErrorCode.RATE_LIMITED, "slow").retryAfterMs(3_600_000L).build();
        assertEquals(60_000, hour.retryAfterMs().getAsLong());
        SagatException absurd = SagatException.builder(ErrorCode.RATE_LIMITED, "slow").retryAfterMs(Long.MAX_VALUE).build();
        assertEquals(60_000, absurd.retryAfterMs().getAsLong());
        assertEquals(1500, SagatException.builder(ErrorCode.RATE_LIMITED, "slow").retryAfterMs(1500L).build().retryAfterMs().getAsLong());
        assertEquals(Duration.ofSeconds(60), config(server.base()).build().maxServerWait());
        assertEquals(60_000, RetryPolicy.DEFAULT.delayAfterMillis(0, hour, () -> 1.0));
    }

    @Test
    void theClientDoesNotSleepLongerThanItsCapBecauseTheServerAsksForAnHour() {
        server.prepare((req, reply) -> reply.send(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow\",\"retryable\":true}", "Retry-After", "86400"));
        SagatClient capped = client(b -> b.maxServerWait(Duration.ofMillis(300)).retry(new RetryPolicy(2, Duration.ofMillis(1), Duration.ofMillis(1))));
        SagatException error = assertTimeoutPreemptively(Duration.ofSeconds(10), () -> assertThrows(SagatException.class, () -> capped.get(RUN)));
        assertEquals(ErrorCode.RATE_LIMITED, error.code());
        assertEquals(2, server.seen.size(), "it waited the cap, not the hour, and tried again");
    }

    @Test
    void aRetryAfterWithMoreDigitsThanALongHoldsIsAVeryLongWaitNotNoWait() {
        server.prepare((req, reply) -> reply.send(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow\",\"retryable\":true}", "Retry-After", "99999999999999999999999"));
        SagatException error = assertThrows(SagatException.class, () -> client(b -> b.retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1)))).get(RUN));
        assertEquals(60_000, error.retryAfterMs().getAsLong());
    }

    @Test
    void anHttpDateInRetryAfterIsIgnoredAndNoWaitIsTaken() {
        server.prepare((req, reply) -> reply.send(429, "{\"code\":\"RATE_LIMITED\",\"message\":\"slow\",\"retryable\":true}", "Retry-After", "Wed, 21 Oct 2099 07:28:00 GMT"));
        SagatException error = assertTimeoutPreemptively(Duration.ofSeconds(10), () -> assertThrows(SagatException.class, () -> client().get(RUN)));
        assertTrue(error.retryAfterMs().isEmpty());
    }

    // --------------------------------------------------------------------------- sizes from the server

    private static HttpServer streaming(AtomicLong written, long totalBytes, boolean declareLength) throws IOException {
        HttpServer big = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        big.createContext("/", exchange -> {
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, declareLength ? totalBytes : 0);
            try (OutputStream out = exchange.getResponseBody()) {
                byte[] chunk = new byte[1 << 20];
                java.util.Arrays.fill(chunk, (byte) 'a');
                for (long sent = 0; sent < totalBytes; sent += chunk.length) {
                    out.write(chunk);
                    out.flush();
                    written.addAndGet(chunk.length);
                }
            } catch (IOException e) {
                // the client let go
            }
        });
        big.setExecutor(java.util.concurrent.Executors.newCachedThreadPool());
        big.start();
        return big;
    }

    @Test
    void anAnswerOverTheCapIsAProtocolErrorAndTheServerIsCutOff() throws Exception {
        AtomicLong written = new AtomicLong();
        long total = 256L * 1024 * 1024;
        HttpServer big = streaming(written, total, false);
        try {
            SagatClient c = new SagatClient(config("http://127.0.0.1:" + big.getAddress().getPort() + "/v1")
                    .timeout(Duration.ofSeconds(30)).retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1))).build());
            SagatException error = assertThrows(SagatException.class, () -> c.get(RUN));
            assertEquals(ErrorCode.PROTOCOL, error.code());
            assertEquals("the answer is larger than the SDK reads", error.getMessage());
            Thread.sleep(300);
            assertTrue(written.get() < total / 2, "the server wrote " + written.get() + " of " + total + " bytes before the client let go");
        } finally {
            big.stop(0);
        }
    }

    @Test
    void aDeclaredLengthOverTheCapIsRefusedWithoutWaitingForTheBody() throws Exception {
        AtomicLong written = new AtomicLong();
        HttpServer big = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        big.createContext("/", exchange -> {
            exchange.sendResponseHeaders(200, 600L * 1024 * 1024);
            exchange.getResponseBody().flush(); // the headers are on the wire; the body never comes
            try {
                Thread.sleep(4000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            exchange.close();
        });
        big.setExecutor(java.util.concurrent.Executors.newCachedThreadPool());
        big.start();
        try {
            SagatClient c = new SagatClient(config("http://127.0.0.1:" + big.getAddress().getPort() + "/v1")
                    .timeout(Duration.ofSeconds(2)).retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1))).build());
            long began = System.nanoTime();
            SagatException error = assertThrows(SagatException.class, () -> c.get(RUN));
            assertEquals(ErrorCode.PROTOCOL, error.code(), error.getMessage());
            assertTrue(TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - began) < 1500, "refused on the headers, not after the timeout");
            assertEquals(0, written.get());
        } finally {
            big.stop(0);
        }
    }

    @Test
    void aLargeAnswerUnderTheCapIsStillRead() {
        String pad = "p".repeat(2 * 1024 * 1024);
        server.prepare((req, reply) -> reply.send(200, RUN_JSON.substring(0, RUN_JSON.length() - 1) + ",\"pad\":\"" + pad + "\"}"));
        Run run = client().get(RUN);
        assertEquals(RUN, run.runId());
        assertEquals(pad.length(), run.raw().get("pad").textValue().length());
    }

    @Test
    void whatTheServerSaysIsBoundedItsMessageItsReasonAndItsTraceId() {
        server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"" + "M".repeat(100_000) + "\",\"traceId\":\"<script>alert(1)</script>\",\"details\":{\"reason\":\""
                + "R".repeat(100_000) + "\"}}", "X-Trace-Id", "also not hex"));
        SagatException error = assertThrows(SagatException.class, () -> client().get(RUN));
        assertTrue(error.getMessage().codePointCount(0, error.getMessage().length()) <= 512, "message " + error.getMessage().length());
        assertTrue(error.reason().orElseThrow().length() <= 128);
        assertEquals("", error.traceId(), "a trace id that is not 32 hexadecimal characters is not repeated");
    }

    @Test
    void aWellFormedTraceIdIsKeptFromTheEnvelopeAndFromTheHeader() {
        server.prepare((req, reply) -> reply.send(409, "{\"code\":\"CONFLICT\",\"message\":\"x\",\"traceId\":\"" + "ab".repeat(16) + "\"}"));
        assertEquals("ab".repeat(16), assertThrows(SagatException.class, () -> client().get(RUN)).traceId());
        server.prepare((req, reply) -> reply.send(409, "{\"code\":\"CONFLICT\",\"message\":\"x\"}", "X-Trace-Id", "cd".repeat(16)));
        assertEquals("cd".repeat(16), assertThrows(SagatException.class, () -> client().get(RUN)).traceId());
        server.prepare((req, reply) -> reply.send(409, "{\"code\":\"CONFLICT\",\"message\":\"x\"}", "X-Trace-Id", "AB".repeat(16)));
        assertEquals("", assertThrows(SagatException.class, () -> client().get(RUN)).traceId(), "upper case is not the 32 lower case hex of the contract");
    }

    @Test
    void aMessageIsCutByCodePointsAndNeverLeavesALoneSurrogate() {
        String emoji = "😀".repeat(600);
        SagatException cut = SagatException.of(ErrorCode.INTERNAL, emoji);
        assertEquals(512, cut.getMessage().codePointCount(0, cut.getMessage().length()));
        assertEquals(1024, cut.getMessage().length(), "never inside a pair");
        SagatException lone = SagatException.of(ErrorCode.INTERNAL, "x\uD800y");
        assertEquals("x�y", lone.getMessage());
        assertEquals("R".repeat(128), SagatException.builder(ErrorCode.INTERNAL, "x").reason("R".repeat(5000)).build().reason().orElseThrow());
        assertEquals("", SagatException.builder(ErrorCode.INTERNAL, "x").traceId("g".repeat(32)).build().traceId());
    }

    // ----------------------------------------------------------------------------------- names and ids

    private static final String[] BAD_RUN_IDS = {"..", ".", "", "a/b", "x", "r-1", "../runs", RUN + "/operations", "0190aaaa-0000-7000-8000-00000000000g", " " + RUN};

    @Test
    void aRunIdThatIsNotAUuidIsRefusedBeforeAnythingIsSentByEveryMethod() {
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        SagatClient c = client();
        for (String id : BAD_RUN_IDS) {
            List<org.junit.jupiter.api.function.Executable> calls = List.of(
                    () -> c.get(id),
                    () -> c.cancel(id, "no longer needed here", null),
                    () -> c.operate(id, OperateAction.PAUSE, "pausing it for now", null),
                    () -> c.signal(id, "name", Map.of(), null),
                    () -> c.events(id, null, null),
                    () -> c.waitForCompletion(id, WaitOptions.create().timeout(Duration.ofSeconds(1))),
                    () -> c.getAsync(id).join());
            for (org.junit.jupiter.api.function.Executable call : calls) {
                Throwable thrown = assertThrows(Throwable.class, call, "run id [" + id + "]");
                Throwable error = thrown instanceof java.util.concurrent.CompletionException ? thrown.getCause() : thrown;
                assertTrue(error instanceof SagatException e && e.code() == ErrorCode.VALIDATION_FAILED, "run id [" + id + "]: " + error);
            }
        }
        assertEquals(0, server.seen.size(), "nothing was sent: " + server.seen);
    }

    @Test
    void aSignalNameThatIsADotSegmentOrEmptyIsRefusedAndOtherNamesAreEncoded() {
        server.prepare((req, reply) -> reply.send(202, "{\"deliveredTo\":[]}"));
        SagatClient c = client();
        for (String name : new String[] {"", ".", ".."}) {
            SagatException error = assertThrows(SagatException.class, () -> c.signal(RUN, name, Map.of(), null), "[" + name + "]");
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        }
        assertEquals(0, server.seen.size());
        c.signal(RUN, "a/b ?#", Map.of(), null);
        c.signal(RUN, "payment.confirmed", Map.of(), null);
        assertEquals("/v1/runs/" + RUN + "/signals/a%2Fb%20%3F%23", server.seen.get(0).uri());
        assertEquals("/v1/runs/" + RUN + "/signals/payment.confirmed", server.seen.get(1).uri());
    }

    @Test
    void aRunIdInUpperCaseIsSentInLowerCase() {
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        client().get(RUN.toUpperCase());
        assertEquals("/v1/runs/" + RUN, server.seen.get(0).uri());
    }

    @Test
    void anEmptyWorkflowAndABadKeyOrTraceAreRefusedBeforeTheWorkflowIsLookedUp() {
        server.prepare((req, reply) -> reply.send(200, "{\"items\":[{\"id\":\"0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee\",\"name\":\"order-flow\"}],\"nextCursor\":null}"));
        SagatClient c = client();
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> c.start("", Map.of())).code());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> c.start("order-flow", Map.of(), StartOptions.create().idempotencyKey("short"))).code());
        assertEquals(ErrorCode.BAD_REQUEST, assertThrows(SagatException.class, () -> c.start("order-flow", Map.of(), StartOptions.create().traceparent("nope"))).code());
        assertEquals(0, server.seen.size(), "no request, not even the lookup of the name");
    }

    // ------------------------------------------------------------------------------------ credentials

    @Test
    void aTokenProviderThatThrowsIsUnauthenticatedWithAReasonAndKeepsNoneOfItsOwnError() {
        SagatClient c = client(b -> b.tokenProvider(() -> {
            throw new IllegalStateException("idp said: client_secret=SECRET123 refused for https://idp.example/token?code=abc");
        }));
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        SagatException error = assertThrows(SagatException.class, () -> c.get(RUN));
        assertEquals(ErrorCode.UNAUTHENTICATED, error.code());
        assertEquals("TOKEN_PROVIDER_FAILED", error.reason().orElseThrow());
        assertNull(error.getCause(), "the provider's exception is not kept: its stack trace carries what it was talking to");
        assertFalse(stackTraceOf(error).contains("SECRET123"));
        assertTrue(error.getMessage().contains("IllegalStateException"), "its class is enough: " + error.getMessage());
        assertFalse(error.getMessage().contains("SECRET123"));
        assertEquals(0, server.seen.size());

        SagatClient empty = client(b -> b.tokenProvider(() -> ""));
        SagatException none = assertThrows(SagatException.class, () -> empty.get(RUN));
        assertEquals("TOKEN_PROVIDER_FAILED", none.reason().orElseThrow());
    }

    @Test
    void theTextOfAMalformedPayloadIsNotKeptInTheCauseOfAnError() {
        SagatException parse = assertThrows(SagatException.class, () -> Json.parse("{\"secret\": sk_live_123"));
        assertFalse(stackTraceOf(parse).contains("sk_live_123"), "Jackson's message quotes the text it could not read");
        server.prepare((req, reply) -> reply.send(200, "sk_live_abc is not json"));
        SagatException answer = assertThrows(SagatException.class, () -> client().get(RUN));
        assertEquals(ErrorCode.PROTOCOL, answer.code());
        assertFalse(stackTraceOf(answer).contains("sk_live_abc"));
    }

    // --------------------------------------------------------------------- the rule of what is repeated

    private static void attempts(int status, String body, boolean get, int expected, String what) {
        server.prepare((req, reply) -> reply.send(status, body));
        SagatClient c = client();
        assertThrows(SagatException.class, () -> {
            if (get) {
                c.get(RUN);
            } else {
                c.start("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", Map.of(), StartOptions.create().idempotencyKey("key-12345678"));
            }
        });
        assertEquals(expected, server.seen.size(), what + (get ? " (GET)" : " (POST with a key)"));
    }

    @Test
    void theRuleIsByCodeASingleTableForGetAndForAKeyedPost() {
        String claims = ",\"message\":\"x\",\"retryable\":true}";
        for (boolean get : new boolean[] {true, false}) {
            attempts(408, "", get, 4, "408 without an envelope is UNAVAILABLE and is repeated");
            attempts(408, "{\"code\":\"UNAVAILABLE\"" + claims, get, 4, "408 with the envelope of UNAVAILABLE");
            attempts(429, "{\"code\":\"RATE_LIMITED\"" + claims, get, 4, "429 rate limited");
            attempts(429, "{\"code\":\"QUOTA_EXCEEDED\"" + claims, get, 1, "a quota is never repeated, whatever it claims");
            attempts(500, "{\"code\":\"INTERNAL\"" + claims, get, 4, "500 that says it may be repeated");
            attempts(500, "{\"code\":\"INTERNAL\",\"message\":\"x\",\"retryable\":false}", get, 1, "500 that says it may not");
            attempts(502, "<html>bad gateway</html>", get, 4, "502 without an envelope");
            attempts(503, "", get, 4, "503");
            attempts(504, "", get, 4, "504");
            attempts(400, "{\"code\":\"BAD_REQUEST\"" + claims, get, 1, "400 claiming to be repeatable");
            attempts(401, "{\"code\":\"UNAUTHENTICATED\"" + claims, get, 1, "401 claiming to be repeatable");
            attempts(403, "{\"code\":\"FORBIDDEN\"" + claims, get, 1, "403 claiming to be repeatable");
            attempts(409, "{\"code\":\"CONFLICT\"" + claims, get, 1, "409 claiming to be repeatable");
            attempts(422, "{\"code\":\"VALIDATION_FAILED\"" + claims, get, 1, "422 claiming to be repeatable");
            attempts(418, "", get, 1, "418");
        }
    }

    // ---------------------------------------------------------------------------- calls the server lacks

    private static void unsupported(int status, String body, String what) {
        server.prepare((req, reply) -> reply.send(status, body));
        SagatException error = assertThrows(SagatException.class, () -> client().get(RUN), what);
        assertEquals(ErrorCode.NOT_FOUND, error.code(), what);
        assertEquals("UNSUPPORTED_BY_SERVER", error.reason().orElse(null), what);
        assertEquals(status, error.httpStatus().getAsInt());
        assertFalse(error.retryable(), what);
        assertEquals(1, server.seen.size(), what);
    }

    @Test
    void aCallTheServerDoesNotHaveIsUnsupportedByServerInEveryShapeItComesIn() {
        unsupported(501, "", "501");
        unsupported(501, "{\"code\":\"INTERNAL\",\"message\":\"not implemented\"}", "501 with an envelope");
        unsupported(404, "", "404 without a body");
        unsupported(404, "<html>nginx</html>", "404 that is not the platform's");
        unsupported(404, "{\"code\":\"NOT_FOUND\",\"message\":\"no route\",\"details\":{\"reason\":\"ROUTE_NOT_FOUND\"}}", "404 whose envelope says ROUTE_NOT_FOUND");
        unsupported(405, "{\"code\":\"BAD_REQUEST\",\"message\":\"no\",\"details\":{\"reason\":\"METHOD_NOT_ALLOWED\"}}", "405 that says METHOD_NOT_ALLOWED");
    }

    @Test
    void aMissingRunIsAPlainNotFoundAndAMethodNotAllowedWithoutTheReasonIsNotUnsupported() {
        server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"no such run\",\"details\":{\"reason\":\"RUN_NOT_FOUND\"}}"));
        SagatException missing = assertThrows(SagatException.class, () -> client().get(RUN));
        assertEquals(ErrorCode.NOT_FOUND, missing.code());
        assertEquals("RUN_NOT_FOUND", missing.reason().orElseThrow(), "the server's own reason is kept");
        server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"no such run\"}"));
        assertTrue(assertThrows(SagatException.class, () -> client().get(RUN)).reason().isEmpty());
        server.prepare((req, reply) -> reply.send(405, "{\"code\":\"BAD_REQUEST\",\"message\":\"no\"}"));
        SagatException method = assertThrows(SagatException.class, () -> client().get(RUN));
        assertEquals(ErrorCode.BAD_REQUEST, method.code());
        assertTrue(method.reason().isEmpty());
    }

    // ------------------------------------------------------------------------- hooks of the caller

    @Test
    void aTraceHookThatThrowsDoesNotStopTheCallAndAThrowingLoggerDoesNotStopTheRetries() {
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        SagatClient hook = client(b -> b.traceContext(() -> {
            throw new IllegalStateException("tracer down");
        }));
        assertEquals(RUN, hook.get(RUN).runId());
        assertEquals(null, server.seen.get(0).header("traceparent"), "no ambient context, no header");

        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 3) {
                reply.send(503, "");
            } else {
                reply.send(200, RUN_JSON);
            }
        });
        SagatClient noisy = client(b -> b.logger(new ThrowingLogger().armed(true)));
        assertEquals(RUN, noisy.get(RUN).runId(), "the warning about the retry could not be written, the retry happened");
        assertEquals(3, server.seen.size());

        SagatClient errors = client(b -> b.logger(new ThrowingLogger(() -> new AssertionError("sink")).armed(true)));
        attempts.set(0);
        server.seen.clear();
        assertEquals(RUN, errors.get(RUN).runId(), "an Error of the sink is survived too");
    }

    // ------------------------------------------------------------------------------- hostile numbers

    @Test
    void aNumberWithAHostileExponentIsNotExpandedInMemory() throws Exception {
        for (String exponent : new String[] {"1e999999999", "1e-999999999", "1e2000000000", "123.456e999999999"}) {
            List<String> command = List.of(
                    Path.of(System.getProperty("java.home"), "bin", "java").toString(), "-Xmx48m", "-cp", System.getProperty("java.class.path"),
                    SmallHeapProbe.class.getName(), exponent);
            Process process = new ProcessBuilder(command).redirectErrorStream(true).start();
            String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
            assertTrue(process.waitFor(60, TimeUnit.SECONDS), "the probe did not finish for " + exponent);
            assertEquals(0, process.exitValue(), exponent + " in a 48 MB heap: " + output);
            String[] lines = output.strip().split("\n");
            assertEquals(2, lines.length, output);
            for (String line : lines) {
                int length = Integer.parseInt(line.substring(line.indexOf('=') + 1, line.indexOf(':')).replace("refused", "0"));
                assertTrue(length < 200, exponent + " -> " + line);
            }
        }
    }

    @Test
    void theCanonicalTextOfAHostileExponentIsRefusedOrZeroAndTheWireTextKeepsTheNumber() {
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> Json.canonical(Json.parse("{\"x\":1e999999999}"))).code());
        assertEquals("{\"x\":0}", Json.canonical(Json.parse("{\"x\":1e-999999999}")), "as a double, like JSON.parse: it underflows");
        assertEquals("{\"x\":1E+999999999}", Json.encode(Json.parse("{\"x\":1e999999999}")));
        assertEquals("{\"x\":1E-999999999}", Json.encode(Json.parse("{\"x\":1e-999999999}")));
        assertEquals("{\"x\":100000}", Json.encode(Json.parse("{\"x\":1e5}")), "an ordinary exponent still reads as digits");
        assertEquals("{\"x\":0.00001}", Json.encode(Json.parse("{\"x\":1e-5}")));
    }

    @Test
    void aRunWithAHostileNumberInItsAnswerIsReadPromptly() {
        server.prepare((req, reply) -> reply.send(200, RUN_JSON.substring(0, RUN_JSON.length() - 1) + ",\"version\":1e999999999,\"depth\":1e-999999999}"));
        Run run = assertTimeoutPreemptively(Duration.ofSeconds(5), () -> client().get(RUN));
        assertEquals(RUN, run.runId());
    }

    // ----------------------------------------------------------------------- credentials in string forms

    private static final String CANARY = "CANARY-bearer-4f9c1e7a";
    private static final String KEY_CANARY = "CANARYKEYMATERIAL0123456789abcdef";

    @Test
    void aConfigAClientAndAnOptionsObjectShowNoCredentialInTheirStringJsonOrSerializedForms() throws Exception {
        TlsOptions tls = TlsOptions.systemRoots().withClientCertificate("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
                "-----BEGIN PRIVATE KEY-----\n" + KEY_CANARY + "\n-----END PRIVATE KEY-----\n");
        ClientConfig config = ClientConfig.builder().apiUrl("https://api.example/v1").namespace("n").token(CANARY).build();
        SagatClient client = new SagatClient(config);
        List<Object> things = List.of(config, client, tls, config.tokenProvider(), TokenProvider.of(CANARY));
        for (Object thing : things) {
            String text = String.valueOf(thing);
            assertFalse(text.contains(CANARY) || text.contains(KEY_CANARY), thing.getClass().getSimpleName() + ".toString(): " + text);
            String json;
            try {
                json = Json.mapper().writeValueAsString(thing);
            } catch (Exception e) {
                json = e.getClass().getSimpleName();
            }
            assertFalse(json.contains(CANARY) || json.contains(KEY_CANARY), thing.getClass().getSimpleName() + " as JSON: " + json);
            java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();
            try (java.io.ObjectOutputStream out = new java.io.ObjectOutputStream(bytes)) {
                out.writeObject(thing);
            } catch (java.io.NotSerializableException e) {
                // the right answer: these objects are not serializable
            }
            assertFalse(new String(bytes.toByteArray(), StandardCharsets.ISO_8859_1).contains(CANARY), thing.getClass().getSimpleName() + " serialized");
        }
        client.close();
    }

    // ============================================================================== round 4

    @Test
    void aTokenProviderThatFailsAtFirstIsRepeatedBeforeAnythingIsSent() {
        AtomicInteger asked = new AtomicInteger();
        SagatClient c = client(b -> b.tokenProvider(() -> {
            if (asked.incrementAndGet() <= 2) {
                throw new IOException("idp down");
            }
            return "tok-" + asked.get();
        }));
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        assertEquals(RUN, c.get(RUN).runId());
        assertEquals(3, asked.get(), "asked three times: two failures and the token");
        assertEquals(1, server.seen.size(), "nothing was sent until there was a token");
        assertEquals("Bearer tok-3", server.seen.get(0).header("authorization"));
    }

    @Test
    void aTokenProviderThatNeverRecoversGivesUpAfterTheAttemptsWithTheReason() {
        AtomicInteger asked = new AtomicInteger();
        SagatClient c = client(b -> b.tokenProvider(() -> {
            asked.incrementAndGet();
            throw new IllegalStateException("client_secret=SECRET123");
        }));
        server.prepare((req, reply) -> reply.send(200, RUN_JSON));
        SagatException error = assertThrows(SagatException.class, () -> c.get(RUN));
        assertEquals(ErrorCode.UNAUTHENTICATED, error.code());
        assertEquals("TOKEN_PROVIDER_FAILED", error.reason().orElseThrow());
        assertEquals(4, asked.get(), "the attempts of the retry policy");
        assertEquals(0, server.seen.size());
        // a keyed POST is repeated too, with the same key (nothing was sent)
        asked.set(0);
        assertThrows(SagatException.class, () -> c.start(WORKFLOW_ID, Map.of(), StartOptions.create().idempotencyKey("key-12345678")));
        assertEquals(4, asked.get());
    }

    private static final String WORKFLOW_ID = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";

    @Test
    void theProviderFailureIsAPrivateFactAServerCannotMakeUp() {
        for (int status : new int[] {400, 401, 403, 404, 409, 422}) {
            String code = switch (status) {
                case 400 -> "BAD_REQUEST";
                case 401 -> "UNAUTHENTICATED";
                case 403 -> "FORBIDDEN";
                case 404 -> "NOT_FOUND";
                case 409 -> "CONFLICT";
                default -> "VALIDATION_FAILED";
            };
            server.prepare((req, reply) -> reply.send(status,
                    "{\"code\":\"" + code + "\",\"message\":\"x\",\"retryable\":true,\"details\":{\"reason\":\"TOKEN_PROVIDER_FAILED\"}}"));
            SagatException error = assertThrows(SagatException.class, () -> client().get(RUN));
            assertEquals("TOKEN_PROVIDER_FAILED", error.reason().orElseThrow(), "the server's words are kept as they are");
            assertFalse(error.isTokenProviderFailure(), status + ": what the server says does not make it the provider's");
            assertFalse(error.isRetriable(), String.valueOf(status));
            assertEquals(1, server.seen.size(), status + " claiming to be the provider's failure is not repeated");
        }
        // nor can a caller of the public API make one
        SagatException made = SagatException.builder(ErrorCode.UNAUTHENTICATED, "x").reason("TOKEN_PROVIDER_FAILED").retryable(true).build();
        assertFalse(made.isTokenProviderFailure());
        assertFalse(made.isRetriable());
    }
}
