package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.junit.jupiter.api.Test;

/** The operator factory of a collection and the {@code Operators} overloads of forEach and map (design 042). */
class CollectionOperatorTest {
    private static Flow line() {
        return Flow.body().step("price", "line.price").returnValue("return", "steps.price.output");
    }

    @Test
    void theFactoryWritesOneStagePerOperatorKeyedByItsName() {
        assertEquals("{\"filter\":{\"where\":\"element.total > 0\"}}", Ops.filter("element.total > 0").toJson());
        assertEquals("{\"map\":{\"select\":\"element.lines\"}}", Ops.map("element.lines").toJson());
        assertEquals("{\"flatMap\":{\"select\":\"element\"}}", Ops.flatMap("element").toJson());
        assertEquals("{\"skip\":{\"count\":0}}", Ops.skip(0).toJson());
        assertEquals("{\"take\":{\"count\":1000}}", Ops.take(1000).toJson());
        assertEquals("{\"distinct\":{\"by\":\"element.sku\"}}", Ops.distinct("element.sku").toJson());
        assertEquals("{\"distinct\":{}}", Ops.distinct().toJson());
        assertEquals("{\"window\":{\"count\":25}}", Ops.window(25).toJson());
        assertEquals("flatMap", Ops.flatMap("element").name());
    }

    @Test
    void theFactoryOnlyAssemblesAndLeavesRangesToTheServer() {
        assertEquals("{\"window\":{\"count\":0}}", Ops.window(0).toJson());
        assertEquals("{\"take\":{\"count\":1001}}", Ops.take(1001).toJson());
    }

    @Test
    void forEachAndMapCarryTheOperatorsInTheOrderGivenAsACopy() {
        Operator shared = Ops.filter("element.ok");
        ObjectNode document = WorkflowBuilder.named("chained-flow")
                .body("line", line())
                .forEach("lines", "input.lines", "line", "bodyResult", CollectionOptions.of(9, 2),
                        Operators.of(shared, Ops.distinct(), Ops.window(2)))
                .map("prices", "input.lines", "line", "bodyResult", CollectionOptions.of(9, 2), Operators.of(Ops.take(3)))
                .end("done")
                .build();
        JsonNode nodes = document.get("nodes");
        assertEquals("[{\"filter\":{\"where\":\"element.ok\"}},{\"distinct\":{}},{\"window\":{\"count\":2}}]",
                Json.canonical(nodes.get(0).get("operators")));
        assertEquals("[{\"take\":{\"count\":3}}]", Json.canonical(nodes.get(1).get("operators")));
        assertEquals("forEach", nodes.get(1).get("type").asText());
        ((ObjectNode) nodes.get(0).get("operators").get(0).get("filter")).put("where", "true");
        assertEquals("{\"filter\":{\"where\":\"element.ok\"}}", shared.toJson());

        List<Operator> list = new ArrayList<>(List.of(Ops.take(1)));
        Operators chain = Operators.of(list);
        list.add(Ops.skip(1));
        assertEquals(1, chain.stages().size());
    }

    @Test
    void everyPresentCollectionCallStillBuildsWithoutOperators() {
        ObjectNode document = WorkflowBuilder.named("plain-flow")
                .body("line", line())
                .forEach("lines", "input.lines", "line", "bodyResult", CollectionOptions.of(17, 3))
                .map("prices", "input.lines", "line", "bodyResult", new CollectionOptions(17, 3))
                .forEach("empty", "input.lines", "line", "bodyResult", CollectionOptions.of(17, 3), Operators.of())
                .end("done")
                .build();
        for (int i = 0; i < 3; i++) {
            assertFalse(document.get("nodes").get(i).has("operators"));
        }
        assertEquals(2, CollectionOptions.class.getRecordComponents().length);
        Flow flow = Flow.body().forEach("lines", "input.lines", "line", "bodyResult", CollectionOptions.of(2, 1), null);
        assertTrue(flow != null);
    }

    @Test
    void aNullStageIsAValidationFailure() {
        SagatException error = assertThrows(SagatException.class, () -> Operators.of(Arrays.asList(Ops.take(1), null)));
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(error.getMessage().contains("operator 1"), error.getMessage());
    }
}
