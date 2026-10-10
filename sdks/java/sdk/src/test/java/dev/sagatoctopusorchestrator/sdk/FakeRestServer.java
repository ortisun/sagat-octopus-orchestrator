package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.function.BiConsumer;

/** An in-process REST server the tests script: a record of every request and a handler for the answers. */
final class FakeRestServer implements AutoCloseable {
    /** One request as the server saw it. */
    record Seen(String method, String uri, Map<String, List<String>> headers, JsonNode body, String rawBody) {
        String path() {
            int q = uri.indexOf('?');
            return q < 0 ? uri : uri.substring(0, q);
        }

        String header(String name) {
            for (Map.Entry<String, List<String>> e : headers.entrySet()) {
                if (e.getKey().equalsIgnoreCase(name)) {
                    return e.getValue().get(0);
                }
            }
            return null;
        }
    }

    /** What a handler answers with. */
    interface Reply {
        void send(int status, String body, String... headerPairs);
    }

    final List<Seen> seen = new CopyOnWriteArrayList<>();
    private final HttpServer server;
    private volatile BiConsumer<Seen, Reply> handler = (request, reply) -> reply.send(200, "{}");

    FakeRestServer() throws IOException {
        server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/", exchange -> {
            try {
                Seen request = read(exchange);
                seen.add(request);
                handler.accept(request, (status, body, headers) -> write(exchange, status, body, headers));
            } catch (RuntimeException | IOException e) {
                exchange.close();
            }
        });
        server.setExecutor(java.util.concurrent.Executors.newCachedThreadPool());
        server.start();
    }

    String base() {
        return "http://127.0.0.1:" + server.getAddress().getPort() + "/v1";
    }

    /** Clears the record and installs the handler of the next test step. */
    void prepare(BiConsumer<Seen, Reply> next) {
        seen.clear();
        handler = next;
    }

    private static Seen read(HttpExchange exchange) throws IOException {
        byte[] bytes = exchange.getRequestBody().readAllBytes();
        String text = new String(bytes, StandardCharsets.UTF_8);
        JsonNode body = text.isEmpty() ? null : Json.mapper().readTree(text);
        return new Seen(exchange.getRequestMethod(), exchange.getRequestURI().toString(), Map.copyOf(exchange.getRequestHeaders()), body, text);
    }

    private static void write(HttpExchange exchange, int status, String body, String... headerPairs) {
        try {
            exchange.getResponseHeaders().add("Content-Type", "application/json");
            boolean ownTrace = false;
            for (int i = 0; i + 1 < headerPairs.length; i += 2) {
                ownTrace |= headerPairs[i].equalsIgnoreCase("X-Trace-Id");
                exchange.getResponseHeaders().add(headerPairs[i], headerPairs[i + 1]);
            }
            if (!ownTrace) {
                exchange.getResponseHeaders().add("X-Trace-Id", "f".repeat(32));
            }
            byte[] bytes = body == null ? new byte[0] : body.getBytes(StandardCharsets.UTF_8);
            exchange.sendResponseHeaders(status, bytes.length == 0 ? -1 : bytes.length);
            if (bytes.length > 0) {
                try (OutputStream out = exchange.getResponseBody()) {
                    out.write(bytes);
                }
            }
            exchange.close();
        } catch (IOException e) {
            exchange.close();
        }
    }

    @Override
    public void close() {
        server.stop(0);
        ((java.util.concurrent.ExecutorService) server.getExecutor()).shutdownNow();
    }
}
