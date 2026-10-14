package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.SagatSdk;
import dev.sagatoctopusorchestrator.sdk.TlsOptions;
import dev.sagatoctopusorchestrator.sdk.TokenProvider;
import io.grpc.ClientInterceptor;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.function.Consumer;

/** The settings of a {@link WorkerHost}. Immutable; build it with {@link #builder()}. */
public final class WorkerConfig {
    private final String grpcUrl;
    private final String namespace;
    private final TokenProvider tokenProvider;
    private final String workerName;
    private final int maxInflight;
    private final WorkerTransport transport;
    private final List<String> languages;
    private final List<String> pools;
    private final TlsOptions tls;
    private final Duration timeout;
    private final RetryPolicy retry;
    private final String protocolVersion;
    private final Duration heartbeatInterval;
    private final System.Logger logger;
    private final List<ClientInterceptor> interceptors;
    private final Consumer<WorkerEvent> onEvent;
    private final Duration maxServerWait;

    private WorkerConfig(Builder b) {
        this.grpcUrl = b.grpcUrl;
        this.namespace = b.namespace;
        this.tokenProvider = b.tokenProvider;
        this.workerName = b.workerName;
        this.maxInflight = b.maxInflight;
        this.transport = b.transport;
        this.languages = List.copyOf(b.languages);
        this.pools = List.copyOf(b.pools);
        this.tls = b.tls;
        this.timeout = b.timeout;
        this.retry = b.retry;
        this.protocolVersion = b.protocolVersion;
        this.heartbeatInterval = b.heartbeatInterval;
        this.logger = b.logger;
        this.interceptors = List.copyOf(b.interceptors);
        this.onEvent = b.onEvent;
        this.maxServerWait = b.maxServerWait;
    }

    public static Builder builder() {
        return new Builder();
    }

    /** {@code http://host:port} or {@code https://host:port} of the worker service. */
    public String grpcUrl() {
        return grpcUrl;
    }

    public String namespace() {
        return namespace;
    }

    public TokenProvider tokenProvider() {
        return tokenProvider;
    }

    /** One live session per name and identity: registering again under the same name replaces it. */
    public String workerName() {
        return workerName;
    }

    /** Tasks in flight at once, 1 to 1000; 10 by default. */
    public int maxInflight() {
        return maxInflight;
    }

    public WorkerTransport transport() { return transport; }

    /** The {@code languages} capability; {@code java} by default. */
    public List<String> languages() {
        return languages;
    }

    public List<String> pools() {
        return pools;
    }

    public TlsOptions tls() {
        return tls;
    }

    /** Deadline of one unary RPC; 30 s by default. */
    public Duration timeout() {
        return timeout;
    }

    public RetryPolicy retry() {
        return retry;
    }

    /** The protocol minor the host offers; the SDK's own when null. */
    public String protocolVersion() {
        return protocolVersion;
    }

    /** Overrides the heartbeat interval; null for a third of the lease, 500 ms to 10 s. */
    public Duration heartbeatInterval() {
        return heartbeatInterval;
    }

    public System.Logger logger() {
        return logger;
    }

    /** Interceptors of the gRPC channel: the hook of tests and of the conformance suite. */
    public List<ClientInterceptor> interceptors() {
        return interceptors;
    }

    /** Called for each thing that happens in the host; null for none. It must not block. */
    public Consumer<WorkerEvent> onEvent() {
        return onEvent;
    }

    /** Never shows the token: only where the host connects and what it is called. */
    @Override
    public String toString() {
        return "WorkerConfig[grpcUrl=" + dev.sagatoctopusorchestrator.sdk.Endpoints.hostOf(grpcUrl) + ", namespace=" + namespace + ", workerName=" + workerName + "]";
    }

    /** The longest the host waits because the server asked it to ({@code retry_after_ms}); 60 s by default. */
    public Duration maxServerWait() {
        return maxServerWait;
    }

    /** Builds a {@link WorkerConfig}. */
    public static final class Builder {
        private String grpcUrl;
        private String namespace;
        private TokenProvider tokenProvider;
        private String workerName;
        private int maxInflight = 10;
        private WorkerTransport transport = WorkerTransport.AUTO;
        private List<String> languages = List.of(SagatSdk.LANGUAGE);
        private List<String> pools = List.of();
        private TlsOptions tls;
        private Duration timeout = Duration.ofSeconds(30);
        private RetryPolicy retry = RetryPolicy.DEFAULT;
        private String protocolVersion;
        private Duration heartbeatInterval;
        private System.Logger logger = System.getLogger("dev.sagatoctopusorchestrator.sdk.worker");
        private final List<ClientInterceptor> interceptors = new ArrayList<>();
        private Consumer<WorkerEvent> onEvent;
        private Duration maxServerWait = Duration.ofSeconds(60);

