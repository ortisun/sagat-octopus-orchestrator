package dev.sagatoctopusorchestrator.sdk.worker;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.protocol.v1.WorkerServiceGrpc;
import dev.sagatoctopusorchestrator.sdk.CapturingLogger;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.TlsFixture;
import dev.sagatoctopusorchestrator.sdk.TlsOptions;
import io.grpc.Grpc;
import io.grpc.Metadata;
import io.grpc.Server;
import io.grpc.ServerCall;
import io.grpc.ServerCallHandler;
import io.grpc.ServerInterceptor;
import io.grpc.ServerInterceptors;
import io.grpc.TlsServerCredentials;
import io.grpc.stub.StreamObserver;
import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.security.cert.Certificate;
import java.time.Duration;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.TimeUnit;
import javax.net.ssl.SSLPeerUnverifiedException;
import javax.net.ssl.SSLSession;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * The worker host over TLS and mTLS against in-process gRPC servers, with identities made by keytool at test time:
 * the gRPC channel, as the REST one is tested in {@code TlsTest}.
 */
@Timeout(120)
class WorkerTlsTest {
    private static final class Registering extends WorkerServiceGrpc.WorkerServiceImplBase {
        @Override
        public void registerWorker(Worker.RegisterWorkerRequest request, StreamObserver<Worker.RegisterWorkerResponse> out) {
            out.onNext(Worker.RegisterWorkerResponse.newBuilder().setWorkerId("w").setSessionToken("s").setAcceptedProtocolVersion("1.2").build());
            out.onCompleted();
        }

        @Override
        public void pollTasks(Worker.PollTasksRequest request, StreamObserver<Worker.PollTasksResponse> out) {
            out.onNext(Worker.PollTasksResponse.newBuilder().setRetryAfterMs(1000).build());
            out.onCompleted();
        }
    }

    private static TlsFixture serverIdentity;
    private static TlsFixture clientIdentity;
    private static TlsFixture stranger;
    private static TlsFixture elsewhere;
    private static Server mutual;
    private static Server oneWay;
    private static Server wrongHost;
    /** The certificates of the client, as the servers saw them on each call. */
    private static final List<List<Certificate>> SEEN_BY_MUTUAL = new CopyOnWriteArrayList<>();
    private static final List<Integer> CLIENT_CERTIFICATES_SEEN_BY_ONE_WAY = new CopyOnWriteArrayList<>();

    private static byte[] bytes(String pem) {
        return pem.getBytes(StandardCharsets.UTF_8);
    }

    private static Server serve(TlsFixture identity, TlsFixture trustedClient, java.util.function.Consumer<SSLSession> seen) throws Exception {
        TlsServerCredentials.Builder credentials = TlsServerCredentials.newBuilder()
                .keyManager(new ByteArrayInputStream(bytes(identity.certificatePem)), new ByteArrayInputStream(bytes(identity.pkcs8KeyPem)));
        if (trustedClient != null) {
            credentials.trustManager(new ByteArrayInputStream(bytes(trustedClient.certificatePem))).clientAuth(TlsServerCredentials.ClientAuth.REQUIRE);
        }
        ServerInterceptor capture = new ServerInterceptor() {
            @Override
            public <Q, R> ServerCall.Listener<Q> interceptCall(ServerCall<Q, R> call, Metadata headers, ServerCallHandler<Q, R> next) {
                seen.accept(call.getAttributes().get(Grpc.TRANSPORT_ATTR_SSL_SESSION));
                return next.startCall(call, headers);
            }
        };
        return Grpc.newServerBuilderForPort(0, credentials.build())
                .addService(ServerInterceptors.intercept(new Registering(), capture))
                .build()
                .start();
    }

    @BeforeAll
    static void start() throws Exception {
        serverIdentity = TlsFixture.create("server");
        clientIdentity = TlsFixture.create("client");
        stranger = TlsFixture.create("stranger");
        elsewhere = TlsFixture.create("elsewhere", "dns:other.example");
        mutual = serve(serverIdentity, clientIdentity, session -> {
            try {
                SEEN_BY_MUTUAL.add(List.of(session.getPeerCertificates()));
            } catch (SSLPeerUnverifiedException e) {
                SEEN_BY_MUTUAL.add(List.of());
            }
        });
        oneWay = serve(serverIdentity, null, session -> {
            try {
                CLIENT_CERTIFICATES_SEEN_BY_ONE_WAY.add(session.getPeerCertificates().length);
            } catch (SSLPeerUnverifiedException e) {
                CLIENT_CERTIFICATES_SEEN_BY_ONE_WAY.add(0);
            }
        });
        wrongHost = serve(elsewhere, null, session -> {});
    }

    @AfterAll
    static void stop() throws InterruptedException {
        for (Server server : new Server[] {mutual, oneWay, wrongHost}) {
            server.shutdownNow().awaitTermination(2, TimeUnit.SECONDS);
        }
    }

