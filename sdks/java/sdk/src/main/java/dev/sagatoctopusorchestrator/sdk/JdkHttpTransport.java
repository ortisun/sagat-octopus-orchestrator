package dev.sagatoctopusorchestrator.sdk;

import java.io.IOException;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.net.http.HttpTimeoutException;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import javax.net.ssl.SSLContext;

/**
 * The default {@link HttpTransport}: {@code java.net.http.HttpClient}, HTTP/1.1, and <strong>never a redirect</strong>:
 * the request carries a bearer token, the namespace and an idempotency key, and a {@code Location} can point anywhere
 * (SPEC section 1). A {@code 3xx} answer reaches the SDK, which raises it as {@code PROTOCOL}.
 */
public final class JdkHttpTransport implements HttpTransport {
    private final HttpClient client;

    /**
     * The body, read up to {@link HttpTransport#MAX_RESPONSE_BYTES}: a longer one (declared, or found out while reading)
     * cancels the stream and the connection and completes {@code tooLarge}, so that the call need not wait for a body
     * that the headers already condemned.
     */
    private static final class BoundedHandler implements HttpResponse.BodyHandler<byte[]> {
        final CompletableFuture<Void> tooLarge = new CompletableFuture<>();

        @Override
        public HttpResponse.BodySubscriber<byte[]> apply(HttpResponse.ResponseInfo info) {
            long declared = info.headers().firstValueAsLong("content-length").orElse(-1);
            if (declared > MAX_RESPONSE_BYTES) {
                tooLarge.complete(null);
            }
            return new BoundedBody(tooLarge, declared > MAX_RESPONSE_BYTES);
        }
    }

    private static final class BoundedBody implements HttpResponse.BodySubscriber<byte[]> {
        private final CompletableFuture<byte[]> result = new CompletableFuture<>();
        private final java.io.ByteArrayOutputStream buffer = new java.io.ByteArrayOutputStream();
        private final CompletableFuture<Void> tooLarge;
        private final boolean refuse;
        private java.util.concurrent.Flow.Subscription subscription;
        private long total;

        BoundedBody(CompletableFuture<Void> tooLarge, boolean refuse) {
            this.tooLarge = tooLarge;
            this.refuse = refuse;
        }

        @Override
        public java.util.concurrent.CompletionStage<byte[]> getBody() {
            return result;
        }

        @Override
        public void onSubscribe(java.util.concurrent.Flow.Subscription subscription) {
            this.subscription = subscription;
            if (refuse) {
                // The headers already say it is too large: the body is not read at all.
                subscription.cancel();
                result.completeExceptionally(new ResponseTooLargeException());
                return;
            }
            subscription.request(1);
        }

        @Override
        public void onNext(java.util.List<java.nio.ByteBuffer> items) {
            if (result.isDone()) {
                return;
            }
            for (java.nio.ByteBuffer item : items) {
                total += item.remaining();
                if (total > MAX_RESPONSE_BYTES) {
                    subscription.cancel();
                    tooLarge.complete(null);
                    result.completeExceptionally(new ResponseTooLargeException());
                    return;
                }
                byte[] chunk = new byte[item.remaining()];
                item.get(chunk);
                buffer.write(chunk, 0, chunk.length);
            }
            subscription.request(1);
        }

        @Override
        public void onError(Throwable throwable) {
            result.completeExceptionally(throwable);
        }

        @Override
        public void onComplete() {
            result.complete(buffer.toByteArray());
        }
    }

    private JdkHttpTransport(HttpClient client) {
        this.client = client;
    }

    /**
     * A transport over an {@link HttpClient} you built (a proxy, a custom executor, your own {@code SSLContext}). The
     * client must not follow redirects ({@code followRedirects(HttpClient.Redirect.NEVER)}, which is also the JDK's
     * default); one that does is refused with {@code VALIDATION_FAILED}, because the SDK cannot make it stop.
     */
    public static JdkHttpTransport using(HttpClient client) {
        if (client.followRedirects() != HttpClient.Redirect.NEVER) {
            throw SagatException.invalid("the HttpClient follows redirects: build it with followRedirects(HttpClient.Redirect.NEVER)");
        }
        return new JdkHttpTransport(client);
    }

    /** A transport with the JVM's default TLS settings. */
    public JdkHttpTransport(Duration connectTimeout) {
        this(connectTimeout, null);
    }

    /** A transport with a given {@link SSLContext} (null for the default). */
    public JdkHttpTransport(Duration connectTimeout, SSLContext sslContext) {
        HttpClient.Builder builder = HttpClient.newBuilder()
                .version(HttpClient.Version.HTTP_1_1)
                .followRedirects(HttpClient.Redirect.NEVER)
                .connectTimeout(connectTimeout);
        if (sslContext != null) {
            builder.sslContext(sslContext);
        }
        this.client = builder.build();
    }

    @Override
    public Result send(Call call) throws IOException, InterruptedException {
        HttpRequest.Builder request = HttpRequest.newBuilder(call.uri()).timeout(call.timeout());
        call.headers().forEach(request::header);
        request.method(
                call.method(),
                call.body() == null ? HttpRequest.BodyPublishers.noBody() : HttpRequest.BodyPublishers.ofByteArray(call.body()));
        // The JDK's request timeout ends when the response headers arrive: a server that sends them and then stalls the body
        // would hold the call for ever. The deadline of the call (SPEC section 1) covers the whole exchange.
        BoundedHandler handler = new BoundedHandler();
        CompletableFuture<HttpResponse<byte[]>> pending = client.sendAsync(request.build(), handler);
        HttpResponse<byte[]> response;
        try {
            CompletableFuture.anyOf(pending, handler.tooLarge).get(call.timeout().toMillis(), TimeUnit.MILLISECONDS);
            if (handler.tooLarge.isDone()) {
                pending.cancel(true);
                throw new ResponseTooLargeException();
            }
            response = pending.get();
        } catch (TimeoutException e) {
            pending.cancel(true);
            throw new HttpTimeoutException("the request timed out");
        } catch (InterruptedException e) {
            pending.cancel(true);
            throw e;
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            while (cause instanceof java.util.concurrent.CompletionException && cause.getCause() != null) {
                cause = cause.getCause();
            }
            if (cause instanceof IOException io) {
                throw io;
            }
            if (cause instanceof RuntimeException runtime) {
                throw runtime;
            }
            if (cause instanceof Error error) {
                throw error;
            }
            throw new IOException(cause);
        }
        Map<String, List<String>> headers = new LinkedHashMap<>();
        response.headers().map().forEach((name, values) -> headers.put(name.toLowerCase(Locale.ROOT), values));
        return new Result(response.statusCode(), headers, response.body());
    }
}
