package sagat.host;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import sagat.inline.InlineScript;
import sagat.inline.Octopus;
import sagat.inline.Response;

import java.io.BufferedReader;
import java.io.PrintStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicLong;

public final class RuntimeHost {
    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Set<String> FIELDS = Set.of(
            "op", "input", "target", "encoding", "files", "idempotencyKey");

    private RuntimeHost() {}

    public static void main(String[] ignored) throws Exception {
        var channel = System.out;
        System.setOut(System.err);
        var replies = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        say(channel, Map.of("op", "ready"));
        try {
            var request = JSON.readTree(replies.readLine());
            if (!request.isObject() || !fieldNames(request).equals(FIELDS)
                    || !"run".equals(request.path("op").asText())
                    || !"jvm-17".equals(request.path("target").asText())
                    || !"base64".equals(request.path("encoding").asText())
                    || !request.path("files").isObject()) {
                throw new IllegalArgumentException("the Java run request has another shape");
            }
            var classes = decodeClasses(request.path("files"));
            var loader = new ScriptLoader(classes);
            var type = Class.forName("Script", true, loader);
            if (!InlineScript.class.isAssignableFrom(type)) {
                throw new IllegalArgumentException("Script must implement sagat.inline.InlineScript");
            }
            var script = (InlineScript) type.getDeclaredConstructor().newInstance();
            var input = JSON.convertValue(request.path("input"), Object.class);
            var key = request.path("idempotencyKey").isNull()
                    ? null : request.path("idempotencyKey").asText();
            var result = script.run(input, new OctopusHost(channel, replies, key));
            var response = new LinkedHashMap<String, Object>();
            response.put("op", "result"); response.put("value", result);
            say(channel, response);
        } catch (Throwable error) {
            var message = error.getMessage() == null ? error.getClass().getSimpleName() : error.getMessage();
            say(channel, Map.of("op", "error", "message", message, "stack", stack(error)));
            System.exit(1);
        }
    }

    private static Map<String, byte[]> decodeClasses(JsonNode files) {
        var decoded = new LinkedHashMap<String, byte[]>();
        long total = 0;
        var fields = files.fields();
        while (fields.hasNext()) {
            var field = fields.next();
            var name = field.getKey();
            if (!(name.equals("Script.class") || name.startsWith("Script$") && name.endsWith(".class"))
                    || !field.getValue().isTextual()) {
                throw new IllegalArgumentException("the Java artifact names an invalid class");
            }
            var encoded = field.getValue().textValue();
            var bytes = Base64.getDecoder().decode(encoded);
            if (!Base64.getEncoder().encodeToString(bytes).equals(encoded)) {
                throw new IllegalArgumentException("the Java artifact has noncanonical base64");
            }
            total += bytes.length;
            if (decoded.size() >= 32 || total > 1400L * 1024L) {
                throw new IllegalArgumentException("the Java artifact is too large");
            }
            decoded.put(name.substring(0, name.length() - ".class".length()), bytes);
        }
        if (!decoded.containsKey("Script")) {
            throw new IllegalArgumentException("the Java artifact has no Script.class");
        }
        return decoded;
    }

    private static Set<String> fieldNames(JsonNode node) {
        var names = new java.util.HashSet<String>();
        node.fieldNames().forEachRemaining(names::add);
        return names;
    }

    private static String stack(Throwable error) {
        var writer = new java.io.StringWriter();
        error.printStackTrace(new java.io.PrintWriter(writer));
        return writer.toString();
    }

    private static synchronized void say(PrintStream channel, Object value) throws Exception {
        channel.println(JSON.writeValueAsString(value));
        channel.flush();
    }

    private static final class ScriptLoader extends ClassLoader {
        private final Map<String, byte[]> classes;

        ScriptLoader(Map<String, byte[]> classes) {
            super(RuntimeHost.class.getClassLoader());
            this.classes = Map.copyOf(classes);
        }

        @Override
        protected Class<?> findClass(String name) throws ClassNotFoundException {
            var bytes = classes.get(name);
            if (bytes == null) {
                throw new ClassNotFoundException(name);
            }
            return defineClass(name, bytes, 0, bytes.length);
        }
    }

    private static final class OctopusHost implements Octopus {
        private final PrintStream channel;
        private final BufferedReader replies;
        private final String key;
        private final AtomicLong next = new AtomicLong();

        OctopusHost(PrintStream channel, BufferedReader replies, String key) {
            this.channel = channel;
            this.replies = replies;
            this.key = key;
        }

        @Override public String idempotencyKey() { return key; }

        @Override
        public Response fetch(String url, String method, Map<String, String> headers, String body)
                throws Exception {
            var request = new LinkedHashMap<String, Object>();
            request.put("op", "fetch"); request.put("url", url);
            request.put("method", method == null || method.isEmpty() ? "GET" : method);
            request.put("headers", headers == null ? Map.of() : headers); request.put("body", body);
            var reply = call(request);
            var replyHeaders = JSON.convertValue(reply.path("headers"), new TypeReference<Map<String, String>>() {});
            return new Response(reply.path("status").asInt(), replyHeaders, reply.path("body").asText(""));
        }

        @Override
        public String secret(String name) throws Exception {
            var request = new LinkedHashMap<String, Object>();
            request.put("op", "secret"); request.put("name", name);
            return call(request).path("value").asText();
        }

        @Override public void log(Object... values) {
            System.err.println(java.util.Arrays.toString(values));
        }

        private synchronized JsonNode call(Map<String, Object> request) throws Exception {
            var id = next.incrementAndGet();
            request.put("id", id);
            say(channel, request);
            while (true) {
                var line = replies.readLine();
                if (line == null) { throw new IllegalStateException("the platform closed the channel"); }
                var reply = JSON.readTree(line);
                if (reply.path("id").asLong(-1) != id) { continue; }
                if (!reply.path("ok").asBoolean(false)) {
                    throw new IllegalStateException(reply.path("error").asText("refused"));
                }
                return reply;
            }
        }
    }
}
