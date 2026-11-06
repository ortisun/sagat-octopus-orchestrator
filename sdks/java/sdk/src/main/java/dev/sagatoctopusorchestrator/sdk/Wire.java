package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Duration;
import java.time.Instant;
import java.time.OffsetDateTime;
import java.time.format.DateTimeParseException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.NoSuchElementException;
import java.util.Spliterator;
import java.util.Spliterators;
import java.util.function.Function;
import java.util.regex.Pattern;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * The checks the feature 018 calls make before anything is sent, and the typed reads of their answers. An answer that
 * does not have the shape the contract promises is a {@code PROTOCOL} error, never a guess; a value the SDK cannot
 * send is {@code VALIDATION_FAILED} and nothing leaves the process.
 */
final class Wire {
    private static final Pattern UUID_TEXT = Pattern.compile("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", Pattern.CASE_INSENSITIVE);
    private static final Pattern KEY = Pattern.compile("[\\x21-\\x7e]{8,160}");
    private static final Pattern UPDATE_KEY = Pattern.compile("[\\x21-\\x7e]{1,128}");
    private static final Pattern BUSINESS_ID_PATH = Pattern.compile("[\\x21-\\x7e]{1,200}");
    private static final Pattern OPERATION_NAME = Pattern.compile("[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}");

    private Wire() {}

    // ------------------------------------------------------------------------------------ what is sent

    /** The idempotency key of a call: the caller's, or one generated once for it. */
    static String key(String given) {
        String key = given != null ? given : java.util.UUID.randomUUID().toString();
        if (!KEY.matcher(key).matches()) {
            throw SagatException.invalid("an idempotency key is 8 to 160 printable ASCII characters");
        }
        return key;
    }

    /** An id that becomes a path segment: a UUID, in lower case. */
    static String uuid(String value, String what) {
        if (value == null || !UUID_TEXT.matcher(value).matches()) {
            throw SagatException.invalid(what + " is a UUID");
        }
        return value.toLowerCase(Locale.ROOT);
    }

    /** The caller's key of an update: 1 to 128 printable ASCII characters. */
    static String updateKey(String value) {
        if (value == null || !UPDATE_KEY.matcher(value).matches()) {
            throw SagatException.invalid("an update key is 1 to 128 printable ASCII characters");
        }
        return value;
    }

    static String operationName(String value) {
        if (value == null || !OPERATION_NAME.matcher(value).matches()) {
            throw SagatException.invalid("an operation name is 1 to 128 letters, digits, dots, underscores, colons or hyphens and starts with a letter or digit");
        }
        return value;
    }

    /**
     * A business identifier as a path segment: sent as given, refused only when it cannot be one. A whole segment of
     * {@code .} or {@code ..} (also written {@code %2E}) is a step in the path for every URL resolver, so the request
     * would reach another route: it is refused. An identifier that merely contains dots ({@code ...}, {@code a..b}) is
     * valid and percent-encoded like any other.
     */
    static String businessIdForPath(String value) {
        if (value == null || !BUSINESS_ID_PATH.matcher(value).matches()) {
            throw SagatException.invalid("a business identifier is 1 to 200 visible ASCII characters");
        }
        if (value.equals(".") || value.equals("..")) {
            throw SagatException.invalid("a business identifier is not \".\" or \"..\": it would be a step in the path");
        }
        return value;
    }

    /** A revision the caller expects: an integer of 1 or more. */
    static long expectedRevision(long value) {
        if (value < 1) {
            throw SagatException.invalid("expectedRevision is a positive integer");
        }
        return value;
    }

    /** A text the caller must give (a note, a reason): present, and not empty when {@code nonEmpty}. */
    static String text(String value, String what, boolean nonEmpty) {
        if (value == null || (nonEmpty && value.isEmpty())) {
            throw SagatException.invalid(what + (nonEmpty ? " is a non-empty text" : " is a text"));
        }
        return value;
    }

    /** An instant the caller gives, written in UTC with a {@code Z}, to the millisecond. */
    static String when(Instant value, String what) {
        if (value == null) {
            throw SagatException.invalid(what + " is not an instant");
        }
        return Json.instant(value);
    }

    /**
     * An instant the caller gives as RFC 3339 text, with any offset; written in UTC with a {@code Z}. Text without an
     * offset, or that is not an instant, is refused: the SDK never guesses a zone.
     */
    static String when(String text, String what) {
        if (text == null) {
            throw SagatException.invalid(what + " is not an instant");
        }
        try {
            return Json.instant(OffsetDateTime.parse(text).toInstant());
        } catch (DateTimeParseException e) {
            throw SagatException.invalid(what + " is not an instant");
        }
    }

    /** An instant given as a {@link Instant} or as RFC 3339 text (the only two things a builder keeps). */
    static String whenGiven(Object value, String what) {
        if (value instanceof Instant instant) {
            return when(instant, what);
        }
        if (value instanceof String text) {
            return when(text, what);
        }
        throw SagatException.invalid(what + " is not an instant");
    }