    private static WorkerHost hostFor(Server server, TlsOptions tls, System.Logger logger) {
        WorkerConfig.Builder builder = WorkerConfig.builder()
                .grpcUrl("https://127.0.0.1:" + server.getPort())
                .namespace("payments")
                .token("tok")
                .workerName("tls-worker")
                .retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1)));
        if (tls != null) {
            builder.tls(tls);
        }
        if (logger != null) {
            builder.logger(logger);
        }
        WorkerHost host = new WorkerHost(builder.build());
        host.registerActivity("a", (in, ctx) -> 1);
        return host;
    }

    private static void registersAndStops(WorkerHost host) {
        try {
            host.start();
            assertEquals("1.2", host.protocolVersion());
        } finally {
            host.stop(Duration.ofSeconds(1));
        }
    }

    @Test
    void withMutualTlsTheServerSeesTheClientCertificateAndTheHostRegisters() throws Exception {
        SEEN_BY_MUTUAL.clear();
        registersAndStops(hostFor(mutual, TlsOptions.trusting(serverIdentity.certificatePem).withClientCertificate(clientIdentity.certificatePem, clientIdentity.pkcs8KeyPem), null));
        assertTrue(SEEN_BY_MUTUAL.size() >= 1);
        assertEquals(1, SEEN_BY_MUTUAL.get(0).size(), "the server saw one certificate");
        assertArrayEquals(clientIdentity.certificate().getEncoded(), SEEN_BY_MUTUAL.get(0).get(0).getEncoded(), "and it is the client's own");
    }

    @Test
    void aServerThatRequiresAClientCertificateRefusesAHostWithoutOne() {
        SEEN_BY_MUTUAL.clear();
        SagatException refused = assertThrows(SagatException.class, hostFor(mutual, TlsOptions.trusting(serverIdentity.certificatePem), null)::start);
        assertEquals(ErrorCode.TRANSPORT, refused.code());
        assertTrue(SEEN_BY_MUTUAL.isEmpty(), "no call got through the handshake");
    }

    @Test
    void aServerThatDoesNotAskForAClientCertificateIsServedWithTheConfiguredCaAndSeesNone() {
        CLIENT_CERTIFICATES_SEEN_BY_ONE_WAY.clear();
        registersAndStops(hostFor(oneWay, TlsOptions.trusting(serverIdentity.certificatePem), null));
        assertTrue(CLIENT_CERTIFICATES_SEEN_BY_ONE_WAY.stream().allMatch(count -> count == 0), "without mTLS configured, no client certificate is sent");
    }

    @Test
    void aServerCertificateTheHostDoesNotTrustFailsAsTransportWhateverTheConfiguration() {
        assertEquals(ErrorCode.TRANSPORT, assertThrows(SagatException.class, hostFor(oneWay, TlsOptions.trusting(stranger.certificatePem), null)::start).code());
        assertEquals(ErrorCode.TRANSPORT, assertThrows(SagatException.class, hostFor(oneWay, null, null)::start).code());
        assertEquals(ErrorCode.TRANSPORT, assertThrows(SagatException.class, hostFor(oneWay, TlsOptions.systemRoots(), null)::start).code());
    }

    @Test
    void theHostNameIsVerifiedEvenWhenTheCertificateIsTrusted() {
        // The server's certificate is trusted, and names other.example, not 127.0.0.1.
        SagatException error = assertThrows(SagatException.class, hostFor(wrongHost, TlsOptions.trusting(elsewhere.certificatePem), null)::start);
        assertEquals(ErrorCode.TRANSPORT, error.code());
    }

    @Test
    void malformedPemIsAValidationErrorAndNeverTheKey() {
        SagatException notPem = assertThrows(SagatException.class, hostFor(oneWay, TlsOptions.trusting("not a certificate"), null)::start);
        assertEquals(ErrorCode.VALIDATION_FAILED, notPem.code());
        TlsOptions pkcs1 = TlsOptions.trusting(serverIdentity.certificatePem)
                .withClientCertificate(clientIdentity.certificatePem, "-----BEGIN RSA PRIVATE KEY-----\nsecret-key-material\n-----END RSA PRIVATE KEY-----\n");
        SagatException refused = assertThrows(SagatException.class, hostFor(oneWay, pkcs1, null)::start);
        assertEquals(ErrorCode.VALIDATION_FAILED, refused.code());
        assertTrue(refused.getMessage().contains("PKCS#8") || refused.getMessage().contains("TLS"), refused.getMessage());
        assertTrue(!refused.getMessage().contains("secret-key-material"));
        TlsOptions garbage = TlsOptions.trusting(serverIdentity.certificatePem)
                .withClientCertificate(clientIdentity.certificatePem, "-----BEGIN PRIVATE KEY-----\nsecret-key-material\n-----END PRIVATE KEY-----\n");
        SagatException bad = assertThrows(SagatException.class, hostFor(oneWay, garbage, null)::start);
        assertEquals(ErrorCode.VALIDATION_FAILED, bad.code());
        assertTrue(!bad.getMessage().contains("secret-key-material"));
    }

    @Test
    void httpsLogsNoPlainTextWarning() {
        CapturingLogger log = new CapturingLogger();
        registersAndStops(hostFor(oneWay, TlsOptions.trusting(serverIdentity.certificatePem), log));
        assertEquals(0, log.warnings("plain text").size());
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("tok")), "no line holds the token");
    }
}
