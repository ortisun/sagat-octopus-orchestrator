package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.util.Objects;
import java.util.concurrent.ExecutorService;
import java.util.function.Supplier;

/** The settings of a {@link SagatClient} (sdks/SPEC.md section 1). Immutable; build it with {@link #builder()}. */
public final class ClientConfig {
    private final String apiUrl;
    private final String namespace;
    private final TokenProvider tokenProvider;
    private final Duration timeout;
    private final RetryPolicy retry;
    private final String userAgent;
    private final TlsOptions tls;
    private final HttpTransport transport;
    private final Supplier<String> traceContext;
    private final System.Logger logger;
    private final ExecutorService asyncExecutor;
    private final Duration maxServerWait;

    private ClientConfig(Builder b) {
        this.apiUrl = b.apiUrl;
        this.namespace = b.namespace;
        this.tokenProvider = b.tokenProvider;
        this.timeout = b.timeout;
        this.retry = b.retry;
        this.userAgent = b.userAgent;
        this.tls = b.tls;
        this.transport = b.transport;
        this.traceContext = b.traceContext;
        this.logger = b.logger;
        this.asyncExecutor = b.asyncExecutor;
        this.maxServerWait = b.maxServerWait;
    }

    public static Builder builder() {
        return new Builder();
    }

    /** Base URL of the REST API including {@code /v1}. */
    public String apiUrl() {
        return apiUrl;
    }

    /** Slug of the namespace every call is made in. */
    public String namespace() {
        return namespace;
    }

    /** Called before every request. */
    public TokenProvider tokenProvider() {
        return tokenProvider;
    }

    /** Deadline of one REST call; 30 s by default. */
    public Duration timeout() {
        return timeout;
    }

    public RetryPolicy retry() {
        return retry;
    }

    /** Appended to {@code sagat-sdk-java/<version>}; may be null. */
    public String userAgent() {
        return userAgent;
    }

    /** TLS settings; null for the JVM's defaults. */
    public TlsOptions tls() {
        return tls;
    }

    /** The REST transport; null for the default. */
    public HttpTransport transport() {
        return transport;
    }

    /** The ambient trace context, called when a call has no explicit {@code traceparent}; may be null. */
    public Supplier<String> traceContext() {
        return traceContext;
    }

    public System.Logger logger() {
        return logger;
    }

    /** The executor that runs the {@code ...Async} calls; null for one the client owns. */
    public ExecutorService asyncExecutor() {
        return asyncExecutor;
    }

    /** Never shows the token: only where the client talks to. */
    @Override
    public String toString() {
        return "ClientConfig[apiUrl=" + Endpoints.hostOf(apiUrl) + ", namespace=" + namespace + "]";
    }

    /** The longest the SDK waits because the server asked it to ({@code Retry-After}); 60 s by default. */
    public Duration maxServerWait() {
        return maxServerWait;
    }

    /** Builds a {@link ClientConfig}. */
    public static final class Builder {
        private String apiUrl;
        private String namespace;
        private TokenProvider tokenProvider;
        private Duration timeout = Duration.ofSeconds(30);
        private RetryPolicy retry = RetryPolicy.DEFAULT;
        private String userAgent;
        private TlsOptions tls;
        private HttpTransport transport;
        private Supplier<String> traceContext;
        private System.Logger logger = System.getLogger("dev.sagatoctopusorchestrator.sdk");
        private ExecutorService asyncExecutor;
        private Duration maxServerWait = Duration.ofSeconds(60);

        private Builder() {}

        /** Base URL of the REST API including {@code /v1}, e.g. {@code https://api.example/v1}. Required. */
        public Builder apiUrl(String value) {
            this.apiUrl = value;
            return this;
        }

        /** Slug of the namespace every call is made in. Required. The tenant is the token's, never configured. */
        public Builder namespace(String value) {
            this.namespace = value;
            return this;
        }

        /** A static bearer token. */
        public Builder token(String value) {
            this.tokenProvider = TokenProvider.of(value);
            return this;
        }

        /** A provider called before every request that returns a fresh token. */
        public Builder tokenProvider(TokenProvider value) {
            this.tokenProvider = value;
            return this;
        }

        public Builder timeout(Duration value) {
            this.timeout = value;
            return this;
        }

        public Builder retry(RetryPolicy value) {
            this.retry = value;
            return this;
        }

        public Builder userAgent(String value) {
            this.userAgent = value;
            return this;
        }

        public Builder tls(TlsOptions value) {
            this.tls = value;
            return this;
        }

        /**
         * Replaces the REST transport (the hook of tests and of the conformance suite). It must not follow redirects;
         * see {@link HttpTransport} and {@link JdkHttpTransport#using(java.net.http.HttpClient)}.
         */
        public Builder transport(HttpTransport value) {
            this.transport = value;
            return this;
        }

        /** The ambient trace context (for example from OpenTelemetry), a W3C {@code traceparent} or null. */
        public Builder traceContext(Supplier<String> value) {
            this.traceContext = value;
            return this;
        }

        /** Where the SDK logs (never a payload, a token or a secret). */
        public Builder logger(System.Logger value) {
            this.logger = value;
            return this;
        }

        /** The executor of the {@code ...Async} calls; the caller shuts it down. */
        public Builder asyncExecutor(ExecutorService value) {
            this.asyncExecutor = value;
            return this;
        }

        private static void requireUrl(String url, String what) {
            try {
                java.net.URI uri = java.net.URI.create(url);
                boolean web = "http".equalsIgnoreCase(uri.getScheme()) || "https".equalsIgnoreCase(uri.getScheme());
                if (!web || uri.getHost() == null) {
                    throw SagatException.invalid(what + " is http://host/... or https://host/...");
                }
            } catch (IllegalArgumentException e) {
                throw SagatException.invalid(what + " is not a URL");
            }
        }

        /**
         * The longest the SDK waits because the server asked it to (a {@code Retry-After} of an hour is a minute); 60 s
         * by default (SPEC section 6.2). Lower it for a short-lived tool.
         */
        public Builder maxServerWait(Duration value) {
            this.maxServerWait = value;
            return this;
        }

        public ClientConfig build() {
            if (apiUrl == null || apiUrl.isBlank()) {
                throw SagatException.invalid("apiUrl is required");
            }
            requireUrl(apiUrl, "apiUrl");
            if (namespace == null || namespace.isBlank()) {
                throw SagatException.invalid("namespace is required");
            }
            if (tokenProvider == null) {
                throw SagatException.invalid("a token or a token provider is required");
            }
            Objects.requireNonNull(timeout, "timeout");
            Objects.requireNonNull(retry, "retry");
            Objects.requireNonNull(logger, "logger");
            if (timeout.isZero() || timeout.isNegative()) {
                throw SagatException.invalid("timeout must be positive");
            }
            if (maxServerWait == null || maxServerWait.toMillis() < 1) {
                throw SagatException.invalid("maxServerWait must be at least 1 ms");
            }
            return new ClientConfig(this);
        }
    }
}