    /**
     * A start delay as the integer milliseconds that are sent: the duration truncated toward zero, converted once. The
     * range (1 ms to 366 days) is the server's to judge; a duration beyond what a 64-bit count of milliseconds holds
     * is sent as the extreme it comes closest to, which the server refuses.
     */
    static Long delayMs(Duration delay) {
        if (delay == null) {
            return null;
        }
        try {
            return delay.toMillis();
        } catch (ArithmeticException tooLarge) {
            return delay.isNegative() ? Long.MIN_VALUE : Long.MAX_VALUE;
        }
    }

    // ------------------------------------------------------------------------------------- what is read

    static JsonNode object(JsonNode value, String what) {
        if (value == null || !value.isObject()) {
            throw SagatException.of(ErrorCode.PROTOCOL, what + " is not an object");
        }
        return value;
    }

    static String str(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        if (!value.isTextual()) {
            throw SagatException.of(ErrorCode.PROTOCOL, key + " is missing from the answer");
        }
        return value.textValue();
    }

    static String optStr(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        return value.isTextual() ? value.textValue() : null;
    }

    /** An instant the answer may lack (or hold as something that is not an instant): null then. */
    static Instant optInstant(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        if (!value.isTextual()) {
            return null;
        }
        try {
            return OffsetDateTime.parse(value.textValue()).toInstant();
        } catch (DateTimeParseException e) {
            return null;
        }
    }

    /** An instant the answer must hold. */
    static Instant instant(JsonNode raw, String key) {
        Instant value = optInstant(raw, key);
        if (value == null) {
            throw SagatException.of(ErrorCode.PROTOCOL, key + " is missing from the answer");
        }
        return value;
    }

    /** A number of the answer, or {@code fallback} when it is not one. */
    static long num(JsonNode raw, String key, long fallback) {
        JsonNode value = raw.path(key);
        return value.isNumber() ? value.longValue() : fallback;
    }

    /** A number of the answer, or null when it is not one. */
    static Long optNum(JsonNode raw, String key) {
        JsonNode value = raw.path(key);
        return value.isNumber() ? Long.valueOf(value.longValue()) : null;
    }

    /** An integer of the answer that is at least {@code minimum}. */
    static long revision(JsonNode raw, String key, long minimum) {
        JsonNode value = raw.path(key);
        if (!value.isIntegralNumber() || !value.canConvertToLong() || value.longValue() < minimum) {
            throw SagatException.of(ErrorCode.PROTOCOL, key + " is invalid in the answer");
        }
        return value.longValue();
    }

    static List<String> strings(JsonNode value) {
        if (value == null || !value.isArray()) {
            return List.of();
        }
        List<String> out = new ArrayList<>(value.size());
        for (JsonNode item : value) {
            out.add(item.asText());
        }
        return List.copyOf(out);
    }

    /** The members of an array of the answer, as they came. */
    static List<JsonNode> array(JsonNode value) {
        if (value == null || !value.isArray()) {
            return List.of();
        }
        List<JsonNode> out = new ArrayList<>(value.size());
        value.forEach(out::add);
        return List.copyOf(out);
    }

    /** Named counts: the members that are numbers, in the order they came. */
    static Map<String, Long> counters(JsonNode value) {
        Map<String, Long> out = new LinkedHashMap<>();
        if (value != null && value.isObject()) {
            for (Map.Entry<String, JsonNode> entry : value.properties()) {
                if (entry.getValue().isNumber()) {
                    out.put(entry.getKey(), entry.getValue().longValue());
                }
            }
        }
        return Collections.unmodifiableMap(out);
    }

    static <T> Page<T> page(JsonNode body, String what, Function<JsonNode, T> read) {
        JsonNode raw = object(body, what);
        JsonNode items = raw.path("items");
        if (!items.isArray()) {
            throw SagatException.of(ErrorCode.PROTOCOL, "items is missing from the page");
        }
        List<T> out = new ArrayList<>(items.size());
        for (JsonNode item : items) {
            out.add(read.apply(item));
        }
        return new Page<>(List.copyOf(out), optStr(raw, "nextCursor"), raw);
    }

    /** Every item of a list, a page at a time: a page is read when the previous one has been consumed. */
    static <T> Stream<T> all(Function<String, Page<T>> fetch) {
        Iterator<T> iterator = new Iterator<>() {
            private Iterator<T> page = List.<T>of().iterator();
            private String cursor;
            private boolean done;

            private void fill() {
                while (!page.hasNext() && !done) {
                    Page<T> next = fetch.apply(cursor);
                    cursor = next.nextCursor();
                    page = next.items().iterator();
                    if (cursor == null || cursor.isEmpty()) {
                        done = true;
                    }
                }
            }

            @Override
            public boolean hasNext() {
                fill();
                return page.hasNext();
            }

            @Override
            public T next() {
                if (!hasNext()) {
                    throw new NoSuchElementException();
                }
                return page.next();
            }
        };
        return StreamSupport.stream(Spliterators.spliteratorUnknownSize(iterator, Spliterator.ORDERED), false);
    }
}
