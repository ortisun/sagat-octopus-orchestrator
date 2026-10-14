package sagat.host;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class BuildHost {
    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Set<String> FIELDS = Set.of("op", "source", "lockfile", "packages");

    private BuildHost() {}

    public static void main(String[] ignored) throws Exception {
        say(Map.of("op", "ready"));
        try {
            build();
        } catch (Exception error) {
            say(Map.of("op", "rejected", "message", String.valueOf(error.getMessage())));
            System.exit(1);
        }
    }

    private static void build() throws Exception {
        var line = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8)).readLine();
        var request = JSON.readTree(line == null ? "" : line);
        if (!request.isObject() || !fieldNames(request).equals(FIELDS)
                || !"build".equals(request.path("op").asText()) || !request.path("source").isTextual()
                || !(request.path("lockfile").isNull() || request.path("lockfile").asText().isEmpty())
                || !request.path("packages").isArray() || !request.path("packages").isEmpty()) {
            throw new IllegalArgumentException("the Java build request has another shape");
        }
        var root = Path.of("/tmp/sagat-build");
        Files.createDirectory(root);
        var source = root.resolve("Script.java");
        Files.writeString(source, request.path("source").textValue(), StandardCharsets.UTF_8);
        var classes = root.resolve("classes");
        Files.createDirectory(classes);

        var command = new ProcessBuilder(
                "/opt/java/openjdk/bin/javac", "--release", "17", "-proc:none", "-encoding", "UTF-8",
                "-classpath", "/opt/sagat/api/sagat-inline-api.jar", "-d", classes.toString(), source.toString());
        command.environment().clear();
        command.environment().put("LANG", "C.UTF-8");
        command.redirectOutput(ProcessBuilder.Redirect.DISCARD);
        var process = command.start();
        var diagnostic = new ByteArrayOutputStream();
        process.getErrorStream().transferTo(diagnostic);
        if (process.waitFor() != 0) {
            System.err.write(diagnostic.toByteArray());
            var lines = diagnostic.toString(StandardCharsets.UTF_8).lines().toList();
            throw new IllegalArgumentException(lines.isEmpty() ? "javac failed" : lines.get(0));
        }

        var files = new LinkedHashMap<String, String>();
        var total = 0L;
        try (var paths = Files.walk(classes)) {
            for (var path : paths.filter(Files::isRegularFile).sorted().toList()) {
                var relative = classes.relativize(path).toString();
                if (relative.contains("/") || relative.contains("\\")
                        || !(relative.equals("Script.class")
                            || relative.startsWith("Script$") && relative.endsWith(".class"))) {
                    throw new IllegalArgumentException("javac produced a class outside Script and its nested classes");
                }
                var bytes = Files.readAllBytes(path);
                total += bytes.length;
                if (files.size() >= 32 || total > 1400L * 1024L) {
                    throw new IllegalArgumentException("the Java artifact is too large");
                }
                files.put(relative, Base64.getEncoder().encodeToString(bytes));
            }
        }
        if (!files.containsKey("Script.class")) {
            throw new IllegalArgumentException("javac produced no Script.class");
        }
        say(Map.of("op", "built", "entry", "Script.class", "files", files));
    }

    private static Set<String> fieldNames(JsonNode node) {
        var names = new java.util.HashSet<String>();
        node.fieldNames().forEachRemaining(names::add);
        return names;
    }

    private static void say(Object value) throws Exception {
        System.out.println(JSON.writeValueAsString(value));
        System.out.flush();
    }
}
