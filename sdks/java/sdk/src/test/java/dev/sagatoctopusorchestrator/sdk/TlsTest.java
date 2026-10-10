package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.sun.net.httpserver.HttpsConfigurator;
import com.sun.net.httpserver.HttpsExchange;
import com.sun.net.httpserver.HttpsParameters;
import com.sun.net.httpserver.HttpsServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.cert.Certificate;
import java.time.Duration;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.regex.Pattern;
import java.util.stream.Stream;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.SSLPeerUnverifiedException;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * TLS and mTLS on the REST client against in-process HTTPS servers, with identities made by keytool at test time
 * (SPEC section 1). The gRPC channel is tested the same way in {@code WorkerTlsTest}.
 */
@Timeout(120)
class TlsTest {
    private static final String RUN = "0190aaaa-0000-7000-8000-000000000001";
    private static final String BODY = "{\"runId\":\"r\",\"workflowId\":\"w\",\"versionId\":\"v\",\"status\":\"RUNNING\",\"createdAt\":\"2026-10-10T10:00:00Z\"}";

    private static TlsFixture serverIdentity;
    private static TlsFixture clientIdentity;
    private static TlsFixture stranger;
    private static TlsFixture elsewhere;
    private static HttpsServer mutual;
    private static HttpsServer oneWay;
    private static HttpsServer wrongHost;
    /** What the mutual TLS server saw of the client, per request. */
    private static final List<List<Certificate>> SEEN_BY_MUTUAL = new CopyOnWriteArrayList<>();
    private static final List<Boolean> SEEN_CLIENT_CERT_BY_ONE_WAY = new CopyOnWriteArrayList<>();

