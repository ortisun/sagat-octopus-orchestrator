package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HashSet;
import java.util.Set;
import java.util.TreeSet;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.junit.jupiter.api.Test;

/** What the SDK shares with the repository: the worker protocol, the error catalog, the version. */
class SharedContractsTest {
    /** The Maven module directory ({@code sdks/java/sdk}); surefire sets {@code basedir}. */
    private static final Path MODULE = Paths.get(System.getProperty("basedir", "sdks/java/sdk")).toAbsolutePath();
    private static final Path REPOSITORY = MODULE.getParent().getParent().getParent();

    @Test
    void theEmbeddedErrorCatalogIsTheSharedOne() throws IOException {
        JsonNode shared = Json.mapper().readTree(Files.readAllBytes(REPOSITORY.resolve("sdks/error-catalog.json")));
        JsonNode embedded;
        try (InputStream in = ErrorCatalog.class.getResourceAsStream(ErrorCatalog.RESOURCE)) {
            assertNotNull(in, "the catalog is in the jar");
            embedded = Json.mapper().readTree(in);
        }
        assertEquals(shared, embedded, "run sdks/java/scripts/sync-shared.sh");
        // The typed view of it agrees with the file, row by row.
        assertEquals(shared.get("errors").size(), ErrorCatalog.entries().size());
        for (int i = 0; i < shared.get("errors").size(); i++) {
            JsonNode row = shared.get("errors").get(i);
            ErrorCatalog.Entry entry = ErrorCatalog.entries().get(i);
            assertEquals(row.get("code").textValue(), entry.code().name());
            assertEquals(row.get("remediation").textValue(), entry.remediation());
            assertEquals(row.get("retryable").booleanValue(), entry.retryable());
            assertEquals(row.get("grpc").textValue(), entry.grpc());
            assertEquals(row.get("origin").textValue(), entry.origin());
        }
    }

    @Test
    void theWorkerProtocolCompiledIntoTheSdkIsTheOneOfTheContracts() throws IOException {
        byte[] contract = Files.readAllBytes(REPOSITORY.resolve("specs/001-platform-foundation/contracts/worker.proto"));
        byte[] shipped = Files.readAllBytes(MODULE.resolve("src/main/proto/worker.proto"));
        assertArrayEquals(contract, shipped, "run sdks/java/scripts/sync-shared.sh");
    }

    @Test
    void everyCodeOfTheEnumIsInTheCatalogAndTheOtherWayAround() {
        Set<String> inCatalog = new TreeSet<>();
        ErrorCatalog.entries().forEach(entry -> inCatalog.add(entry.code().name()));
        Set<String> inEnum = new TreeSet<>();
        for (ErrorCode code : ErrorCode.values()) {
            inEnum.add(code.name());
        }
        assertEquals(inCatalog, inEnum);
        assertEquals(ErrorCode.values().length, new HashSet<>(ErrorCatalog.entries().stream().map(ErrorCatalog.Entry::code).toList()).size());
    }

    @Test
    void everyErrorHasARemediationAndADefault() {
        for (ErrorCatalog.Entry entry : ErrorCatalog.entries()) {
            assertTrue(entry.remediation().length() > 20, entry.code().name());
            assertEquals(entry.remediation(), entry.code().remediation());
            assertEquals(entry.retryable(), entry.code().retryableByDefault());
        }
    }

    @Test
    void anErrorCarriesTheCodeTheTraceAndTheRemediationAndSaysNothingElse() {
        SagatException error = SagatException.builder(ErrorCode.RATE_LIMITED, "slow down")
                .httpStatus(429)
                .traceId("a".repeat(32))
                .reason("TENANT")
                .retryAfterMs(2000L)
                .build();
        assertTrue(error.retryable(), "the default of the code");
        assertEquals(ErrorCode.RATE_LIMITED.remediation(), error.remediation());
        assertEquals("429", error.status().orElseThrow());
        assertEquals(2000, error.retryAfter().orElseThrow().toMillis());
        assertEquals("TENANT", error.reason().orElseThrow());
        SagatException classified = SagatException.builder(ErrorCode.INTERNAL, "x").retryable(true).build();
        assertTrue(classified.retryable(), "the server's own classification wins");
        SagatException grpc = SagatException.builder(ErrorCode.TRANSPORT, "x").grpcStatus("UNAVAILABLE").build();
        assertEquals("UNAVAILABLE", grpc.status().orElseThrow());
        assertTrue(grpc.httpStatus().isEmpty());
        SagatException none = SagatException.of(ErrorCode.TRANSPORT, "x");
        assertTrue(none.status().isEmpty());
        assertEquals("", none.traceId());
    }

