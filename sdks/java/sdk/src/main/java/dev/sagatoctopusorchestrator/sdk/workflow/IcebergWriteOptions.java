package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.workflow.DatabaseOptions.ConnectionRef;
import java.nio.charset.StandardCharsets;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * An append to an Iceberg table through the isolated Sagat lake runner. Only a scoped logical connection name and
 * revision appear in a workflow: no catalog, bucket, credential or secret. The mode is {@code append}; overwrite and
 * upsert are not built yet. Shape validation only: the catalog, access, modes and classification belong to the server.
 */
public final class IcebergWriteOptions {
    /** A table of an Iceberg catalog: a namespace of 1 to 8 levels and a name. */
    public record Table(List<String> namespace, String name) {}

    /** A field of a record (a JSON pointer, {@code /customer/name}) and the column it is written to. */
    public record Mapping(String field, String column) {}

    /** The undo of a write: removing its commit. */
    public enum Compensation { REVERT }

    /**
     * Where the records of the write come from: exactly one of an expression that evaluates to a list of objects, one
     * that evaluates to 1 to 32 blob descriptors, or the id of an earlier root-level database step.
     */
    public static final class Records {
        private final String source;
        private final String value;

        private Records(String source, String value) {
            this.source = source;
            this.value = value;
        }

        public static Records inline(String expression) { return new Records("inline", expression); }
        public static Records blobs(String expression) { return new Records("blobs", expression); }
        public static Records databaseResult(String stepId) { return new Records("databaseResult", stepId); }
        @Override public String toString() { return "[iceberg records]"; }
    }

    /** Optional tighter limits on one write; an empty set is kept as an empty object. */
    public static final class Limits {
        private Long maxRecordBytes;
        private Long maxRecords;
        private Long timeoutMs;

        public Limits maxRecordBytes(long value) { this.maxRecordBytes = value; return this; }
        public Limits maxRecords(long value) { this.maxRecords = value; return this; }
        public Limits timeoutMs(long value) { this.timeoutMs = value; return this; }
        @Override public String toString() { return "[iceberg limits]"; }
    }

    private static final JsonNodeFactory F = JsonNodeFactory.instance;
    private static final Pattern BAD_ESCAPE = Pattern.compile("~(?![01])");

    private final ConnectionRef connectionRef;
    private final Table table;
    private final Records records;
    private String ref;
    private List<Mapping> mapping;
    private Boolean ignoreUnknownFields;
    private Limits limits;
    private Compensation compensation;
    private String onError;

    private IcebergWriteOptions(ConnectionRef connectionRef, Table table, Records records) {
        this.connectionRef = connectionRef;
        this.table = table;
        this.records = records;
    }

    public static IcebergWriteOptions create(ConnectionRef connectionRef, Table table, Records records) {
        return new IcebergWriteOptions(connectionRef, table, records);
    }

    /** The branch or tag to commit to; {@code main} when absent. */
    public IcebergWriteOptions ref(String value) { this.ref = value; return this; }
    public IcebergWriteOptions mapping(List<Mapping> values) { this.mapping = values; return this; }
    public IcebergWriteOptions ignoreUnknownFields(boolean value) { this.ignoreUnknownFields = value; return this; }
    public IcebergWriteOptions limits(Limits value) { this.limits = value; return this; }
    public IcebergWriteOptions compensation(Compensation value) { this.compensation = value; return this; }
    public IcebergWriteOptions onError(String value) { this.onError = value; return this; }
    @Override public String toString() { return "[iceberg write]"; }

    static SagatException invalid() { return SagatException.invalid("invalid iceberg write"); }

    /** 1 to {@code limit} bytes of UTF-8 without a control character: the platform's plain text. */
    private static boolean plain(String text, int limit) {
        if (text == null || text.isEmpty() || text.getBytes(StandardCharsets.UTF_8).length > limit) return false;
        return text.codePoints().noneMatch(c -> Character.getType(c) == Character.CONTROL);
    }

