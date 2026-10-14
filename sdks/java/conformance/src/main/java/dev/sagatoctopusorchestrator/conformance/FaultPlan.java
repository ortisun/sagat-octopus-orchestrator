package dev.sagatoctopusorchestrator.conformance;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.HttpTransport;
import dev.sagatoctopusorchestrator.sdk.Json;
import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.ClientCall;
import io.grpc.ClientInterceptor;
import io.grpc.ForwardingClientCall;
import io.grpc.ForwardingClientCallListener;
import io.grpc.Metadata;
import io.grpc.MethodDescriptor;
import io.grpc.Status;
import java.io.IOException;
import java.net.ConnectException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Pattern;

/**
 * The faults of a scenario (tests/contract/sdk-conformance/README.md, "Faults"): one plan for every client and worker
 * host of the run. A call is counted against every fault that matches it, and the first fault in file order that is due
 * answers it; {@link #applied()} says, per fault and in file order, how many calls it answered.
 */
final class FaultPlan {
    private record Fault(
            String channel,
            String match,
            String action,
            long times,
            long after,
            int status,
            String code,
            String reason,
            Long retryAfterMs,
            JsonNode body) {}

    private final List<Fault> faults = new ArrayList<>();
    private final long[] seen;
    private final long[] answered;

    FaultPlan(JsonNode spec) {
        for (JsonNode f : spec) {
            faults.add(new Fault(
                    f.path("channel").asText(),
                    f.path("match").asText(),
                    f.path("action").asText(),
                    f.has("times") ? f.get("times").asLong() : Long.MAX_VALUE,
                    f.has("after") ? f.get("after").asLong() : 0,
                    f.has("status") ? f.get("status").asInt() : 503,
                    f.has("code") ? f.get("code").asText() : "UNAVAILABLE",
                    f.has("reason") ? f.get("reason").asText() : null,
                    f.has("retryAfterMs") ? Long.valueOf(f.get("retryAfterMs").asLong()) : null,
                    f.get("body")));
        }
        this.seen = new long[faults.size()];
        this.answered = new long[faults.size()];
    }

    synchronized long[] applied() {
        return answered.clone();
    }

    private synchronized Fault next(String channel, String key) {
        int due = -1;
        for (int i = 0; i < faults.size(); i++) {
            Fault fault = faults.get(i);
            if (!fault.channel().equals(channel) || !key.startsWith(fault.match())) {
                continue;
            }
            long count = seen[i]++;
            if (due < 0 && count >= fault.after() && count - fault.after() < fault.times()) {
                due = i;
            }
        }
        if (due < 0) {
            return null;
        }
        answered[due]++;
        return faults.get(due);
    }

    boolean hasGrpcFaults() {
        return faults.stream().anyMatch(f -> f.channel().equals("grpc"));
    }

    /** The REST transport hook: what a fault says happens to the call, otherwise the real thing. */
    HttpTransport wrap(HttpTransport delegate, String apiPath) {
        return call -> {
            String path = call.uri().getPath().replaceFirst(Pattern.quote(apiPath), "");
            String key = call.method().toUpperCase(Locale.ROOT) + " " + (path.isEmpty() ? "/" : path);
            Fault fault = next("http", key);
            if (fault == null) {
                return delegate.send(call);
            }
            switch (fault.action()) {
                case "failBefore":
                    throw new ConnectException("connection refused");
                case "status": {
                    Map<String, List<String>> headers = new java.util.LinkedHashMap<>();
                    headers.put("content-type", List.of("application/json"));
                    if (fault.retryAfterMs() != null) {
                        headers.put("retry-after", List.of(Long.toString((fault.retryAfterMs() + 999) / 1000)));
                    }
                    JsonNode body = fault.body() == null ? Json.mapper().createObjectNode() : fault.body();
                    return new HttpTransport.Result(fault.status(), headers, Json.encode(body).getBytes(StandardCharsets.UTF_8));
                }
                default:
                    // dropResponse: the call is made, the server applies it, the answer is lost.
                    delegate.send(call);
                    throw new IOException("connection reset by peer");
            }
        };
    }

