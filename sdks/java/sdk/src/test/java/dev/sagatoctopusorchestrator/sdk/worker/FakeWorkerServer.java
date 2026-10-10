package dev.sagatoctopusorchestrator.sdk.worker;

import com.google.protobuf.ByteString;
import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.protocol.v1.WorkerServiceGrpc;
import io.grpc.Context;
import io.grpc.Contexts;
import io.grpc.Grpc;
import io.grpc.InsecureServerCredentials;
import io.grpc.Metadata;
import io.grpc.Server;
import io.grpc.ServerCall;
import io.grpc.ServerCallHandler;
import io.grpc.ServerInterceptor;
import io.grpc.ServerInterceptors;
import io.grpc.Status;
import io.grpc.stub.StreamObserver;
import java.io.IOException;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.TimeUnit;
import java.util.function.Function;

/** An in-memory worker service the tests script: queued leases, queued failures, a record of every call. */
final class FakeWorkerServer extends WorkerServiceGrpc.WorkerServiceImplBase implements AutoCloseable {
    /** One call as the service saw it. */
    record Call(String method, Object request, Metadata headers, long nanos) {
        String header(String name) {
            return headers.get(Metadata.Key.of(name, Metadata.ASCII_STRING_MARSHALLER));
        }
    }

    /** A scripted failure. */
    record Failure(Status.Code code, String reason, Long retryAfterMs) {
        static Failure of(Status.Code code) {
            return new Failure(code, null, null);
        }

        static Failure of(Status.Code code, String reason) {
            return new Failure(code, reason, null);
        }
    }

    private static final Context.Key<Metadata> HEADERS = Context.key("headers");

    final List<Call> calls = new CopyOnWriteArrayList<>();
    final Deque<Worker.TaskLease> leases = new ArrayDeque<>();
    /** Per method, failures to answer with before succeeding (consumed in order). */
    private final Map<String, Deque<Failure>> failures = new ConcurrentHashMap<>();
    volatile String acceptedVersion = "1.2";
    volatile boolean cancelRequested;
    volatile long heartbeatExpiresMs = System.currentTimeMillis() + 60_000;
    volatile List<Worker.ResolvedSecret> secrets = List.of();
    /** How long a poll takes to answer, and the wait the answer asks for when it brings no task. */
    volatile long pollDelayMs;
    volatile int pollRetryAfterMs = 50;
    /** A misbehaving service: answers a poll with every task it has, whatever {@code max_tasks} says. */
    volatile boolean ignoreMax;
    private final Server server;

    FakeWorkerServer() throws IOException {
        ServerInterceptor capture = new ServerInterceptor() {
            @Override
            public <Q, R> ServerCall.Listener<Q> interceptCall(ServerCall<Q, R> call, Metadata headers, ServerCallHandler<Q, R> next) {
                return Contexts.interceptCall(Context.current().withValue(HEADERS, headers), call, headers, next);
            }
        };
        server = Grpc.newServerBuilderForPort(0, InsecureServerCredentials.create())
                .addService(ServerInterceptors.intercept(this, capture))
                .build()
                .start();
    }

    String url() {
        return "http://127.0.0.1:" + server.getPort();
    }

    void failNext(String method, Failure... scripted) {
        Deque<Failure> queue = failures.computeIfAbsent(method, k -> new java.util.concurrent.ConcurrentLinkedDeque<>());
        for (Failure failure : scripted) {
            queue.add(failure);
        }
    }

    synchronized void addLeases(Worker.TaskLease... more) {
        for (Worker.TaskLease lease : more) {
            leases.add(lease);
        }
    }

    List<Call> of(String method) {
        List<Call> out = new ArrayList<>();
        for (Call call : calls) {
            if (call.method().equals(method)) {
                out.add(call);
            }
        }
        return out;
    }

