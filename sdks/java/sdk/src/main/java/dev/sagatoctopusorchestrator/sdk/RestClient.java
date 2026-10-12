package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.NullNode;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.http.HttpTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ThreadLocalRandom;
import java.util.regex.Pattern;

/** The REST transport: one place for headers, error mapping and the retry rules of SPEC section 5. */
final class RestClient {
    /** A call. {@code safeToRepeat} is true for a call whose repetition cannot do harm (GET, or a POST that carries a key). */
    record Request(
            String method,
            String path,
            Map<String, String> query,
            JsonNode body,
            String idempotencyKey,
            String traceparent,
            boolean safeToRepeat,
            InputStream stream,
            Map<String, String> headers) {
        Request(String method, String path, Map<String, String> query, JsonNode body, String idempotencyKey,
                String traceparent, boolean safeToRepeat) {
            this(method, path, query, body, idempotencyKey, traceparent, safeToRepeat, null, null);
        }
    }

    record Response(int status, String traceId, JsonNode body) {}

    private static final Pattern DIGITS = Pattern.compile("^\\d+$");

    private final ClientConfig config;
    private final HttpTransport transport;
    private final String base;

    RestClient(ClientConfig config) {
        this.config = config;
        this.base = config.apiUrl().replaceAll("/+$", "");
        this.transport = config.transport() != null
                ? config.transport()
                : new JdkHttpTransport(
                        config.timeout(), config.tls() == null ? null : config.tls().sslContext());
        Endpoints.warnIfPlainText(config.logger(), config.apiUrl(), "apiUrl");
    }

    Response request(Request req) {
        // Encoded once, before any attempt: a value the SDK cannot write is refused before anything is sent.
        byte[] body = req.body() == null ? null : Json.encode(req.body()).getBytes(StandardCharsets.UTF_8);
        int attempts = req.safeToRepeat() ? Math.max(1, config.retry().maxAttempts()) : 1;
        SagatException last = null;
        for (int n = 0; n < attempts; n++) {
            try {
                return once(req, body);
            } catch (SagatException error) {
                last = error;
                if (!error.isRetriable() || n + 1 >= attempts) {
                    throw error;
                }
                long wait = config.retry().delayAfterMillis(n, error, config.maxServerWait().toMillis(), () -> ThreadLocalRandom.current().nextDouble());
                Hooks.log(
                        config.logger(),
                        System.Logger.Level.WARNING,
                        "retrying a request method=" + req.method() + " path=" + req.path() + " code=" + error.code()
                                + " attempt=" + (n + 1) + " waitMs=" + wait);
                pause(wait);
            }
        }
        throw last != null ? last : SagatException.of(ErrorCode.INTERNAL, "no attempt was made");
    }

