package dev.sagatoctopusorchestrator.sdk.worker;

import io.grpc.CallOptions;
import io.grpc.Channel;
import io.grpc.ClientCall;
import io.grpc.ClientInterceptor;
import io.grpc.ForwardingClientCall;
import io.grpc.ForwardingClientCallListener;
import io.grpc.Metadata;
import io.grpc.MethodDescriptor;
import io.grpc.Status;
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * What the host sent and which of its calls were answered, in the order the host saw it: "sent Heartbeat",
 * "answered CompleteStep". The side of the host, so a beat that was sent before the report but reaches the fake after
 * it cannot blur what a test reads.
 */
final class SendLog implements ClientInterceptor {
    final List<String> entries = new CopyOnWriteArrayList<>();

    @Override
    public <Q, R> ClientCall<Q, R> interceptCall(MethodDescriptor<Q, R> method, CallOptions options, Channel next) {
        String name = method.getBareMethodName();
        return new ForwardingClientCall.SimpleForwardingClientCall<>(next.newCall(method, options)) {
            @Override
            public void start(Listener<R> listener, Metadata headers) {
                entries.add("sent " + name);
                super.start(new ForwardingClientCallListener.SimpleForwardingClientCallListener<>(listener) {
                    @Override
                    public void onClose(Status status, Metadata trailers) {
                        if (status.isOk()) {
                            entries.add("answered " + name);
                        }
                        super.onClose(status, trailers);
                    }
                }, headers);
            }
        };
    }

    /** How many times {@code entry} comes after the last {@code mark}; every time if there is no mark. */
    long after(String mark, String entry) {
        List<String> all = List.copyOf(entries);
        return all.subList(all.lastIndexOf(mark) + 1, all.size()).stream().filter(entry::equals).count();
    }
}
