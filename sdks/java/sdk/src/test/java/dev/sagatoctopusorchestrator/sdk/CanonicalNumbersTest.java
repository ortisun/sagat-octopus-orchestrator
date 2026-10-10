package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.LinkedHashMap;
import java.util.Map;
import org.junit.jupiter.api.Test;

/**
 * Canonical JSON is RFC 8785 (SPEC section 7): numbers as ECMAScript prints them. The table is the output of
 * {@code JSON.stringify} in Node.js for these IEEE bit patterns (it includes the examples of RFC 8785, appendix B).
 */
class CanonicalNumbersTest {
    private static final String[][] VECTORS = {
        {"0000000000000000", "0"},
        {"8000000000000000", "0"},
        {"0000000000000001", "5e-324"},
        {"8000000000000001", "-5e-324"},
        {"7fefffffffffffff", "1.7976931348623157e+308"},
        {"ffefffffffffffff", "-1.7976931348623157e+308"},
        {"4340000000000000", "9007199254740992"},
        {"c340000000000000", "-9007199254740992"},
        {"4430000000000000", "295147905179352830000"},
        {"44b52d02c7e14af5", "9.999999999999997e+22"},
        {"44b52d02c7e14af6", "1e+23"},
        {"44b52d02c7e14af7", "1.0000000000000001e+23"},
        {"444b1ae4d6e2ef4e", "999999999999999700000"},
        {"444b1ae4d6e2ef4f", "999999999999999900000"},
        {"444b1ae4d6e2ef50", "1e+21"},
        {"3eb0c6f7a0b5ed8c", "9.999999999999997e-7"},
        {"3eb0c6f7a0b5ed8d", "0.000001"},
        {"41b3de4355555553", "333333333.3333332"},
        {"41b3de4355555554", "333333333.33333325"},
        {"41b3de4355555555", "333333333.3333333"},
        {"41b3de4355555556", "333333333.3333334"},
        {"41b3de4355555557", "333333333.33333343"},
        {"becbf647612f3696", "-0.0000033333333333333333"},
        {"43143ff3c1cb0959", "1424953923781206.2"},
        {"3ff0000000000000", "1"},
        {"3fb999999999999a", "0.1"},
        {"4059000000000000", "100"},
        {"4415af1d78b58c40", "100000000000000000000"},
        {"3e7ad7f29abcaf48", "1e-7"},
        {"0010000000000000", "2.2250738585072014e-308"},
        {"000fffffffffffff", "2.225073858507201e-308"},
        {"3fe0000000000000", "0.5"},
        {"4024000000000000", "10"},
        {"40c3880000000000", "10000"},
        {"bff8000000000000", "-1.5"},
        {"7fe0000000000000", "8.98846567431158e+307"},
        {"3f50624dd2f1a9fc", "0.001"},
        {"fc7919286680f900", "-3.913448779440473e+291"},
        {"e6bbaa6fe49a464f", "-7.523502831915282e+186"},
        {"c8ae76200f620e32", "-1.3267822562092985e+42"},
        {"041a5dd66bc8ce19", "6.763905458681612e-289"},
        {"4ba6c11ce87f22b4", "2.7896748684220982e+56"},
        {"fb5c7edb8abbe6f3", "-1.6949338398152967e+286"},
        {"3f386e8e2170a706", "0.0003728005407902388"},
        {"e3556946b98ed75d", "-3.2322079185713746e+170"},
        {"8e50fa67677b7fa8", "-1.018482517949405e-239"},
        {"bfec4151993949d7", "-0.8829734795238505"},
        {"116988e9203e251a", "8.623147822868008e-225"},
        {"f743b839c6ddece1", "-3.1792521766959343e+266"},
        {"49f64059afe2c3dc", "2.032525719921841e+48"},
        {"f8cc1851b07a12fb", "-7.599325692445786e+273"},
        {"20ddcf361c5e5c6e", "2.2766469836812832e-150"},
        {"8b365dc737de52a5", "-1.1916761075101576e-254"},
        {"d5b21c2b2a84e350", "-6.489896428115427e+104"},
        {"c46141116b18265f", "-2.5462663655327914e+21"},
        {"87345ad54efa6102", "-5.879104282403329e-274"},
        {"eafd870d1db48ca9", "-2.3699782359792967e+207"},
        {"90506c16cc491204", "-4.231119225253858e-230"},
        {"a6fa5d6dcc83a803", "-6.381294871902019e-121"},
        {"3c3ae349e8a486d6", "1.4575930798816027e-18"},
        {"0b5c054e7dc55eed", "5.9717703354743415e-254"},
        {"d902ba679921c3f8", "-6.045093213550762e+120"},
        {"2eeb9e1951e6fbe7", "1.1373044150551584e-82"},
        {"69b58768256c61ea", "1.647933282093069e+201"},
        {"be3f26a6c539cd71", "-7.252903239465689e-9"},
        {"76a3c7266e40ad2c", "3.1139172131902916e+263"},
        {"720d4dbc494ac60b", "2.4424648701803993e+241"},
        {"76a7b5337a32c63e", "3.732654429850808e+263"},
        {"cb2002786cc31c35", "-7.667099154113939e+53"},
        {"14345bdf078ac1a0", "2.4190049804021394e-211"},
        {"216a64cf52f9ea6f", "1.0320812071079943e-147"},
        {"13888cabef3dc7d2", "1.4242811456147962e-214"},
        {"85aa5f99509ecf39", "-2.2701822850589932e-281"},
        {"f3062b69744c3554", "-1.2109993173501528e+246"},
        {"42592dae31258d13", "432562291862.2043"},
        {"e1689692fc0cbaa6", "-1.728439118184315e+161"},
        {"6b24605f80daaa7d", "1.308385429777836e+208"},
        {"618cd3106f2c7c48", "8.104925220678921e+161"},
        {"d41c1efb49c911f7", "-1.5016534529252984e+97"},
        {"271dcf0d786c32ba", "2.8859328782398127e-120"},
        {"88c9f8558dd8b201", "-2.5169044174519473e-266"},
        {"64b1b60c4d624a7c", "1.121407925935741e+177"},
        {"0736162a7ece1d1b", "6.379292911687223e-274"},
        {"ed6b4e3976ca040e", "-1.204863981596347e+219"},
        {"2834e8c5951329c5", "5.306649809347139e-115"},
        {"ef90fb5d792793f0", "-2.574677664963496e+229"},
        {"7e7a0f2c4070927f", "1.7451706879067903e+301"},
        {"767dd3941bc942a2", "5.870001608105236e+262"},
        {"ecb330b6922095c9", "-4.134646224789101e+215"},
        {"5d6b3fd8ab6d8ca4", "1.0384005679897717e+142"},
        {"9cbe86f7fde29623", "-3.1597283112723458e-170"},
        {"752f05a4c7164276", "2.9112316136247506e+256"},
        {"02b0daa7b7f7ba0d", "1.0308379337467599e-295"},
        {"21cdd5f258d0a898", "7.466701125650979e-146"},
        {"b039775f58308c07", "-2.1993124711032423e-76"},
        {"d53acb10217a978a", "-3.7506226354519107e+102"},
        {"d2123a89d6739a91", "-2.2663847598661066e+87"}
    };

