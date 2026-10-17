package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.charset.StandardCharsets;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

final class StateDefinitionValidator {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;
    private static final Pattern OPERATION_NAME = Pattern.compile("^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$");

    private StateDefinitionValidator() {}

    static ObjectNode build(StateDefinition definition) {
        if (definition == null) {
            throw SagatException.invalid("state definition is required");
        }
        ObjectNode state = F.objectNode();
        state.set("schema", resultSchema(definition.schema(), "state.schema"));
        state.put("initial", expression(definition.initial(), "state.initial"));
        Map<String, QueryDefinition> queries = definition.queries() == null ? Map.of() : definition.queries();
        if (queries.size() > 32) {
            throw SagatException.invalid("state supports at most 32 queries");
        }
        if (!queries.isEmpty()) {
            ObjectNode declared = state.putObject("queries");
            queries.forEach((name, query) -> {
                operationName(name, "query");
                if (query == null) {
                    throw SagatException.invalid("query " + name + " is required");
                }
                disclosure(query.disclosure(), "query " + name);
                ObjectNode entry = declared.putObject(name);
                entry.set("argsSchema", argsSchema(query.argsSchema(), "query " + name));
                entry.set("resultSchema", resultSchema(query.resultSchema(), "query " + name + ".resultSchema"));
                entry.put("expression", expression(query.expression(), "query " + name));
                entry.put("disclosure", query.disclosure());
            });
        }
        Map<String, UpdateDefinition> updates = definition.updates() == null ? Map.of() : definition.updates();
        if (updates.size() > 32) {
            throw SagatException.invalid("state supports at most 32 updates");
        }
        if (!updates.isEmpty()) {
            ObjectNode declared = state.putObject("updates");
            updates.forEach((name, update) -> {
                operationName(name, "update");
                if (update == null) {
                    throw SagatException.invalid("update " + name + " is required");
                }
                disclosure(update.disclosure(), "update " + name);
                ObjectNode entry = declared.putObject(name);
                entry.set("argsSchema", argsSchema(update.argsSchema(), "update " + name));
                if (update.validate() != null) {
                    entry.put("validate", expression(update.validate(), "update " + name + ".validate"));
                }
                entry.put("state", expression(update.state(), "update " + name + ".state"));
                entry.put("result", expression(update.result(), "update " + name + ".result"));
                entry.set("resultSchema", resultSchema(update.resultSchema(), "update " + name + ".resultSchema"));
                entry.put("disclosure", update.disclosure());
            });
        }
        return state;
    }

    private static void operationName(String name, String kind) {
        if (name == null || !OPERATION_NAME.matcher(name).matches()) {
            throw SagatException.invalid(kind + " name \"" + name + "\" is invalid");
        }
    }

    private static void disclosure(String value, String subject) {
        if (!OperationDisclosure.DECLARED_BUSINESS_RESULT_V1.equals(value)) {
            throw SagatException.invalid(subject + " needs disclosure " + OperationDisclosure.DECLARED_BUSINESS_RESULT_V1);
        }
    }

    private static String expression(String value, String subject) {
        if (value == null || value.isEmpty()) {
            throw SagatException.invalid(subject + " needs an expression");
        }
        if (value.getBytes(StandardCharsets.UTF_8).length > 1024) {
            throw SagatException.invalid(subject + " expression exceeds 1024 UTF-8 bytes");
        }
        return value;
    }

    private static ObjectNode argsSchema(JsonNode value, String subject) {
        if (!(value instanceof ObjectNode object)) {
            throw SagatException.invalid(subject + ".argsSchema must be an object");
        }
        return object.deepCopy();
    }

