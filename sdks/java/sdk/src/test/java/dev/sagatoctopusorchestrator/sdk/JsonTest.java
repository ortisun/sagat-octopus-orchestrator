package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.time.Instant;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

/** Canonical JSON and the way numbers and strings survive (SPEC sections 7 and 8). */
class JsonTest {
    @Test
    void theCanonicalTextSortsKeysAtEveryDepthAndLeavesOutWhitespace() {
        JsonNode node = Json.parse("{\"b\":1,\"a\":{\"d\":[2,{\"z\":1,\"y\":2}],\"c\":null},\"e\":[]}");
        assertEquals("{\"a\":{\"c\":null,\"d\":[2,{\"y\":2,\"z\":1}]},\"b\":1,\"e\":[]}", Json.canonical(node));
        assertEquals("{\"b\":1,\"a\":{\"d\":[2,{\"z\":1,\"y\":2}],\"c\":null},\"e\":[]}", Json.encode(node), "encode keeps the order it was given");
    }

    @Test
    void keysAreOrderedByUtf16CodeUnitLikeJavaScript() {
        Map<String, Object> map = new LinkedHashMap<>();
        map.put("b", 1);
        map.put("B", 2);
        map.put("a", 3);
        map.put("\uD83D\uDE00", 4);
        map.put("\uFF5E", 5);
        // U+FF5E (a BMP code unit above the surrogates) sorts after the surrogate pair, as JavaScript's default sort does.
        assertEquals("{\"B\":2,\"a\":3,\"b\":1,\"\uD83D\uDE00\":4,\"\uFF5E\":5}", Json.canonical(Json.toTree(map)));
    }

    @Test
    void integersStayIntegersAndDecimalsKeepTheirDigits() {
        String text = "{\"a\":600000,\"b\":9007199254740993,\"c\":9223372036854775807,\"d\":123456789012345678901234567890,\"e\":0.5,\"f\":0.1,\"g\":1.50,\"h\":-0,\"i\":1e3,\"j\":\"19.99\"}";
        assertEquals(
                "{\"a\":600000,\"b\":9007199254740993,\"c\":9223372036854775807,\"d\":123456789012345678901234567890,\"e\":0.5,\"f\":0.1,\"g\":1.50,\"h\":0,\"i\":1000,\"j\":\"19.99\"}",
                Json.encode(Json.parse(text)));
    }

    @Test
    void numbersBuiltInJavaAreWrittenInPlainNotation() {
        assertEquals("10000000000", Json.encode(Json.toTree(1.0E10)));
        assertEquals("0.5", Json.encode(Json.toTree(0.5)));
        assertEquals("0.1", Json.encode(Json.toTree(0.1)));
        assertEquals("1.0", Json.encode(Json.toTree(1.0)), "a double keeps the digits it was written with");
        assertEquals("1.0", Json.encode(JsonNodeFactory.instance.numberNode(1.0)));
        assertEquals("100.0", Json.encode(JsonNodeFactory.instance.numberNode(100.0)));
        assertEquals("0.00000010", Json.encode(JsonNodeFactory.instance.numberNode(1.0E-7)), "plain notation, never an exponent");
        assertEquals("1.5", Json.encode(Json.toTree(1.5f)));
        assertEquals("19.99", Json.encode(Json.toTree(new BigDecimal("19.99"))));
        assertEquals("100", Json.encode(Json.toTree(new BigDecimal("1E+2"))));
        assertEquals("123456789012345678901234567890", Json.encode(Json.toTree(new BigInteger("123456789012345678901234567890"))));
        assertEquals("[1,\"two\",null,true]", Json.encode(Json.toTree(java.util.Arrays.asList(1, "two", null, true))));
    }

    @Test
    void stringsAreEscapedLikeJsonStringify() {
        String text = "quote \" backslash \\ tab \t newline \n bell \u0007 unit sep \u001f del \u007f \u2028 ação € \uD83D\uDE00";
        JsonNode node = JsonNodeFactory.instance.textNode(text);
        assertEquals("\"quote \\\" backslash \\\\ tab \\t newline \\n bell \\u0007 unit sep \\u001f del \u007f \u2028 ação € \uD83D\uDE00\"", Json.encode(node));
        // A lone surrogate is not text that UTF-8 can carry: an error on the way out (SPEC section 8).
        SagatException lone = assertThrows(SagatException.class, () -> Json.encode(Json.toTree(Map.of("k", "\uD800"))));
        assertEquals(ErrorCode.VALIDATION_FAILED, lone.code());
        assertTrue(lone.getMessage().contains("$.k"), lone.getMessage());
        assertThrows(SagatException.class, () -> Json.encode(JsonNodeFactory.instance.textNode("x\uDC00")));
        assertThrows(SagatException.class, () -> Json.encode(Json.toTree(Map.of("\uD800", 1))));
    }

    @Test
    void notANumberAndTheInfinitiesAreNotJson() {
        SagatException nan = assertThrows(SagatException.class, () -> Json.encode(Json.toTree(Map.of("x", List.of(Double.NaN)))));
        assertEquals(ErrorCode.VALIDATION_FAILED, nan.code());
        assertEquals(true, nan.getMessage().contains("$.x[0]"), nan.getMessage());
        assertThrows(SagatException.class, () -> Json.encode(Json.toTree(Float.NEGATIVE_INFINITY)));
    }

    @Test
    void aBeanARecordAndAMapAreConvertedByTheMapper() {
        record Order(String id, BigDecimal amount, List<String> tags) {}
        assertEquals(
                "{\"id\":\"o-1\",\"amount\":19.99,\"tags\":[\"a\",\"b\"]}", Json.encode(Json.toTree(new Order("o-1", new BigDecimal("19.99"), List.of("a", "b")))));
        assertEquals("null", Json.encode(Json.toTree(null)));
        assertThrows(SagatException.class, () -> Json.toTree(new Object() {
            public Object getSelf() {
                return this;
            }
        }));
    }

    @Test
    void anInstantIsWrittenInUtcToTheMillisecondWithAZ() {
        assertEquals("2030-01-01T00:00:00.000Z", Json.instant(Instant.parse("2030-01-01T00:00:00Z")));
        assertEquals("2030-01-01T00:00:00.123Z", Json.instant(Instant.parse("2030-01-01T00:00:00.123456Z")));
    }

    @Test
    void parsingNeverRepeatsTheText() {
        SagatException error = assertThrows(SagatException.class, () -> Json.parse("{\"secret\": sk_live_123"));
        assertEquals(false, error.getMessage().contains("sk_live"));
    }
}