        private Builder() {}

        public Builder grpcUrl(String value) {
            this.grpcUrl = value;
            return this;
        }

        public Builder namespace(String value) {
            this.namespace = value;
            return this;
        }

        public Builder token(String value) {
            this.tokenProvider = TokenProvider.of(value);
            return this;
        }

        public Builder tokenProvider(TokenProvider value) {
            this.tokenProvider = value;
            return this;
        }

        public Builder workerName(String value) {
            this.workerName = value;
            return this;
        }

        public Builder maxInflight(int value) {
            this.maxInflight = value;
            return this;
        }

        /** Selects automatic, polling, or required streaming task reception. */
        public Builder transport(WorkerTransport value) {
            this.transport = Objects.requireNonNull(value);
            return this;
        }

        public Builder languages(List<String> value) {
            this.languages = List.copyOf(value);
            return this;
        }

        public Builder pools(List<String> value) {
            this.pools = List.copyOf(value);
            return this;
        }

        public Builder tls(TlsOptions value) {
            this.tls = value;
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

        /** Overrides the newest protocol minor the host offers (tests use it to play an older or a newer SDK). */
        public Builder protocolVersion(String value) {
            this.protocolVersion = value;
            return this;
        }

        public Builder heartbeatInterval(Duration value) {
            this.heartbeatInterval = value;
            return this;
        }

        public Builder logger(System.Logger value) {
            this.logger = value;
            return this;
        }

        /** Adds an interceptor to the gRPC channel. */
        public Builder interceptor(ClientInterceptor value) {
            this.interceptors.add(Objects.requireNonNull(value));
            return this;
        }

        public Builder onEvent(Consumer<WorkerEvent> value) {
            this.onEvent = value;
            return this;
        }

        /**
         * The longest the host waits because the server asked it to (a {@code retry_after_ms} of a day is a minute); 60 s
         * by default (SPEC section 6.2). Lower it for a short-lived tool.
         */
        public Builder maxServerWait(Duration value) {
            this.maxServerWait = value;
            return this;
        }

        public WorkerConfig build() {
            if (grpcUrl == null || grpcUrl.isBlank()) {
                throw SagatException.invalid("grpcUrl is required");
            }
            try {
                java.net.URI uri = java.net.URI.create(grpcUrl);
                boolean web = "http".equalsIgnoreCase(uri.getScheme()) || "https".equalsIgnoreCase(uri.getScheme());
                if (!web || uri.getHost() == null || uri.getPort() < 0) {
                    throw SagatException.invalid("grpcUrl is http://host:port or https://host:port");
                }
            } catch (IllegalArgumentException e) {
                throw SagatException.invalid("grpcUrl is not a URL");
            }
            if (namespace == null || namespace.isBlank()) {
                throw SagatException.invalid("namespace is required");
            }
            if (tokenProvider == null) {
                throw SagatException.invalid("a token or a token provider is required");
            }
            if (workerName == null || workerName.isBlank()) {
                throw SagatException.invalid("workerName is required");
            }
            if (maxInflight < 1 || maxInflight > 1000) {
                throw SagatException.invalid("maxInflight is between 1 and 1000");
            }
            if (languages.isEmpty()) {
                throw SagatException.invalid("languages is not empty");
            }
            Objects.requireNonNull(timeout, "timeout");
            Objects.requireNonNull(retry, "retry");
            Objects.requireNonNull(logger, "logger");
            if (heartbeatInterval != null && heartbeatInterval.toMillis() < 1) {
                // Clamped to a millisecond it would be a hot loop of heartbeats; a host that wants the default sets none.
                throw SagatException.invalid("heartbeatInterval must be at least 1 ms");
            }
            if (maxServerWait == null || maxServerWait.toMillis() < 1) {
                throw SagatException.invalid("maxServerWait must be at least 1 ms");
            }
            if (timeout.isZero() || timeout.isNegative()) {
                throw SagatException.invalid("timeout must be positive");
            }
            return new WorkerConfig(this);
        }
    }
}