    @Test
    void everyDoublePrintsAsEcmaScriptPrintsIt() {
        for (String[] vector : VECTORS) {
            double value = Double.longBitsToDouble(Long.parseUnsignedLong(vector[0], 16));
            assertEquals(vector[1], Json.ecmaNumber(value), "bits 0x" + vector[0]);
            assertEquals(vector[1], Json.canonical(JsonNodeFactory.instance.numberNode(value)), "as a node, bits 0x" + vector[0]);
        }
    }

    @Test
    void aWholeValueWrittenWithAFractionIsTheIntegerAndAnIntegerStaysAsItIs() {
        assertEquals("1", Json.canonical(Json.parse("1.0")));
        assertEquals("1", Json.canonical(Json.parse("1.000")));
        assertEquals("1.5", Json.canonical(Json.parse("1.50")));
        assertEquals("600000", Json.canonical(Json.parse("600000")));
        assertEquals("600000", Json.canonical(Json.parse("6e5")));
        assertEquals("0.1", Json.canonical(Json.parse("0.1")));
        assertEquals("0.5", Json.canonical(Json.parse("0.5")));
        assertEquals("-3", Json.canonical(Json.parse("-3.0")));
        assertEquals("0", Json.canonical(Json.parse("-0")));
        assertEquals("0", Json.canonical(Json.parse("-0.0")));
        assertEquals("1e+21", Json.canonical(Json.parse("1e21")));
        assertEquals("1e-7", Json.canonical(Json.parse("0.0000001")));
        assertEquals("9007199254740991", Json.canonical(Json.parse("9007199254740991")));
        assertEquals("-9007199254740991", Json.canonical(Json.parse("-9007199254740991")));
        assertEquals("[1,2.5,{\"a\":1e+30}]", Json.canonical(Json.parse("[1.0,2.50,{\"a\":1E30}]")));
    }

    @Test
    void whatTheWireKeepsTheCanonicalTextNormalises() {
        JsonNode node = Json.parse("{\"n\":1.0,\"m\":1.50}");
        assertEquals("{\"n\":1.0,\"m\":1.50}", Json.encode(node), "the wire text keeps the digits it was given");
        assertEquals("{\"m\":1.5,\"n\":1}", Json.canonical(node));
    }

    @Test
    void anIntegerBeyondTwoToThe53MinusOneIsRefusedNotRounded() {
        SagatException big = assertThrows(SagatException.class, () -> Json.canonical(Json.parse("{\"id\":9007199254740993}")));
        assertEquals(ErrorCode.VALIDATION_FAILED, big.code());
        assertTrue(big.getMessage().contains("$.id"), big.getMessage());
        assertThrows(SagatException.class, () -> Json.canonical(JsonNodeFactory.instance.numberNode(9223372036854775807L)));
        assertThrows(SagatException.class, () -> Json.canonical(JsonNodeFactory.instance.numberNode(new BigInteger("123456789012345678901234567890"))));
        assertThrows(SagatException.class, () -> Json.canonical(JsonNodeFactory.instance.numberNode(new BigDecimal("1e999"))));
        assertThrows(SagatException.class, () -> Json.canonical(JsonNodeFactory.instance.numberNode(Double.NaN)));
        assertThrows(SagatException.class, () -> Json.canonical(JsonNodeFactory.instance.numberNode(Double.POSITIVE_INFINITY)));
    }

    @Test
    void keysAreSortedByUtf16CodeUnitAtEveryDepth() {
        Map<String, Object> inner = new LinkedHashMap<>();
        inner.put("b", 1);
        inner.put("a", 2);
        Map<String, Object> outer = new LinkedHashMap<>();
        outer.put("\uD83D\uDE00", 1); // U+1F600, code units D83D DE00
        outer.put("\uFF5E", 2); // U+FF5E, code unit FF5E: sorts after the surrogate pair
        outer.put("z", inner);
        outer.put("\u00e9", 3);
        outer.put("B", 4);
        assertEquals("{\"B\":4,\"z\":{\"a\":2,\"b\":1},\"\u00e9\":3,\"\uD83D\uDE00\":1,\"\uFF5E\":2}", Json.canonical(Json.toTree(outer)));
    }
}
