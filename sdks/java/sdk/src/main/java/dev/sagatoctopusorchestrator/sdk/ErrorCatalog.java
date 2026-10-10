package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.util.ArrayList;
import java.util.EnumMap;
import java.util.List;
import java.util.Map;

/**
 * The embedded copy of {@code sdks/error-catalog.json}. A unit test proves the copy equals the repository's file.
 */
public final class ErrorCatalog {
    /** One row of the catalog. */
    public record Entry(ErrorCode code, String origin, List<Integer> http, String grpc, boolean retryable, String remediation) {}

    /** The resource inside the jar. */
    public static final String RESOURCE = "/dev/sagatoctopusorchestrator/sdk/error-catalog.json";

    private static final List<Entry> ENTRIES;
    private static final Map<ErrorCode, Entry> BY_CODE = new EnumMap<>(ErrorCode.class);

    static {
        try (InputStream in = ErrorCatalog.class.getResourceAsStream(RESOURCE)) {
            if (in == null) {
                throw new IllegalStateException("the error catalog is missing from the SDK jar: " + RESOURCE);
            }
            JsonNode root = Json.mapper().readTree(in);
            List<Entry> entries = new ArrayList<>();
            for (JsonNode row : root.path("errors")) {
                List<Integer> http = new ArrayList<>();
                row.path("http").forEach(status -> http.add(status.intValue()));
                Entry entry = new Entry(
                        ErrorCode.valueOf(row.path("code").asText()),
                        row.path("origin").asText(),
                        List.copyOf(http),
                        row.path("grpc").asText(),
                        row.path("retryable").asBoolean(false),
                        row.path("remediation").asText());
                entries.add(entry);
                BY_CODE.put(entry.code(), entry);
            }
            ENTRIES = List.copyOf(entries);
        } catch (IOException e) {
            throw new UncheckedIOException("the error catalog cannot be read", e);
        }
    }

    private ErrorCatalog() {}

    /** Every row, in the order of the file. */
    public static List<Entry> entries() {
        return ENTRIES;
    }

    /** The row of a code. */
    public static Entry entry(ErrorCode code) {
        Entry entry = BY_CODE.get(code);
        if (entry == null) {
            throw new IllegalStateException("the error catalog has no row for " + code);
        }
        return entry;
    }

    /** The code the catalog gives an HTTP status when the body carried no envelope. */
    public static ErrorCode codeOfHttpStatus(int status) {
        for (Entry entry : ENTRIES) {
            if ("rest".equals(entry.origin()) && entry.http().contains(status)) {
                return entry.code();
            }
        }
        return status >= 500 ? ErrorCode.INTERNAL : ErrorCode.BAD_REQUEST;
    }
}