    @Test
    void aStatusWithoutAnEnvelopeIsClassifiedByTheCatalog() {
        assertEquals(ErrorCode.UNAUTHENTICATED, ErrorCatalog.codeOfHttpStatus(401));
        assertEquals(ErrorCode.NOT_FOUND, ErrorCatalog.codeOfHttpStatus(404));
        assertEquals(ErrorCode.CONFLICT, ErrorCatalog.codeOfHttpStatus(409));
        assertEquals(ErrorCode.VALIDATION_FAILED, ErrorCatalog.codeOfHttpStatus(422));
        assertEquals(ErrorCode.UNAVAILABLE, ErrorCatalog.codeOfHttpStatus(503));
        assertEquals(ErrorCode.INTERNAL, ErrorCatalog.codeOfHttpStatus(500));
        assertEquals(ErrorCode.BAD_REQUEST, ErrorCatalog.codeOfHttpStatus(418));
        assertEquals(ErrorCode.BAD_REQUEST, ErrorCatalog.codeOfHttpStatus(413));
    }

    @Test
    void theVersionInTheCodeIsTheVersionOfThePom() throws IOException {
        String pom = Files.readString(MODULE.getParent().resolve("pom.xml"), StandardCharsets.UTF_8);
        Matcher match = Pattern.compile("<artifactId>sagat-octopus-orchestrator-sdk-parent</artifactId>\\s*<version>([^<]+)</version>").matcher(pom);
        assertTrue(match.find());
        assertEquals(match.group(1), SagatSdk.VERSION);
        assertEquals("sagat-sdk-java/" + SagatSdk.VERSION, SagatSdk.userAgent(null));
        assertEquals("sagat-sdk-java/" + SagatSdk.VERSION + " my-app/2", SagatSdk.userAgent("my-app/2"));
        assertFalse(SagatSdk.PROTOCOL_VERSION.isEmpty());
    }

    @Test
    void theTraceparentRulesAreTheW3COnes() {
        assertTrue(Traceparents.isValid("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"));
        assertFalse(Traceparents.isValid("00-" + "0".repeat(32) + "-b7ad6b7169203331-01"));
        assertFalse(Traceparents.isValid("00-0af7651916cd43dd8448eb211c80319c-" + "0".repeat(16) + "-01"));
        assertFalse(Traceparents.isValid("01-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"));
        assertFalse(Traceparents.isValid("00-0AF7651916CD43DD8448EB211C80319C-b7ad6b7169203331-01"), "lower case only");
        assertFalse(Traceparents.isValid(null));
        assertEquals("0af7651916cd43dd8448eb211c80319c", Traceparents.traceIdOf("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"));
        assertEquals(null, Traceparents.traceIdOf("nope"));
        assertTrue(Traceparents.isValid(Traceparents.create()));
    }

    @Test
    void theBackoffIsExponentialCappedAndJitteredBetweenHalfAndWhole() {
        RetryPolicy policy = RetryPolicy.DEFAULT;
        assertEquals(4, policy.maxAttempts());
        assertEquals(200, policy.backoffMillis(0, () -> 1.0));
        assertEquals(100, policy.backoffMillis(0, () -> 0.0));
        assertEquals(400, policy.backoffMillis(1, () -> 1.0));
        assertEquals(5000, policy.backoffMillis(10, () -> 1.0), "capped at maxDelay");
        assertEquals(2500, policy.backoffMillis(10, () -> 0.0));
        SagatException asked = SagatException.builder(ErrorCode.RATE_LIMITED, "x").retryAfterMs(9000L).build();
        assertEquals(9000, policy.delayAfterMillis(0, asked, () -> 1.0), "what the server asked for, when larger");
        assertEquals(200, policy.delayAfterMillis(0, SagatException.of(ErrorCode.UNAVAILABLE, "x"), () -> 1.0));
    }
}