    /** The interceptor of the worker's gRPC channel. */
    List<ClientInterceptor> interceptors() {
        if (!hasGrpcFaults()) {
            return List.of();
        }
        return List.of(new ClientInterceptor() {
            @Override
            public <Q, R> ClientCall<Q, R> interceptCall(MethodDescriptor<Q, R> method, CallOptions options, Channel next) {
                Fault fault = FaultPlan.this.next("grpc", method.getBareMethodName());
                if (fault == null) {
                    return next.newCall(method, options);
                }
                if (fault.action().equals("failBefore") || fault.action().equals("status")) {
                    Status.Code code = fault.action().equals("failBefore") ? Status.Code.UNAVAILABLE : Status.Code.valueOf(fault.code());
                    Metadata trailers = new Metadata();
                    if (fault.reason() != null) {
                        trailers.put(Metadata.Key.of("x-sagat-error", Metadata.ASCII_STRING_MARSHALLER), fault.reason());
                    }
                    if (fault.retryAfterMs() != null) {
                        trailers.put(Metadata.Key.of("retry-after-ms", Metadata.ASCII_STRING_MARSHALLER), Long.toString(fault.retryAfterMs()));
                    }
                    // The call never reaches the server: there is nothing underneath.
                    return new FailingCall<>(code.toStatus().withDescription("injected"), trailers);
                }
                if (fault.action().equals("dropStreamAfterFirst")) {
                    return new DropAfterFirstCall<>(next.newCall(method, options));
                }
                // dropResponse: the server applies the call; its answer is replaced by a failure to deliver it.
                return new ForwardingClientCall.SimpleForwardingClientCall<Q, R>(next.newCall(method, options)) {
                    @Override
                    public void start(Listener<R> listener, Metadata headers) {
                        super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<R>(listener) {
                            @Override
                            public void onMessage(R message) {
                                // thrown away
                            }

                            @Override
                            public void onClose(Status status, Metadata trailers) {
                                super.onClose(Status.UNAVAILABLE.withDescription("injected"), new Metadata());
                            }
                        }, headers);
                    }
                };
            }
        });
    }

    /** Forwards one real nonempty task frame, then cancels only that receive and reports UNAVAILABLE. */
    private static final class DropAfterFirstCall<Q, R> extends ForwardingClientCall.SimpleForwardingClientCall<Q, R> {
        private boolean forwarded;

        DropAfterFirstCall(ClientCall<Q, R> delegate) {
            super(delegate);
        }

        @Override
        public void start(Listener<R> listener, Metadata headers) {
            super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<R>(listener) {
                @Override
                public void onMessage(R message) {
                    super.onMessage(message);
                    if (!forwarded && message instanceof Worker.PollTasksResponse frame && frame.getTasksCount() > 0) {
                        forwarded = true;
                        DropAfterFirstCall.this.cancel("injected", null);
                    }
                }

                @Override
                public void onClose(Status status, Metadata trailers) {
                    if (forwarded) {
                        super.onClose(Status.UNAVAILABLE.withDescription("injected"), new Metadata());
                    } else {
                        super.onClose(status, trailers);
                    }
                }
            }, headers);
        }
    }

    private static final class FailingCall<Q, R> extends ClientCall<Q, R> {
        private final Status status;
        private final Metadata trailers;

        FailingCall(Status status, Metadata trailers) {
            this.status = status;
            this.trailers = trailers;
        }

        @Override
        public void start(Listener<R> listener, Metadata headers) {
            listener.onClose(status, trailers);
        }

        @Override
        public void request(int numMessages) {}

        @Override
        public void cancel(String message, Throwable cause) {}

        @Override
        public void halfClose() {}

        @Override
        public void sendMessage(Q message) {}
    }
}