    private static HttpsServer serve(TlsOptions identity, boolean requireClientCertificate, java.util.function.Consumer<HttpsExchange> seen) throws IOException {
        HttpsServer server = HttpsServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.setHttpsConfigurator(new HttpsConfigurator(identity.sslContext()) {
            @Override
            public void configure(HttpsParameters params) {
                SSLParameters parameters = getSSLContext().getDefaultSSLParameters();
                parameters.setNeedClientAuth(requireClientCertificate);
                params.setSSLParameters(parameters);
            }
        });
        server.createContext("/", exchange -> {
            seen.accept((HttpsExchange) exchange);
            byte[] body = BODY.getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, body.length);
            exchange.getResponseBody().write(body);
            exchange.close();
        });
        server.start();
        return server;
    }

    @BeforeAll
    static void start() throws Exception {
        serverIdentity = TlsFixture.create("server");
        clientIdentity = TlsFixture.create("client");
        stranger = TlsFixture.create("stranger");
        elsewhere = TlsFixture.create("elsewhere", "dns:other.example");
        // The server of mutual TLS trusts the client certificate and requires it.
        mutual = serve(TlsOptions.trusting(clientIdentity.certificatePem).withClientCertificate(serverIdentity.certificatePem, serverIdentity.pkcs8KeyPem), true, exchange -> {
            try {
                SEEN_BY_MUTUAL.add(List.of(exchange.getSSLSession().getPeerCertificates()));
            } catch (SSLPeerUnverifiedException e) {
                SEEN_BY_MUTUAL.add(List.of());
            }
        });
        oneWay = serve(TlsOptions.systemRoots().withClientCertificate(serverIdentity.certificatePem, serverIdentity.pkcs8KeyPem), false, exchange -> {
            try {
                exchange.getSSLSession().getPeerCertificates();
                SEEN_CLIENT_CERT_BY_ONE_WAY.add(true);
            } catch (SSLPeerUnverifiedException e) {
                SEEN_CLIENT_CERT_BY_ONE_WAY.add(false);
            }
        });
        // A server whose certificate is trusted by the client but names another host than the one it is reached by.
        wrongHost = serve(TlsOptions.systemRoots().withClientCertificate(elsewhere.certificatePem, elsewhere.pkcs8KeyPem), false, exchange -> {});
    }

    @AfterAll
    static void stop() {
        mutual.stop(0);
        oneWay.stop(0);
        wrongHost.stop(0);
    }

    private static SagatClient clientFor(HttpsServer server, TlsOptions tls, System.Logger logger) {
        ClientConfig.Builder builder = ClientConfig.builder()
                .apiUrl("https://127.0.0.1:" + server.getAddress().getPort() + "/v1")
                .namespace("payments")
                .token("tok")
                .tls(tls)
                .retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1)));
        if (logger != null) {
            builder.logger(logger);
        }
        return new SagatClient(builder.build());
    }

    private static SagatClient clientFor(HttpsServer server, TlsOptions tls) {
        return clientFor(server, tls, null);
    }

    @Test
    void withMutualTlsTheServerSeesTheClientCertificateAndServesIt() throws Exception {
        SEEN_BY_MUTUAL.clear();
        TlsOptions tls = TlsOptions.trusting(serverIdentity.certificatePem).withClientCertificate(clientIdentity.certificatePem, clientIdentity.pkcs8KeyPem);
        assertEquals(RunStatus.RUNNING, clientFor(mutual, tls).get(RUN).status());
        assertEquals(1, SEEN_BY_MUTUAL.size());
        assertEquals(1, SEEN_BY_MUTUAL.get(0).size(), "the server saw one certificate");
        assertArrayEquals(clientIdentity.certificate().getEncoded(), SEEN_BY_MUTUAL.get(0).get(0).getEncoded(), "and it is the client's own");
    }

    @Test
    void aServerThatRequiresAClientCertificateRefusesAClientWithoutOne() {
        SEEN_BY_MUTUAL.clear();
        SagatException error = assertThrows(SagatException.class, () -> clientFor(mutual, TlsOptions.trusting(serverIdentity.certificatePem)).get(RUN));
        assertEquals(ErrorCode.TRANSPORT, error.code());
        assertTrue(SEEN_BY_MUTUAL.isEmpty(), "no request got through the handshake");
    }

    @Test
    void aServerThatDoesNotAskForAClientCertificateIsServedWithTheConfiguredCaAndSeesNone() {
        SEEN_CLIENT_CERT_BY_ONE_WAY.clear();
        assertEquals(RunStatus.RUNNING, clientFor(oneWay, TlsOptions.trusting(serverIdentity.certificatePem)).get(RUN).status());
        assertEquals(List.of(false), SEEN_CLIENT_CERT_BY_ONE_WAY, "without mTLS configured, no client certificate is sent");
    }

    @Test
    void aServerCertificateTheClientDoesNotTrustFailsAsTransportWhateverTheConfiguration() {
        // The CA is configured, but it is not the server's.
        SagatException wrongCa = assertThrows(SagatException.class, () -> clientFor(oneWay, TlsOptions.trusting(stranger.certificatePem)).get(RUN));
        assertEquals(ErrorCode.TRANSPORT, wrongCa.code());
        assertTrue(wrongCa.retryable());
        // No TLS configuration at all: the JVM's roots do not hold a self-signed certificate.
        SagatException defaults = assertThrows(SagatException.class, () -> clientFor(oneWay, null).get(RUN));
        assertEquals(ErrorCode.TRANSPORT, defaults.code());
        SagatException systemRoots = assertThrows(SagatException.class, () -> clientFor(oneWay, TlsOptions.systemRoots()).get(RUN));
        assertEquals(ErrorCode.TRANSPORT, systemRoots.code());
    }

    @Test
    void theHostNameIsVerifiedEvenWhenTheCertificateIsTrusted() {
        // The CA of the server is configured and trusted; its certificate names other.example, not 127.0.0.1.
        SagatException error = assertThrows(SagatException.class, () -> clientFor(wrongHost, TlsOptions.trusting(elsewhere.certificatePem)).get(RUN));
        assertEquals(ErrorCode.TRANSPORT, error.code());
    }

    @Test
    void httpsLogsNoPlainTextWarning() {
        CapturingLogger log = new CapturingLogger();
        clientFor(oneWay, TlsOptions.trusting(serverIdentity.certificatePem), log).get(RUN);
        assertEquals(0, log.warnings("plain text").size());
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("tok")), "no line holds the token");
    }

    @Test
    void badPemIsAValidationErrorThatDoesNotEchoTheKey() {
        SagatException notPem = assertThrows(SagatException.class, () -> TlsOptions.trusting("not a certificate").sslContext());
        assertEquals(ErrorCode.VALIDATION_FAILED, notPem.code());
        TlsOptions rsaStyle = TlsOptions.systemRoots().withClientCertificate(clientIdentity.certificatePem, "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n");
        SagatException pkcs1 = assertThrows(SagatException.class, rsaStyle::sslContext);
        assertTrue(pkcs1.getMessage().contains("PKCS#8"), pkcs1.getMessage());
        TlsOptions garbage = TlsOptions.systemRoots().withClientCertificate(clientIdentity.certificatePem, "-----BEGIN PRIVATE KEY-----\n@@@@\n-----END PRIVATE KEY-----\n");
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, garbage::sslContext).code());
    }

    private static final Pattern FORBIDDEN = Pattern.compile(
            "X509TrustManager|X509ExtendedTrustManager|checkServerTrusted|InsecureTrustManagerFactory|TrustAll|trustAll|HostnameVerifier"
                    + "|setEndpointIdentificationAlgorithm|disableHostnameVerification|allowUnsafeRenegotiation|SSLContext\\.setDefault"
                    + "|HttpsURLConnection\\.setDefault|jdk\\.internal\\.httpclient|com\\.sun\\.net\\.ssl");

    @Test
    void theGuardRecognisesTheWaysOfSwitchingVerificationOff() {
        String[] offenders = {
            "new X509TrustManager() { public void checkServerTrusted(X509Certificate[] c, String t) {} }",
            "TrustManager[] trustAll = ...",
            "HostnameVerifier ok = (host, session) -> true;",
            "System.setProperty(\"jdk.internal.httpclient.disableHostnameVerification\", \"true\");",
            "-Djdk.internal.httpclient.disableHostnameVerification=true",
            "parameters.setEndpointIdentificationAlgorithm(\"\");",
            "InsecureTrustManagerFactory.INSTANCE",
        };
        for (String offender : offenders) {
            assertTrue(FORBIDDEN.matcher(offender).find(), offender);
        }
        assertFalse(FORBIDDEN.matcher("TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())").find(), "the standard factory is what the SDK uses");
    }

    @Test
    void noCodeAndNoScriptOfTheSdkSwitchesCertificateOrHostNameVerificationOff() throws IOException {
        Path module = Paths.get(System.getProperty("basedir", "sdks/java/sdk")).toAbsolutePath();
        Path root = module.getParent();
        List<Path> scanned = new java.util.ArrayList<>();
        for (Path dir : List.of(module.resolve("src/main"), root.resolve("conformance/src/main"), root.resolve("conformance"), root.resolve("scripts"), root.resolve(".mvn"))) {
            if (!Files.exists(dir)) {
                continue;
            }
            try (Stream<Path> files = Files.walk(dir)) {
                files.filter(Files::isRegularFile)
                        .filter(f -> !f.toString().contains("/target/"))
                        .filter(f -> f.toString().endsWith(".java") || f.toString().endsWith(".sh") || f.toString().endsWith(".xml")
                                || f.toString().endsWith(".properties") || f.toString().endsWith(".config"))
                        .forEach(scanned::add);
            }
        }
        scanned.add(root.resolve("pom.xml"));
        scanned.add(module.resolve("pom.xml"));
        assertTrue(scanned.size() > 40, "the guard looked at " + scanned.size() + " files");
        for (Path file : scanned) {
            assertFalse(FORBIDDEN.matcher(Files.readString(file, StandardCharsets.UTF_8)).find(), file + " touches certificate or host name checks");
        }
    }
}