    void reset() {
        calls.clear();
        synchronized (this) {
            leases.clear();
        }
        failures.clear();
        acceptedVersion = "1.2";
        cancelRequested = false;
        heartbeatExpiresMs = System.currentTimeMillis() + 60_000;
        secrets = List.of();
        pollDelayMs = 0;
        pollRetryAfterMs = 50;
        ignoreMax = false;
    }

    private <R> void answer(String method, Object request, StreamObserver<R> out, Function<Object, R> make) {
        calls.add(new Call(method, request, HEADERS.get(), System.nanoTime()));
        Deque<Failure> queue = failures.get(method);
        Failure failure = queue == null ? null : queue.poll();
        if (failure != null) {
            Metadata trailers = new Metadata();
            if (failure.reason() != null) {
                trailers.put(Metadata.Key.of("x-sagat-error", Metadata.ASCII_STRING_MARSHALLER), failure.reason());
            }
            if (failure.retryAfterMs() != null) {
                trailers.put(Metadata.Key.of("retry-after-ms", Metadata.ASCII_STRING_MARSHALLER), Long.toString(failure.retryAfterMs()));
            }
            out.onError(failure.code().toStatus().withDescription("scripted").asRuntimeException(trailers));
            return;
        }
        if (method.equals("PollTasks") && pollDelayMs > 0) {
            try {
                TimeUnit.MILLISECONDS.sleep(pollDelayMs);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        out.onNext(make.apply(request));
        out.onCompleted();
    }

    @Override
    public void registerWorker(Worker.RegisterWorkerRequest request, StreamObserver<Worker.RegisterWorkerResponse> out) {
        answer("RegisterWorker", request, out, r -> Worker.RegisterWorkerResponse.newBuilder()
                .setWorkerId("w-1")
                .setSessionToken("session-1")
                .setAcceptedProtocolVersion(acceptedVersion)
                .build());
    }

    @Override
    public void pollTasks(Worker.PollTasksRequest request, StreamObserver<Worker.PollTasksResponse> out) {
        answer("PollTasks", request, out, r -> {
            Worker.PollTasksResponse.Builder response = Worker.PollTasksResponse.newBuilder().setRetryAfterMs(pollRetryAfterMs);
            synchronized (this) {
                // Like the real service: no more than were asked for.
                for (int i = 0; (ignoreMax || i < request.getMaxTasks()) && !leases.isEmpty(); i++) {
                    response.addTasks(leases.poll());
                }
            }
            return response.build();
        });
    }

    @Override
    public void heartbeat(Worker.HeartbeatRequest request, StreamObserver<Worker.HeartbeatResponse> out) {
        answer("Heartbeat", request, out, r -> Worker.HeartbeatResponse.newBuilder()
                .setLeaseExpiresAtEpochMs(heartbeatExpiresMs)
                .setCancelRequested(cancelRequested)
                .build());
    }

    @Override
    public void completeStep(Worker.CompleteStepRequest request, StreamObserver<Worker.StepCommandResponse> out) {
        answer("CompleteStep", request, out, r -> Worker.StepCommandResponse.newBuilder()
                .setCommandId("c").setStepStatus("SUCCEEDED").setRunStatus("RUNNING").build());
    }

    @Override
    public void failStep(Worker.FailStepRequest request, StreamObserver<Worker.StepCommandResponse> out) {
        answer("FailStep", request, out, r -> Worker.StepCommandResponse.newBuilder()
                .setCommandId("c").setStepStatus("FAILED").setRunStatus("RUNNING").build());
    }

    @Override
    public void resolveSecrets(Worker.ResolveSecretsRequest request, StreamObserver<Worker.ResolveSecretsResponse> out) {
        answer("ResolveSecrets", request, out, r -> Worker.ResolveSecretsResponse.newBuilder().addAllSecrets(secrets).build());
    }

    static Worker.ResolvedSecret secret(String name, String value, int version) {
        return Worker.ResolvedSecret.newBuilder().setName(name).setValue(ByteString.copyFromUtf8(value)).setVersion(version).build();
    }

    @Override
    public void close() {
        server.shutdownNow();
        try {
            server.awaitTermination(2, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }
}
