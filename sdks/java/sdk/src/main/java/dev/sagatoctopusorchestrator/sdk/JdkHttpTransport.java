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
        CompletableFuture<HttpResponse<byte[]>> pending = client.sendAsync(request.build(), HttpResponse.BodyHandlers.ofByteArray());
        HttpResponse<byte[]> response;
        try {
            response = pending.get(call.timeout().toMillis(), TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            pending.cancel(true);
            throw new HttpTimeoutException("the request timed out");
        } catch (InterruptedException e) {
            pending.cancel(true);
            throw e;
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
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
