package dev.sagatoctopusorchestrator.sdk;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.GeneralSecurityException;
import java.security.KeyFactory;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.SecureRandom;
import java.security.cert.Certificate;
import java.security.cert.CertificateFactory;
import java.security.spec.PKCS8EncodedKeySpec;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collection;
import java.util.List;
import java.util.Objects;
import javax.net.ssl.KeyManager;
import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;

/**
 * TLS settings (SPEC section 1): the trust roots, and a client certificate for mTLS. Everything is PEM; the private
 * key is PKCS#8 ({@code -----BEGIN PRIVATE KEY-----}). Absent trust roots mean the JVM's default roots.
 */
public final class TlsOptions {
    private final byte[] trustRootsPem;
    private final byte[] clientCertificatePem;
    private final byte[] clientKeyPem;

    private TlsOptions(byte[] trustRootsPem, byte[] clientCertificatePem, byte[] clientKeyPem) {
        this.trustRootsPem = trustRootsPem;
        this.clientCertificatePem = clientCertificatePem;
        this.clientKeyPem = clientKeyPem;
    }

    /** Only the JVM's default trust roots. */
    public static TlsOptions systemRoots() {
        return new TlsOptions(null, null, null);
    }

    /** Trusts exactly the certificates in the PEM text. */
    public static TlsOptions trusting(String pem) {
        return new TlsOptions(pem.getBytes(StandardCharsets.UTF_8), null, null);
    }

    /** Trusts exactly the certificates in the PEM file. */
    public static TlsOptions trusting(Path pemFile) throws IOException {
        return new TlsOptions(Files.readAllBytes(pemFile), null, null);
    }

    /** The same trust roots, plus a client certificate chain and its PKCS#8 private key, both PEM. */
    public TlsOptions withClientCertificate(String certificatePem, String pkcs8KeyPem) {
        Objects.requireNonNull(certificatePem, "certificatePem");
        Objects.requireNonNull(pkcs8KeyPem, "pkcs8KeyPem");
        return new TlsOptions(trustRootsPem, certificatePem.getBytes(StandardCharsets.UTF_8), pkcs8KeyPem.getBytes(StandardCharsets.UTF_8));
    }

    /** The same trust roots, plus a client certificate chain and its PKCS#8 private key, both PEM files. */
    public TlsOptions withClientCertificate(Path certificateFile, Path pkcs8KeyFile) throws IOException {
        return new TlsOptions(trustRootsPem, Files.readAllBytes(certificateFile), Files.readAllBytes(pkcs8KeyFile));
    }

    /** The credentials of the gRPC channel (https): the trust roots and, for mTLS, the client certificate. */
    public io.grpc.ChannelCredentials channelCredentials() {
        try {
            io.grpc.TlsChannelCredentials.Builder builder = io.grpc.TlsChannelCredentials.newBuilder();
            if (trustRootsPem != null) {
                builder.trustManager(new ByteArrayInputStream(trustRootsPem));
            }
            if (clientCertificatePem != null) {
                builder.keyManager(new ByteArrayInputStream(clientCertificatePem), new ByteArrayInputStream(clientKeyPem));
            }
            return builder.build();
        } catch (IOException e) {
            throw SagatException.builder(ErrorCode.VALIDATION_FAILED, "the TLS settings cannot be used (" + e.getClass().getSimpleName() + ")")
                    .cause(e)
                    .build();
        }
    }

    /** The {@link SSLContext} for the REST client. */
    public SSLContext sslContext() {
        try {
            TrustManager[] trust = null;
            if (trustRootsPem != null) {
                KeyStore roots = KeyStore.getInstance("PKCS12");
                roots.load(null, null);
                int i = 0;
                for (Certificate certificate : certificates(trustRootsPem)) {
                    roots.setCertificateEntry("root-" + i++, certificate);
                }
                if (i == 0) {
                    throw SagatException.invalid("the trust roots hold no certificate");
                }
                TrustManagerFactory factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
                factory.init(roots);
                trust = factory.getTrustManagers();
            }
            KeyManager[] keys = null;
            if (clientCertificatePem != null) {
                List<Certificate> chain = new ArrayList<>(certificates(clientCertificatePem));
                if (chain.isEmpty()) {
                    throw SagatException.invalid("the client certificate holds no certificate");
                }
                PrivateKey key = privateKey(clientKeyPem);
                char[] password = new char[24];
                byte[] random = new byte[password.length];
                new SecureRandom().nextBytes(random);
                for (int i = 0; i < password.length; i++) {
                    password[i] = (char) ('a' + (random[i] & 0x0f));
                }
                KeyStore store = KeyStore.getInstance("PKCS12");
                store.load(null, null);
                store.setKeyEntry("client", key, password, chain.toArray(new Certificate[0]));
                KeyManagerFactory factory = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
                factory.init(store, password);
                keys = factory.getKeyManagers();
            }
            SSLContext context = SSLContext.getInstance("TLS");
            context.init(keys, trust, null);
            return context;
        } catch (GeneralSecurityException | IOException e) {
            throw SagatException.builder(ErrorCode.VALIDATION_FAILED, "the TLS settings cannot be used (" + e.getClass().getSimpleName() + ")")
                    .cause(e)
                    .build();
        }
    }

    private static Collection<? extends Certificate> certificates(byte[] pem) throws GeneralSecurityException {
        return CertificateFactory.getInstance("X.509").generateCertificates(new ByteArrayInputStream(pem));
    }

    private static PrivateKey privateKey(byte[] pem) throws GeneralSecurityException {
        String text = new String(pem, StandardCharsets.UTF_8);
        if (text.contains("BEGIN RSA PRIVATE KEY") || text.contains("BEGIN EC PRIVATE KEY")) {
            throw SagatException.invalid("the client key must be PKCS#8 (BEGIN PRIVATE KEY); convert it with openssl pkcs8 -topk8 -nocrypt");
        }
        String base64 = text.replaceAll("-----(BEGIN|END) PRIVATE KEY-----", "").replaceAll("\\s", "");
        byte[] der;
        try {
            der = Base64.getDecoder().decode(base64);
        } catch (IllegalArgumentException e) {
            throw SagatException.invalid("the client key is not PEM");
        }
        PKCS8EncodedKeySpec spec = new PKCS8EncodedKeySpec(der);
        GeneralSecurityException last = null;
        for (String algorithm : new String[] {"RSA", "EC", "Ed25519"}) {
            try {
                return KeyFactory.getInstance(algorithm).generatePrivate(spec);
            } catch (GeneralSecurityException e) {
                last = e;
            }
        }
        throw last;
    }
}
