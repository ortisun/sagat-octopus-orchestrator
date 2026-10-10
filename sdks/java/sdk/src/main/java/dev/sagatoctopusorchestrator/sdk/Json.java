package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.cfg.JsonNodeFeature;
import com.fasterxml.jackson.databind.json.JsonMapper;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.math.MathContext;
import java.math.RoundingMode;
import java.time.Instant;
import java.time.ZoneOffset;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Iterator;
import java.util.List;

/**
 * JSON as the SDK reads and writes it (sdks/SPEC.md sections 7 and 8).
 *
 * <p>Values travel as {@link JsonNode}s and are never converted through {@code double}: an integer stays an
 * integer ({@code 600000} is never {@code 600000.0}), a decimal is a {@link BigDecimal} that keeps its digits
 * and a string such as {@code "19.99"} is never turned into a number. The text the SDK writes is UTF-8 JSON
 * without whitespace; {@link #canonical(JsonNode)} also sorts the keys and is byte for byte what
 * {@code JSON.stringify} gives for the same sorted document.
 */
public final class Json {
    private static final ObjectMapper MAPPER = JsonMapper.builder()
            .enable(DeserializationFeature.USE_BIG_DECIMAL_FOR_FLOATS)
            .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
            .disable(JsonNodeFeature.STRIP_TRAILING_BIGDECIMAL_ZEROES)
            .build();

    private static final char[] HEX = "0123456789abcdef".toCharArray();

    private Json() {}

    /** The mapper the SDK uses to read answers and to turn application objects into JSON. */
    public static ObjectMapper mapper() {
        return MAPPER;
    }

    /** Parses JSON text; a failure is a {@code PROTOCOL} error (the message never repeats the text). */
    public static JsonNode parse(byte[] bytes) {
        try {
            return MAPPER.readTree(bytes);
        } catch (java.io.IOException e) {
            // The parser's message quotes the text it could not read: not kept, not even as a cause.
            throw SagatException.of(ErrorCode.PROTOCOL, "the answer is not JSON");
        }
    }

    /** Parses JSON text; a failure is a {@code VALIDATION_FAILED} error (the message never repeats the text). */
    public static JsonNode parse(String text) {
        try {
            return MAPPER.readTree(text);
        } catch (java.io.IOException e) {
            throw SagatException.of(ErrorCode.VALIDATION_FAILED, "the text is not JSON");
        }
    }

    /**
     * The JSON tree of an application value: a {@link JsonNode} as it is, {@code null} as JSON null, anything
     * else through the mapper (maps, lists, records and beans).
     */
    public static JsonNode toTree(Object value) {
        if (value == null) {
            return JsonNodeFactory.instance.nullNode();
        }
        if (value instanceof JsonNode node) {
            return node;
        }
        try {
            return MAPPER.valueToTree(value);
        } catch (IllegalArgumentException e) {
            // Jackson's message can quote the values it was writing: not kept.
            throw SagatException.of(ErrorCode.VALIDATION_FAILED, "a value of type " + value.getClass().getSimpleName() + " cannot be written as JSON");
        }
    }

    /**
     * The text of a value to send: insertion order of keys, no whitespace. {@code NaN} and the infinities are not
     * JSON and raise {@code VALIDATION_FAILED}.
     */
    public static String encode(JsonNode node) {
        StringBuilder out = new StringBuilder(128);
        write(node, out, false, "$");
        return out.toString();
    }

    /**
     * Canonical JSON (SPEC section 7, RFC 8785): UTF-8, object keys sorted at every depth by UTF-16 code unit, no
     * insignificant whitespace, strings escaped as {@code JSON.stringify} does and numbers as ECMAScript prints
     * them ({@code 1.0} is {@code 1}, {@code 600000} stays {@code 600000}, {@code 1e21} is {@code 1e+21}). A number is
     * taken as its nearest IEEE double, as the RFC defines; an integer beyond 2^53 - 1 is refused (send it as a
     * string) rather than rounded.
     */
    public static String canonical(JsonNode node) {
        StringBuilder out = new StringBuilder(256);
        write(node, out, true, "$");
        return out.toString();
    }