    /** A level or a name of a table: plain, and not a path component of its own. */
    private static boolean part(String text) {
        return plain(text, 255) && !text.equals(".") && !text.equals("..");
    }

    /** A JSON pointer into a record whose escapes are {@code ~0} and {@code ~1}, of at most 16 segments. */
    private static boolean pointer(String text) {
        if (!plain(text, 512) || !text.startsWith("/")) return false;
        String[] segments = text.substring(1).split("/", -1);
        if (segments.length > 16) return false;
        for (String segment : segments) if (segment.isEmpty()) return false;
        return !BAD_ESCAPE.matcher(text).find();
    }

    private static void bounded(Long value, long low, long high) {
        if (value != null && (value < low || value > high)) throw invalid();
    }

    ObjectNode toNode(String id) {
        try {
            if (!DatabaseOptions.validConnectionRef(connectionRef) || table == null || records == null) throw invalid();
            List<String> namespace = table.namespace();
            if (namespace == null || namespace.isEmpty() || namespace.size() > 8 || !part(table.name())) throw invalid();
            for (String level : namespace) if (!part(level)) throw invalid();
            if (ref != null && !plain(ref, 255)) throw invalid();
            if (records.value == null) throw invalid();
            if (records.source.equals("databaseResult")) {
                if (!plain(records.value, 64)) throw invalid();
            } else {
                // an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the engine takes 1024
                int bytes = records.value.getBytes(StandardCharsets.UTF_8).length;
                if (bytes < 1 || bytes > 1024) throw invalid();
            }
            if (mapping != null && mapping.size() > 1024) throw invalid();
            if (onError != null && onError.isEmpty()) throw invalid();
            if (limits != null) {
                bounded(limits.maxRecordBytes, 1024, 2147483648L);
                bounded(limits.maxRecords, 1, 100000000L);
                bounded(limits.timeoutMs, 1000, 3600000L);
            }

            ObjectNode node = F.objectNode().put("id", id).put("type", "icebergWrite").put("mode", "append");
            ObjectNode connection = F.objectNode().put("name", connectionRef.name());
            if (connectionRef.revision() != null) connection.put("revision", connectionRef.revision());
            node.set("connectionRef", connection);
            ArrayNode levels = F.arrayNode();
            namespace.forEach(levels::add);
            node.set("table", F.objectNode().put("name", table.name()).<ObjectNode>set("namespace", levels));
            if (ref != null) node.put("ref", ref);
            node.set("records", F.objectNode().put(records.source, records.value));
            if (mapping != null) {
                // a field or a column twice is two rows of the mapping that disagree about one thing
                Set<String> fields = new HashSet<>();
                Set<String> columns = new HashSet<>();
                ArrayNode rows = F.arrayNode();
                for (Mapping pair : mapping) {
                    if (pair == null || !pointer(pair.field()) || !plain(pair.column(), 255)) throw invalid();
                    if (!fields.add(pair.field()) || !columns.add(pair.column())) throw invalid();
                    rows.add(F.objectNode().put("field", pair.field()).put("column", pair.column()));
                }
                node.set("mapping", rows);
            }
            if (ignoreUnknownFields != null) node.put("ignoreUnknownFields", ignoreUnknownFields);
            if (limits != null) {
                ObjectNode bounds = F.objectNode();
                if (limits.maxRecordBytes != null) bounds.put("maxRecordBytes", limits.maxRecordBytes);
                if (limits.maxRecords != null) bounds.put("maxRecords", limits.maxRecords);
                if (limits.timeoutMs != null) bounds.put("timeoutMs", limits.timeoutMs);
                node.set("limits", bounds);
            }
            if (compensation != null) node.put("compensation", "revert");
            if (onError != null) node.put("onError", onError);
            // Snapshot all caller-owned values, and reject non-canonical ones consistently across SDKs.
            return (ObjectNode) Json.parse(Json.canonical(node));
        } catch (RuntimeException error) {
            // No cause: conversion errors can quote names and expressions.
            throw invalid();
        }
    }
}
