package dev.sagatoctopusorchestrator.sdk;

import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * The REST transport hook: one blocking HTTP exchange. The SDK's default is {@link JdkHttpTransport}; an
 * application can wrap it (a proxy, a metrics layer) and the conformance suite wraps it to inject faults. An
 * implementation must throw {@link IOException} when there is no answer (connection refused, reset, timeout) and
 * must <em>not</em> retry (the SDK owns the retry rules of SPEC section 5) and <em>never follow a redirect</em>: the
 * request carries a bearer token, the namespace and an idempotency key, and a {@code Location} can point anywhere. A
 * {@code 3xx} answer must be returned as it is; the SDK raises it as {@code PROTOCOL}.
 */
@FunctionalInterface
public interface HttpTransport {
    /** The most of an answer the SDK reads (SPEC section 10): a server that streams on is told "no". */
    long MAX_RESPONSE_BYTES = 32L * 1024 * 1024;

    /** Thrown by a transport whose answer is larger than {@link #MAX_RESPONSE_BYTES}; the SDK raises it as {@code PROTOCOL}. */
    final class ResponseTooLargeException extends IOException {
        private static final long serialVersionUID = 1L;

        public ResponseTooLargeException() {
            super("the answer is larger than the SDK reads");
        }
    }

    /** One request. */
    record Call(String method, URI uri, Map<String, String> headers, byte[] body, InputStream stream, Duration timeout) {
        public Call(String method, URI uri, Map<String, String> headers, byte[] body, Duration timeout) {
            this(method, uri, headers, body, null, timeout);
        }
    }

    /** One answer. Header names are lower case. */
    record Result(int status, Map<String, List<String>> headers, byte[] body) {
        /** The first value of a header (the name is case-insensitive). */
        public Optional<String> header(String name) {
            List<String> values = headers.get(name.toLowerCase(java.util.Locale.ROOT));
            return values == null || values.isEmpty() ? Optional.empty() : Optional.of(values.get(0));
        }
    }

    /** Sends the request and returns the answer whatever its status. */
    Result send(Call call) throws IOException, InterruptedException;
}