    private static ObjectNode resultSchema(JsonNode value, String subject) {
        if (!(value instanceof ObjectNode schema)) {
            throw SagatException.invalid(subject + " must be a closed bounded schema");
        }
        JsonNode description = schema.get("description");
        if (description != null && (!description.isTextual()
                || description.textValue().codePointCount(0, description.textValue().length()) > 1024)) {
            throw SagatException.invalid(subject + ".description must be a string of at most 1024 characters");
        }
        Set<String> allowed = new HashSet<>(Set.of("type", "description"));
        String kind = schema.path("type").textValue();
        switch (kind == null ? "" : kind) {
            case "object" -> {
                allowed.addAll(Set.of("properties", "required", "additionalProperties"));
                if (!(schema.get("properties") instanceof ObjectNode properties)) {
                    throw SagatException.invalid(subject + ".properties must be an object");
                }
                if (!schema.has("additionalProperties") || !schema.path("additionalProperties").isBoolean()
                        || schema.path("additionalProperties").booleanValue()) {
                    throw SagatException.invalid(subject + ".additionalProperties must be false");
                }
                properties.properties().forEach(entry ->
                        resultSchema(entry.getValue(), subject + ".properties." + entry.getKey()));
                if (schema.has("required")) {
                    if (!(schema.get("required") instanceof ArrayNode required) || required.size() > 10_000) {
                        throw SagatException.invalid(subject + ".required must name unique declared properties");
                    }
                    Set<String> seen = new HashSet<>();
                    required.forEach(item -> {
                        if (!item.isTextual() || !seen.add(item.textValue()) || !properties.has(item.textValue())) {
                            throw SagatException.invalid(subject + ".required must name unique declared properties");
                        }
                    });
                }
            }
            case "array" -> {
                allowed.addAll(Set.of("items", "minItems", "maxItems"));
                resultSchema(schema.get("items"), subject + ".items");
                long maximum = integer(schema.get("maxItems"), 0, 10_000, subject + ".maxItems");
                if (schema.has("minItems")
                        && integer(schema.get("minItems"), 0, 10_000, subject + ".minItems") > maximum) {
                    throw SagatException.invalid(subject + ".minItems must not exceed maxItems");
                }
            }
            case "string" -> {
                allowed.addAll(Set.of("minLength", "maxLength"));
                long maximum = integer(schema.get("maxLength"), 0, 262_144, subject + ".maxLength");
                if (schema.has("minLength")
                        && integer(schema.get("minLength"), 0, 262_144, subject + ".minLength") > maximum) {
                    throw SagatException.invalid(subject + ".minLength must not exceed maxLength");
                }
            }
            case "integer", "number" -> {
                allowed.addAll(Set.of("minimum", "maximum"));
                Double minimum = number(schema.get("minimum"), subject, "minimum");
                Double maximum = number(schema.get("maximum"), subject, "maximum");
                if (minimum != null && maximum != null && minimum > maximum) {
                    throw SagatException.invalid(subject + ".minimum must not exceed maximum");
                }
            }
            case "boolean", "null" -> { }
            default -> throw SagatException.invalid(subject + ".type is not a supported closed result type");
        }
        schema.propertyStream().forEach(entry -> {
            if (!allowed.contains(entry.getKey())) {
                throw SagatException.invalid(subject + "." + entry.getKey()
                        + " is not supported in a closed result schema");
            }
        });
        if (Json.canonical(schema).getBytes(StandardCharsets.UTF_8).length > 65_536) {
            throw SagatException.invalid(subject + " exceeds 65536 UTF-8 bytes");
        }
        return schema.deepCopy();
    }

    private static long integer(JsonNode value, long minimum, long maximum, String subject) {
        if (value == null || !value.isIntegralNumber() || !value.canConvertToLong()) {
            throw SagatException.invalid(subject + " must be between " + minimum + " and " + maximum);
        }
        long parsed = value.longValue();
        if (parsed < minimum || parsed > maximum) {
            throw SagatException.invalid(subject + " must be between " + minimum + " and " + maximum);
        }
        return parsed;
    }

    private static Double number(JsonNode value, String subject, String field) {
        if (value == null) {
            return null;
        }
        if (!value.isNumber() || !Double.isFinite(value.doubleValue())) {
            throw SagatException.invalid(subject + "." + field + " must be a finite number");
        }
        return value.doubleValue();
    }
}