    private static final DateTimeFormatter INSTANT =
            DateTimeFormatter.ofPattern("uuuu-MM-dd'T'HH:mm:ss.SSS'Z'").withZone(ZoneOffset.UTC);

    /** An RFC 3339 instant in UTC with a {@code Z}, to the millisecond, as the SDK writes every instant. */
    public static String instant(Instant value) {
        return INSTANT.format(value);
    }

    private static void write(JsonNode node, StringBuilder out, boolean sort, String path) {
        if (node == null || node.isMissingNode() || node.isNull()) {
            out.append("null");
            return;
        }
        switch (node.getNodeType()) {
            case BOOLEAN -> out.append(node.booleanValue() ? "true" : "false");
            case STRING -> quote(node.textValue(), out, path);
            case NUMBER -> {
                if (sort) {
                    canonicalNumber(node, out, path);
                } else {
                    number(node, out, path);
                }
            }
            case BINARY -> {
                try {
                    quote(Base64.getEncoder().encodeToString(node.binaryValue()), out, path);
                } catch (java.io.IOException e) {
                    throw SagatException.invalid(path + " holds bytes that cannot be written");
                }
            }
            case POJO -> write(toTree(((com.fasterxml.jackson.databind.node.POJONode) node).getPojo()), out, sort, path);
            case ARRAY -> {
                out.append('[');
                int i = 0;
                for (JsonNode item : node) {
                    if (i > 0) {
                        out.append(',');
                    }
                    write(item, out, sort, path + "[" + i + "]");
                    i++;
                }
                out.append(']');
            }
            case OBJECT -> {
                out.append('{');
                List<String> names = new ArrayList<>(node.size());
                for (Iterator<String> it = node.fieldNames(); it.hasNext(); ) {
                    names.add(it.next());
                }
                if (sort) {
                    // String.compareTo orders by UTF-16 code unit, as JavaScript's default sort does.
                    names.sort(null);
                }
                int i = 0;
                for (String name : names) {
                    if (i++ > 0) {
                        out.append(',');
                    }
                    quote(name, out, path);
                    out.append(':');
                    write(node.get(name), out, sort, path + "." + name);
                }
                out.append('}');
            }
            default -> out.append("null");
        }
    }

    private static void number(JsonNode node, StringBuilder out, String path) {
        if (node.isIntegralNumber()) {
            out.append(node.bigIntegerValue().toString());
        } else if (node.isBigDecimal()) {
            out.append(decimal(node.decimalValue()));
        } else {
            double value = node.doubleValue();
            if (Double.isNaN(value) || Double.isInfinite(value)) {
                throw SagatException.invalid(path + " is not a finite number");
            }
            // The digits Double.toString gives, in plain notation ("10000000000", not "1.0E10"; 1.0 stays 1.0).
            BigDecimal exact = node.isFloat() ? new BigDecimal(Float.toString(node.floatValue())) : BigDecimal.valueOf(value);
            out.append(exact.toPlainString());
        }
    }

    /**
     * A decimal as it was given: its digits in plain notation when that is short, and in the exponent form the number
     * came in otherwise. An exponent is never expanded in memory: {@code 1e999999999} is a number of eighteen characters,
     * not a billion digits (SPEC section 8).
     */
    private static String decimal(BigDecimal value) {
        long adjusted = (long) value.precision() - value.scale() - 1;
        if (value.scale() >= -64 && value.scale() <= 400 && adjusted <= 400 && adjusted >= -400) {
            return value.toPlainString();
        }
        return value.toString();
    }

    private static final BigInteger MAX_SAFE = BigInteger.valueOf(9007199254740991L);

    private static void canonicalNumber(JsonNode node, StringBuilder out, String path) {
        if (node.isIntegralNumber()) {
            BigInteger value = node.bigIntegerValue();
            if (value.abs().compareTo(MAX_SAFE) > 0) {
                throw SagatException.invalid(path + " is an integer beyond 2^53 - 1: send it as a string");
            }
            out.append(value.toString());
            return;
        }
        double value;
        if (node.isBigDecimal()) {
            BigDecimal decimal = node.decimalValue();
            long adjusted = (long) decimal.precision() - decimal.scale() - 1;
            // Out of the range of a double before any arithmetic is done on a huge exponent: too big is not a number the
            // text can carry, too small is a double's zero (as JSON.parse reads it).
            if (adjusted > 400) {
                throw SagatException.invalid(path + " is not a finite number");
            }
            value = adjusted < -400 ? 0 : decimal.doubleValue();
        } else {
            value = node.doubleValue();
        }
        if (Double.isNaN(value) || Double.isInfinite(value)) {
            throw SagatException.invalid(path + " is not a finite number");
        }
        out.append(ecmaNumber(value));
    }