    static void pause(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw SagatException.builder(ErrorCode.CANCELLED, "interrupted while waiting").cause(e).build();
        }
    }

    private Response once(Request req, byte[] body) {
        StringBuilder url = new StringBuilder(base).append(req.path());
        if (req.query() != null && !req.query().isEmpty()) {
            char separator = '?';
            for (Map.Entry<String, String> entry : req.query().entrySet()) {
                if (entry.getValue() == null) {
                    continue;
                }
                url.append(separator).append(Endpoints.encodeComponent(entry.getKey())).append('=')
                        .append(Endpoints.encodeComponent(entry.getValue()));
                separator = '&';
            }
        }
        Map<String, String> headers = new LinkedHashMap<>();
        headers.put("Authorization", "Bearer " + TokenProvider.resolve(config.tokenProvider()));
        headers.put("Accept", "application/json");
        headers.put("X-Octopus-Namespace", config.namespace());
        headers.put("User-Agent", SagatSdk.userAgent(config.userAgent()));
        if (req.traceparent() != null) {
            headers.put("traceparent", req.traceparent());
        }
        if (req.idempotencyKey() != null) {
            headers.put("Idempotency-Key", req.idempotencyKey());
        }
        if (body != null) {
            headers.put("Content-Type", "application/json");
        }
        if (req.headers() != null) {
            headers.putAll(req.headers());
        }

        HttpTransport.Result result;
        try {
            result = transport.send(new HttpTransport.Call(req.method(), URI.create(url.toString()), headers, body, req.stream(), config.timeout()));
        } catch (SagatException e) {
            throw e;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw SagatException.builder(ErrorCode.CANCELLED, "interrupted while waiting for the server").cause(e).build();
        } catch (HttpTransport.ResponseTooLargeException e) {
            throw SagatException.of(ErrorCode.PROTOCOL, "the answer is larger than the SDK reads");
        } catch (IOException e) {
            boolean timedOut = e instanceof HttpTimeoutException;
            throw SagatException.builder(ErrorCode.TRANSPORT, timedOut ? "the request timed out" : "no answer from the server")
                    .cause(e)
                    .build();
        } catch (RuntimeException e) {
            // A transport of the caller: what it says is not kept (a library may print the request it was making).
            throw SagatException.of(ErrorCode.TRANSPORT, "no answer from the server (" + e.getClass().getSimpleName() + ")");
        }

        String traceId = result.header("x-trace-id").orElse("");
        byte[] bytes = result.body() == null ? new byte[0] : result.body();
        if (bytes.length > HttpTransport.MAX_RESPONSE_BYTES) {
            throw SagatException.builder(ErrorCode.PROTOCOL, "the answer is larger than the SDK reads").httpStatus(result.status()).build();
        }
        if (result.status() >= 300 && result.status() < 400) {
            // A redirect is never followed (SPEC section 1): the request carries a bearer token, the namespace and an
            // idempotency key, and a Location can point anywhere. Whatever its target, it is a PROTOCOL error that
            // does not repeat the Location.
            throw SagatException.builder(ErrorCode.PROTOCOL, "the server redirected the call")
                    .httpStatus(result.status())
                    .traceId(traceId)
                    .build();
        }
        if (result.status() >= 200 && result.status() < 300) {
            if (bytes.length == 0) {
                return new Response(result.status(), traceId, NullNode.getInstance());
            }
            try {
                return new Response(result.status(), traceId, Json.mapper().readTree(bytes));
            } catch (IOException e) {
                // Jackson's message quotes what it could not read: not kept, not even as a cause.
                throw SagatException.builder(ErrorCode.PROTOCOL, "the answer is not JSON")
                        .httpStatus(result.status())
                        .traceId(traceId)
                        .build();
            }
        }
        throw errorOf(result, bytes, traceId);
    }

    private static SagatException errorOf(HttpTransport.Result result, byte[] bytes, String headerTraceId) {
        // Seconds only (an HTTP date is ignored). More digits than a long holds is a very long wait: capped like any other.
        Long retryAfterMs = result.header("retry-after")
                .filter(value -> DIGITS.matcher(value).matches())
                .map(value -> value.length() > 12 ? Long.MAX_VALUE : Long.parseLong(value) * 1000L)
                .orElse(null);
        JsonNode envelope = null;
        try {
            JsonNode parsed = Json.mapper().readTree(bytes);
            if (parsed != null && parsed.isObject()) {
                envelope = parsed;
            }
        } catch (IOException e) {
            // An intermediary's page: classify by status.
        }
        ErrorCode given = envelope != null && envelope.path("code").isTextual() ? ErrorCode.fromEnvelope(envelope.path("code").textValue()) : null;
        // SPEC section 9: a call the (older) server does not have is answered 404 or 501 with no error envelope of the
        // platform; it is NOT_FOUND with the reason UNSUPPORTED_BY_SERVER. A 404 with an envelope is a missing resource.
        JsonNode reasonNode = envelope == null ? null : envelope.path("details").path("reason");
        String serverReason = reasonNode != null && reasonNode.isTextual() ? reasonNode.textValue() : null;
        boolean unsupported = result.status() == 501
                || (result.status() == 404 && (given == null || "ROUTE_NOT_FOUND".equals(serverReason)))
                || (result.status() == 405 && "METHOD_NOT_ALLOWED".equals(serverReason));
        ErrorCode code = unsupported ? ErrorCode.NOT_FOUND : given != null ? given : ErrorCatalog.codeOfHttpStatus(result.status());
        String reason = unsupported ? "UNSUPPORTED_BY_SERVER" : serverReason;
        String message = unsupported
                ? "the server does not have this call"
                : envelope != null && given != null && envelope.path("message").isTextual()
                        ? envelope.path("message").textValue()
                        : "the server answered " + result.status();
        Boolean retryable = unsupported ? Boolean.FALSE : envelope != null && given != null && envelope.path("retryable").isBoolean()
                ? envelope.path("retryable").booleanValue()
                : null;
        String traceId = envelope != null && envelope.path("traceId").isTextual() ? envelope.path("traceId").textValue() : headerTraceId;
        return SagatException.builder(code, message)
                .httpStatus(result.status())
                .retryable(retryable)
                .traceId(traceId)
                .reason(reason)
                .retryAfterMs(retryAfterMs)
                .build();
    }
}
