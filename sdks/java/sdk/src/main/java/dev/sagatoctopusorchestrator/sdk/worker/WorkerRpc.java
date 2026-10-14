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
import io.grpc.Context;
import io.grpc.ChannelCredentials;
import io.grpc.Grpc;
import io.grpc.InsecureChannelCredentials;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;
import io.grpc.Metadata;
import io.grpc.StatusRuntimeException;
import io.grpc.stub.MetadataUtils;
import io.grpc.stub.StreamObserver;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.HexFormat;
import java.util.Iterator;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;

/** The typed, authenticated, error-mapped worker RPCs. One instance owns one channel, opened when it is created. */
final class WorkerRpc implements AutoCloseable {
    private static final Metadata.Key<String> AUTHORIZATION = Metadata.Key.of("authorization", Metadata.ASCII_STRING_MARSHALLER);
    private static final Metadata.Key<String> TRACEPARENT = Metadata.Key.of("traceparent", Metadata.ASCII_STRING_MARSHALLER);

    private final WorkerConfig config;
    private final ManagedChannel channel;
    private final WorkerServiceGrpc.WorkerServiceBlockingStub stub;
    private final WorkerServiceGrpc.WorkerServiceStub asyncStub;

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
            throw SagatException.of(ErrorCode.VALIDATION_FAILED, "the channel to the worker service cannot be opened (" + e.getClass().getSimpleName() + ")");
        }
        if (!config.interceptors().isEmpty()) {
            builder.intercept(config.interceptors());
        }
        this.channel = builder.build();
        this.stub = WorkerServiceGrpc.newBlockingStub(channel);
        this.asyncStub = WorkerServiceGrpc.newStub(channel);
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

    record TaskStream(Iterator<Worker.PollTasksResponse> frames, Context.CancellableContext context) implements AutoCloseable {
        @Override public void close() { context.cancel(null); }
    }

    TaskStream streamTasks(Worker.PollTasksRequest request) {
        Metadata metadata = new Metadata();
        metadata.put(AUTHORIZATION, "Bearer " + TokenProvider.resolve(config.tokenProvider()));
        long timeout = Math.min(config.timeout().toMillis(), 6_000L);
        Context.CancellableContext context = Context.current().withCancellation();
        try {
            Iterator<Worker.PollTasksResponse> source = context.call(() ->
                    stub.withInterceptors(MetadataUtils.newAttachHeadersInterceptor(metadata))
                            .withDeadlineAfter(timeout, TimeUnit.MILLISECONDS)
                            .streamTasks(request));
            Iterator<Worker.PollTasksResponse> frames = new Iterator<>() {
                @Override public boolean hasNext() {
                    try { return source.hasNext(); } catch (StatusRuntimeException e) { throw streamError(e); }
                }
                @Override public Worker.PollTasksResponse next() {
                    try { return source.next(); } catch (StatusRuntimeException e) { throw streamError(e); }
                }
            };
            return new TaskStream(frames, context);
        } catch (StatusRuntimeException e) {
            context.cancel(e);
            if (e.getStatus().getCode() == io.grpc.Status.Code.UNIMPLEMENTED) throw new StreamTasksUnimplemented();
            throw GrpcErrors.of("StreamTasks", e);
        } catch (Exception e) {
            context.cancel(e);
            throw SagatException.of(ErrorCode.INTERNAL, "the task stream could not be opened");
        }
    }

    private static RuntimeException streamError(StatusRuntimeException error) {
        if (error.getStatus().getCode() == io.grpc.Status.Code.UNIMPLEMENTED) return new StreamTasksUnimplemented();
        return GrpcErrors.of("StreamTasks", error);
    }

    static final class StreamTasksUnimplemented extends RuntimeException {}

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

    record WrittenBlob(Worker.WriteBlobResponse response, long size, String sha256) {}

    WrittenBlob writeBlob(Worker.WriteBlobHeader header, InputStream source, String traceparent, CancellationSignal cancellation) {
        Metadata metadata = metadata(traceparent);
        var prepared = asyncStub.withInterceptors(MetadataUtils.newAttachHeadersInterceptor(metadata))
                .withDeadlineAfter(config.timeout().toMillis(), TimeUnit.MILLISECONDS);
        AtomicReference<Worker.WriteBlobResponse> response = new AtomicReference<>();
        AtomicReference<Throwable> failure = new AtomicReference<>();
        CountDownLatch done = new CountDownLatch(1);
        StreamObserver<Worker.WriteBlobRequest> requests = prepared.writeBlob(new StreamObserver<>() {
            @Override public void onNext(Worker.WriteBlobResponse value) { response.set(value); }
            @Override public void onError(Throwable error) { failure.set(error); done.countDown(); }
            @Override public void onCompleted() { done.countDown(); }
        });
        MessageDigest digest;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException(impossible);
        }
        long size = 0;
        try {
            requests.onNext(Worker.WriteBlobRequest.newBuilder().setHeader(header).build());
            byte[] buffer = new byte[256 * 1024];
            while (true) {
                if (cancellation.isCancelled()) {
                    throw SagatException.of(ErrorCode.CANCELLED, "the blob upload was cancelled");
                }
                int read = source.read(buffer);
                if (read < 0) {
                    break;
                }
                if (read == 0) {
                    throw SagatException.of(ErrorCode.PROTOCOL, "the blob source made no progress");
                }
                size += read;
                digest.update(buffer, 0, read);
                requests.onNext(Worker.WriteBlobRequest.newBuilder()
                        .setChunk(com.google.protobuf.ByteString.copyFrom(buffer, 0, read)).build());
            }
            requests.onCompleted();
            if (!done.await(config.timeout().toMillis() + 1000, TimeUnit.MILLISECONDS)) {
                throw SagatException.of(ErrorCode.TRANSPORT, "no answer from the worker service");
            }
        } catch (IOException e) {
            requests.onError(e);
            throw SagatException.of(ErrorCode.PROTOCOL, "the blob source failed before EOF");
        } catch (InterruptedException e) {
            requests.onError(e);
            Thread.currentThread().interrupt();
            throw SagatException.of(ErrorCode.CANCELLED, "the blob upload was cancelled");
        } catch (RuntimeException e) {
            requests.onError(e);
            throw e;
        }
        if (failure.get() != null) {
            throw GrpcErrors.of("WriteBlob", io.grpc.Status.fromThrowable(failure.get()).asRuntimeException());
        }
        if (response.get() == null) {
            throw SagatException.of(ErrorCode.PROTOCOL, "the blob service returned no descriptor");
        }
        return new WrittenBlob(response.get(), size, HexFormat.of().formatHex(digest.digest()));
    }

    record ReadBlob(Iterator<Worker.ReadBlobResponse> parts, Context.CancellableContext context) {
        void cancel() { context.cancel(null); }
    }

    ReadBlob readBlob(Worker.ReadBlobRequest request, String traceparent) {
        Context.CancellableContext context = Context.current().withCancellation();
        try {
            Iterator<Worker.ReadBlobResponse> parts = context.call(() ->
                    stub.withInterceptors(MetadataUtils.newAttachHeadersInterceptor(metadata(traceparent)))
                            .withDeadlineAfter(config.timeout().toMillis(), TimeUnit.MILLISECONDS)
                            .readBlob(request));
            return new ReadBlob(parts, context);
        } catch (StatusRuntimeException e) {
            context.cancel(e);
            throw GrpcErrors.of("ReadBlob", e);
        } catch (Exception e) {
            context.cancel(e);
            throw SagatException.of(ErrorCode.INTERNAL, "the blob stream could not be opened");
        }
    }

    private Metadata metadata(String traceparent) {
        Metadata metadata = new Metadata();
        metadata.put(AUTHORIZATION, "Bearer " + TokenProvider.resolve(config.tokenProvider()));
        if (traceparent != null && !traceparent.isEmpty()) {
            metadata.put(TRACEPARENT, Traceparents.check(traceparent));
        }
        return metadata;
    }
}