    /**
     * {@code Number.prototype.toString} of ECMAScript (what RFC 8785 requires): the shortest decimal digits that read
     * back as the same double, laid out in plain notation for exponents from -6 to 20 and as {@code d.ddde+x}
     * otherwise. The digits are found here, not by {@code Double.toString}, whose digits were not always the
     * shortest before Java 19.
     */
    static String ecmaNumber(double value) {
        if (value == 0) {
            return "0";
        }
        boolean negative = value < 0;
        double v = Math.abs(value);
        BigDecimal exact = new BigDecimal(v);
        BigDecimal best = null;
        for (int p = 1; p <= 17 && best == null; p++) {
            BigDecimal down = exact.round(new MathContext(p, RoundingMode.DOWN));
            BigDecimal up = exact.round(new MathContext(p, RoundingMode.UP));
            boolean downOk = down.doubleValue() == v;
            boolean upOk = up.doubleValue() == v;
            if (downOk && upOk) {
                int closer = exact.subtract(down).compareTo(up.subtract(exact));
                if (closer < 0) {
                    best = down;
                } else if (closer > 0) {
                    best = up;
                } else {
                    best = down.unscaledValue().testBit(0) ? up : down; // a tie: the even digit
                }
            } else if (downOk) {
                best = down;
            } else if (upOk) {
                best = up;
            }
        }
        BigDecimal digitsValue = best.stripTrailingZeros();
        String digits = digitsValue.unscaledValue().toString();
        int k = digits.length();
        int n = k - digitsValue.scale(); // the value is 0.<digits> * 10^n
        StringBuilder text = new StringBuilder(negative ? "-" : "");
        if (k <= n && n <= 21) {
            text.append(digits).append("0".repeat(n - k));
        } else if (0 < n && n <= 21) {
            text.append(digits, 0, n).append('.').append(digits, n, k);
        } else if (-6 < n && n <= 0) {
            text.append("0.").append("0".repeat(-n)).append(digits);
        } else {
            int e = n - 1;
            text.append(digits.charAt(0));
            if (k > 1) {
                text.append('.').append(digits, 1, k);
            }
            text.append('e').append(e < 0 ? '-' : '+').append(Math.abs(e));
        }
        return text.toString();
    }

    /**
     * A JSON string literal, escaped as {@code JSON.stringify} does (lower-case hex). A lone surrogate cannot be
     * written as UTF-8: it is a {@code VALIDATION_FAILED} error (SPEC section 8).
     */
    static void quote(String text, StringBuilder out, String path) {
        out.append('"');
        int n = text.length();
        for (int i = 0; i < n; i++) {
            char c = text.charAt(i);
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\b' -> out.append("\\b");
                case '\f' -> out.append("\\f");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (c < 0x20) {
                        unicode(c, out);
                    } else if (Character.isHighSurrogate(c)) {
                        if (i + 1 < n && Character.isLowSurrogate(text.charAt(i + 1))) {
                            out.append(c).append(text.charAt(++i));
                        } else {
                            throw SagatException.invalid(path + " holds text that is not valid Unicode (a lone surrogate)");
                        }
                    } else if (Character.isLowSurrogate(c)) {
                        throw SagatException.invalid(path + " holds text that is not valid Unicode (a lone surrogate)");
                    } else {
                        out.append(c);
                    }
                }
            }
        }
        out.append('"');
    }

    private static void unicode(char c, StringBuilder out) {
        out.append("\\u").append(HEX[(c >> 12) & 0xf]).append(HEX[(c >> 8) & 0xf]).append(HEX[(c >> 4) & 0xf]).append(HEX[c & 0xf]);
    }
}
