package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Pattern;

/** Managed database authoring. SQL, authorization, policy and retry semantics remain server-owned. */
public final class DatabaseOptions {
    public enum Operation { QUERY, EXECUTE }
    public enum Type { NULL, BOOLEAN, INTEGER, DECIMAL, FLOAT, TEXT, TIMESTAMP, DATE, BINARY, JSON }

    /** A scoped logical name, never a DSN or provider path. */
    public record ConnectionRef(String name, Integer revision) {
        public ConnectionRef(String name) { this(name, null); }
    }

    /** Explicit SQL binding: integer/decimal strings, base64 binary, RFC3339 timestamp, YYYY-MM-DD date. */
    public static final class Parameter {
        private final Type type;
        private final Object value;
        private final boolean expression;

        private Parameter(Type type, Object value, boolean expression) {
            this.type = type;
            this.value = value;
            this.expression = expression;
        }

        public static Parameter literal(Type type, Object value) { return new Parameter(type, value, false); }
        public static Parameter expression(Type type, String expression) { return new Parameter(type, expression, true); }
        @Override public String toString() { return "[database parameter]"; }

        private ObjectNode toNode() {
            if (type == null) throw invalid();
            String name = type.name().toLowerCase(Locale.ROOT);
            if (expression) {
                if (!(value instanceof String text) || text.isEmpty()) throw invalid();
                return F.objectNode().put("type", name).put("expression", text);
            }
            JsonNode node = Json.toTree(value);
            boolean valid = switch (type) {
                case NULL -> node.isNull();
                case BOOLEAN -> node.isBoolean();
                case INTEGER -> node.isTextual() && node.textValue().length() <= 40 && INTEGER.matcher(node.textValue()).matches();
                case DECIMAL -> node.isTextual() && node.textValue().length() <= 16384 && DECIMAL.matcher(node.textValue()).matches();
                case FLOAT -> node.isNumber() && Double.isFinite(node.doubleValue());
                case TEXT -> node.isTextual() && node.textValue().indexOf('\0') < 0;
                case TIMESTAMP, DATE, BINARY -> node.isTextual();
                case JSON -> true;
            };
            if (!valid) throw invalid();
            ObjectNode literal = F.objectNode().put("type", name);
            literal.set("value", node);
            return F.objectNode().set("literal", literal);
        }
    }

    private static final JsonNodeFactory F = JsonNodeFactory.instance;
    private static final Pattern NAME = Pattern.compile("[a-z][a-z0-9-]{1,61}[a-z0-9]");
    private static final Pattern INTEGER = Pattern.compile("-?[0-9]+");
    private static final Pattern DECIMAL = Pattern.compile("-?[0-9]+(?:\\.[0-9]+)?");
    private static final Map<String, Integer> CEILINGS = Map.of(
        "maxStatementBytes", 65536, "maxParameters", 256, "maxParameterBytes", 262144,
        "maxRows", 10000, "maxResultBytes", 4194304, "timeoutMs", 30000,
        "connectTimeoutMs", 10000, "poolSize", 16, "idleTimeoutMs", 60000);

    private final ConnectionRef connectionRef;
    private final Operation operation;
    private final String statement;
    private List<Parameter> parameters;
    private Map<String, Integer> limits;
    private Long timeoutMs;
    private String onError;

    private DatabaseOptions(ConnectionRef connectionRef, Operation operation, String statement) {
        this.connectionRef = connectionRef;
        this.operation = operation;
        this.statement = statement;
    }

    public static DatabaseOptions create(ConnectionRef connectionRef, Operation operation, String statement) {
        return new DatabaseOptions(connectionRef, operation, statement);
    }

    public DatabaseOptions parameters(List<Parameter> values) { this.parameters = values; return this; }
    /** Optional tighter execution limits using the canonical IR field names. */
    public DatabaseOptions limits(Map<String, Integer> values) { this.limits = values; return this; }
    public DatabaseOptions timeoutMs(long value) { this.timeoutMs = value; return this; }
    public DatabaseOptions onError(String value) { this.onError = value; return this; }
    @Override public String toString() { return "[managed database operation]"; }

    static SagatException invalid() { return SagatException.invalid("invalid managed database operation"); }

    ObjectNode toNode(String id) {
        try {
            if (connectionRef == null || connectionRef.name() == null || !NAME.matcher(connectionRef.name()).matches()
                || connectionRef.name().contains("--") || (connectionRef.revision() != null && connectionRef.revision() < 1)
                || operation == null || statement == null || statement.isBlank() || statement.indexOf('\0') >= 0
                || statement.getBytes(StandardCharsets.UTF_8).length > 65536 || (parameters != null && parameters.size() > 256)
                || (onError != null && onError.isEmpty())) throw invalid();
            ObjectNode ref = F.objectNode().put("name", connectionRef.name());
            if (connectionRef.revision() != null) ref.put("revision", connectionRef.revision());
            ObjectNode node = F.objectNode().put("id", id).put("type", "database")
                .put("operation", operation.name().toLowerCase(Locale.ROOT)).put("statement", statement);
            node.set("connectionRef", ref);
            if (parameters != null) {
                ArrayNode values = F.arrayNode();
                for (Parameter parameter : parameters) values.add(parameter.toNode());
                if (Json.canonical(values).getBytes(StandardCharsets.UTF_8).length > 262144) throw invalid();
                node.set("parameters", values);
            }
            if (limits != null) {
                ObjectNode bounded = F.objectNode();
                for (Map.Entry<String, Integer> limit : limits.entrySet()) {
                    Integer maximum = CEILINGS.get(limit.getKey());
                    if (maximum == null || limit.getValue() == null || limit.getValue() < 1 || limit.getValue() > maximum) throw invalid();
                    bounded.put(limit.getKey(), limit.getValue());
                }
                node.set("limits", bounded);
            }
            if (timeoutMs != null) {
                if (timeoutMs < 1 || timeoutMs > 60000) throw invalid();
                node.put("timeoutMs", timeoutMs);
            }
            if (onError != null) node.put("onError", onError);
            // Snapshot all caller-owned JSON, and reject non-canonical values consistently across SDKs.
            return (ObjectNode) Json.parse(Json.canonical(node));
        } catch (RuntimeException error) {
            // No cause: conversion errors can quote SQL values or JSON property names.
            throw invalid();
        }
    }
}
