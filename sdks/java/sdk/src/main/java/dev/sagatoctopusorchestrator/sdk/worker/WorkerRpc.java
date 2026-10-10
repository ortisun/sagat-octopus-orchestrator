package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.protocol.v1.WorkerServiceGrpc;
import dev.sagatoctopusorchestrator.sdk.Endpoints;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.TlsOptions;
import dev.sagatoctopusorchestrator.sdk.TokenProvider;
import dev.sagatoctopusorchestrator.sdk.Traceparents;
import io.grpc.Channel;
import io.grpc.ChannelCredentials;
import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;
import io.grpc.Metadata;
import io.grpc.StatusRuntimeException;
import io.grpc.stub.MetadataUtils;
import java.net.URI;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;

/** The typed, authenticated, error-mapped worker RPCs. One instance owns one channel, opened when it is created. */
final class WorkerRpc implements AutoCloseable {
    private static final Metadata.Key<String> AUTHORIZATION = Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER);
    private static final Metadata.Key<String> TRACEPARENT = Metadata.Key.of("traceparent", Metadata.ASCII_STRING_MARSHALLER);

    private final WorkerConfig config;
    private final ManagedChannel channel;
    private final WorkerServiceGrpc.WorkerServiceBlockingStub stub;

    WorkerRpc(WorkerConfig config) {
        this.config = config;
        URI url = URI.create(config.grpcUrl());
        boolean secure = "https".equalsIgnoreCase(url.getScheme());
        if (!secure && !"http".equalsIgnoreCase(url.getScheme())) {
            throw SagatException.invalid("grpcUrl is http://host:port or https://host:port");
        }
        if (url.getHost() == null || url.getPort() < 0) {
            throw SagatException.invalid("grpcUrl needs a host and a port");
        }
        Endpoints.warnIfPlainText(config.logger(), config.grpcUrl(), "grpcUrl");
        ChannelCredentials credentials;
        if (secure) {
            TlsOptions tls = config.tls() == null ? TlsOptions.systemRoots() : config.tls();
            // Parsed once here as the REST client does, so that a malformed PEM is the same VALIDATION_FAILED on both
            // channels (and never the raw failure of the transport library).
            tls.sslContext();
            credentials = tls.channelCredentials();
        } else {
            credentials = InsecureChannelCredentials.create();
        }
        ManagedChannelBuilder<?> builder;
        try {
            builder = Grpc.newChannelBuilderForAddress(url.getHost(), url.getPort(), credentials)
                    .maxInboundMessageSize(16 * 1024 * 1024)
                    .keepAliveTime(120, TimeUnit.SECONDS);
        } catch (RuntimeException e) {
            throw SagatException.builder(ErrorCode.VALIDATION_FAILED, "the channel to the worker service cannot be opened (" + e.getClass().getSimpleName() + ")")
                    .cause(e)
                    .build();
        }
        if (!config.interceptors().isEmpty()) {
            builder.intercept(config.interceptors());
        }
        this.channel = builder.build();
        this.stub = WorkerServiceGrpc.newBlockingStub(channel);
    }

    private volatile boolean closed;

    boolean isClosed() {
        return closed;
    }

    @Override
    public void close() {
        closed = true;
        channel.shutdownNow();
        try {
            channel.awaitTermination(2, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    /** Cancels what is in flight on the channel without waiting. */
    void abort() {
        closed = true;
        channel.shutdownNow();
    }

    Channel channel() {
        return channel;
    }

    private <R> R call(String method, String traceparent, Function<WorkerServiceGrpc.WorkerServiceBlockingStub, R> invoke) {
        Metadata metadata = new Metadata();
        metadata.put(AUTHORIZATION, "Bearer " + TokenProvider.resolve(config.tokenProvider()));
        if (traceparent != null && !traceparent.isEmpty()) {
            metadata.put(TRACEPARENT, Traceparents.check(traceparent));
        }
        try {
            return invoke.apply(stub.withInterceptors(MetadataUtils.newAttachHeadersInterceptor(metadata))
                    .withDeadlineAfter(config.timeout().toMillis(), TimeUnit.MILLISECONDS));
        } catch (StatusRuntimeException e) {
            throw GrpcErrors.of(method, e);
        }
    }

    Worker.RegisterWorkerResponse registerWorker(Worker.RegisterWorkerRequest request) {
        return call("RegisterWorker", null, s -> s.registerWorker(request));
    }

    Worker.PollTasksResponse pollTasks(Worker.PollTasksRequest request) {
        return call("PollTasks", null, s -> s.pollTasks(request));
    }

    Worker.HeartbeatResponse heartbeat(Worker.HeartbeatRequest request, String traceparent) {
        return call("Heartbeat", traceparent, s -> s.heartbeat(request));
    }

    Worker.StepCommandResponse completeStep(Worker.CompleteStepRequest request, String traceparent) {
        return call("CompleteStep", traceparent, s -> s.completeStep(request));
    }

    Worker.StepCommandResponse failStep(Worker.FailStepRequest request, String traceparent) {
        return call("FailStep", traceparent, s -> s.failStep(request));
    }

    Worker.ResolveSecretsResponse resolveSecrets(Worker.ResolveSecretsRequest request, String traceparent) {
        return call("ResolveSecrets", traceparent, s -> s.resolveSecrets(request));
    }
}
