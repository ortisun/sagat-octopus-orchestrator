package dev.sagatoctopusorchestrator.sdk;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.cert.Certificate;
import java.util.Base64;
import java.util.concurrent.TimeUnit;

/**
 * A throw-away self-signed identity for the TLS tests, made by the JDK's own {@code keytool} each time the tests run, so
 * that no private key is kept in the repository. By default the certificate is valid for 127.0.0.1 and localhost.
 */
public final class TlsFixture {
    public final String certificatePem;
    public final String pkcs8KeyPem;

    private TlsFixture(String certificatePem, String pkcs8KeyPem) {
        this.certificatePem = certificatePem;
        this.pkcs8KeyPem = pkcs8KeyPem;
    }

    /** The certificate, parsed. */
    public java.security.cert.X509Certificate certificate() throws java.security.GeneralSecurityException {
        return (java.security.cert.X509Certificate) java.security.cert.CertificateFactory.getInstance("X.509")
                .generateCertificate(new java.io.ByteArrayInputStream(certificatePem.getBytes(StandardCharsets.UTF_8)));
    }

    public static TlsFixture create(String name) throws Exception {
        return create(name, "ip:127.0.0.1,dns:localhost");
    }

    /** An identity whose certificate names only the hosts in {@code san} (keytool syntax, e.g. {@code dns:other.example}). */
    public static TlsFixture create(String name, String san) throws Exception {
        Path dir = Files.createTempDirectory("sagat-tls-" + name);
        Path store = dir.resolve("store.p12");
        String keytool = Path.of(System.getProperty("java.home"), "bin", "keytool").toString();
        Process process = new ProcessBuilder(
                        keytool, "-genkeypair", "-alias", name, "-keyalg", "EC", "-groupname", "secp256r1",
                        "-dname", "CN=localhost", "-ext", "san=" + san, "-validity", "2",
                        "-keystore", store.toString(), "-storetype", "PKCS12", "-storepass", "changeit", "-keypass", "changeit")
                .redirectErrorStream(true)
                .start();
        try (InputStream out = process.getInputStream()) {
            out.readAllBytes();
        }
        if (!process.waitFor(60, TimeUnit.SECONDS) || process.exitValue() != 0) {
            throw new IOException("keytool failed");
        }
        KeyStore keys = KeyStore.getInstance("PKCS12");
        try (InputStream in = Files.newInputStream(store)) {
            keys.load(in, "changeit".toCharArray());
        }
        PrivateKey key = (PrivateKey) keys.getKey(name, "changeit".toCharArray());
        Certificate certificate = keys.getCertificate(name);
        Files.deleteIfExists(store);
        Files.deleteIfExists(dir);
        return new TlsFixture(pem("CERTIFICATE", certificate.getEncoded()), pem("PRIVATE KEY", key.getEncoded()));
    }

    private static String pem(String type, byte[] der) {
        String body = Base64.getMimeEncoder(64, "\n".getBytes(StandardCharsets.US_ASCII)).encodeToString(der);
        return "-----BEGIN " + type + "-----\n" + body + "\n-----END " + type + "-----\n";
    }
}
