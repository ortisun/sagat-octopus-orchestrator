package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;
import org.junit.jupiter.api.Test;

/** The builders of the IR (mirrors workflow.test.ts). */
class WorkflowBuilderTest {
    private static JsonNode json(String text) {
        return Json.parse(text);
    }

    /** Equal as JSON documents: the numbers 100 and 100L are the same number, and the order of the keys is not compared. */
    private static void same(JsonNode expected, JsonNode actual) {
        assertEquals(Json.canonical(expected), Json.canonical(actual));
    }

    private static WorkflowBuilder workflow(String name) {
        return WorkflowBuilder.named(name);
    }

    @Test
    void aChainOfStepsIsLinkedInOrderAndStartsAtTheFirst() {
        JsonNode doc = workflow("order-flow")
                .description("charges and ships")
                .step("charge", "payments.charge", StepOptions.create().retry(RetryOptions.create().maxAttempts(3).strategy(RetryOptions.Strategy.EXPONENTIAL).initialDelayMs(100).maxDelayMs(1000).jitter(true)))
                .sleepMs("pause", 1500)
                .step("ship", "shipping.ship")
                .build();
        assertEquals("charge", doc.get("entrypoint").textValue());
        same(json("{\"name\":\"order-flow\",\"description\":\"charges and ships\"}"), doc.get("metadata"));
        same(json("[{\"id\":\"charge\",\"type\":\"activity\",\"activity\":\"payments.charge\",\"retry\":{\"maxAttempts\":3,\"strategy\":\"exponential\",\"initialDelayMs\":100,\"maxDelayMs\":1000,\"jitter\":true},\"next\":\"pause\"},"
                        + "{\"id\":\"pause\",\"type\":\"timer\",\"durationMs\":1500,\"next\":\"ship\"},"
                        + "{\"id\":\"ship\",\"type\":\"activity\",\"activity\":\"shipping.ship\"}]"),
                doc.get("nodes"));
    }

    @Test
    void absentOptionsAreLeftOutAndNeverNull() {
        JsonNode doc = workflow("plain-flow").step("a", "act.a", StepOptions.create()).build();
        same(json("[{\"id\":\"a\",\"type\":\"activity\",\"activity\":\"act.a\"}]"), doc.get("nodes"));
        assertFalse(doc.has("triggers") || doc.has("inputSchema") || doc.has("outputSchema"));
        same(json("{\"name\":\"plain-flow\"}"), doc.get("metadata"));
    }

    @Test
    void compensationReconciliationAndTheOtherStepOptionsAreWrittenAsTheSchemaNamesThem() {
        JsonNode doc = workflow("saga-flow")
                .step("reserve", "limit.reserve", StepOptions.create()
                        .compensateWith("limit.release")
                        .reconcileWith("limit.lookup")
                        .idempotencyTemplate("reserve:${run.id}")
                        .secrets(List.of("payments-key"))
                        .compensationRetry(RetryOptions.create().maxAttempts(5))
                        .compensationTimeoutMs(10000)
                        .highRisk(true)
                        .irreversible(false)
                        .timeoutMs(2500)
                        .onError("reserve"))
                .build();
        same(json("{\"id\":\"reserve\",\"type\":\"activity\",\"activity\":\"limit.reserve\",\"compensateWith\":\"limit.release\",\"reconcileWith\":\"limit.lookup\","
                        + "\"idempotencyTemplate\":\"reserve:${run.id}\",\"secrets\":[\"payments-key\"],\"compensationRetry\":{\"maxAttempts\":5},"
                        + "\"compensationTimeoutMs\":10000,\"highRisk\":true,\"irreversible\":false,\"timeoutMs\":2500,\"onError\":\"reserve\"}"),
                doc.get("nodes").get(0));
    }

    @Test
    void aParallelListsTheFirstNodeOfEachBranchAndContinuesWithTheNextNodeOfItsFlow() {
        JsonNode doc = workflow("fan-out")
                .parallel("split", Flow.Join.ALL,
                        Flow.create().step("card", "pay.card").step("card-2", "pay.card2"),
                        Flow.create().step("stock", "stock.hold"))
                .step("done", "order.done")
                .build();
        same(json("[{\"id\":\"split\",\"type\":\"parallel\",\"branches\":[\"card\",\"stock\"],\"join\":\"all\",\"next\":\"done\"},"
                        + "{\"id\":\"done\",\"type\":\"activity\",\"activity\":\"order.done\"},"
                        + "{\"id\":\"card\",\"type\":\"activity\",\"activity\":\"pay.card\",\"next\":\"card-2\"},"
                        + "{\"id\":\"card-2\",\"type\":\"activity\",\"activity\":\"pay.card2\"},"
                        + "{\"id\":\"stock\",\"type\":\"activity\",\"activity\":\"stock.hold\"}]"),
                doc.get("nodes"));
    }

    @Test
    void aSagaBeginsAtItsBodyHoldsNoEndAndContinuesWithTheNextNodeOfItsFlow() {
        JsonNode doc = workflow("saga-flow")
                .saga("tx",
                        SagaOptions.create().onCompensationFailure(SagaOptions.OnCompensationFailure.FAIL).cancelPolicy(SagaOptions.CancelPolicy.ABORT_NO_COMPENSATION),
                        Flow.create().step("reserve", "a.reserve", StepOptions.create().compensateWith("a.release")).step("submit", "a.submit"))
                .step("notify", "a.notify")
                .build();
        same(json("{\"id\":\"tx\",\"type\":\"saga\",\"begin\":\"reserve\",\"onCompensationFailure\":\"fail\",\"cancelPolicy\":\"abort_no_compensation\",\"next\":\"notify\"}"),
                doc.get("nodes").get(0));
        assertEquals(List.of("tx", "notify", "reserve", "submit"), doc.get("nodes").findValuesAsText("id"));
    }

    @Test
    void waitForAnEventInvokeAChildDecideAndEnd() {
        JsonNode doc = workflow("mixed-flow")
                .waitForEvent("paid", WaitForEventOptions.of("payment.confirmed", "input.orderId", 60000).onTimeout("cancel").lookbackMs(1000))
                .invokeChild("ship", "shipping-flow", ChildOptions.create().version(2).cancelPolicy(ChildOptions.CancelPolicy.DETACH).input("input.order"))
                .condition("big", "input.amount > 100", "review", "cancel")
                .step("review", "order.review", StepOptions.create().onError("cancel"))
                .step("cancel", "order.cancel")
                .end("done", "steps.ship.output")
                .build();
        JsonNode nodes = doc.get("nodes");
        assertFalse(nodes.get(2).has("next"), "a condition names both branches and has no next");
        same(json("{\"id\":\"paid\",\"type\":\"waitEvent\",\"eventType\":\"payment.confirmed\",\"correlationExpr\":\"input.orderId\",\"timeoutMs\":60000,\"onTimeout\":\"cancel\",\"lookbackMs\":1000,\"next\":\"ship\"}"),
                nodes.get(0));
        same(json("{\"id\":\"ship\",\"type\":\"child\",\"workflowId\":\"shipping-flow\",\"version\":2,\"cancelPolicy\":\"detach\",\"input\":\"input.order\",\"next\":\"big\"}"),
                nodes.get(1));
        same(json("{\"id\":\"done\",\"type\":\"end\",\"outputExpr\":\"steps.ship.output\"}"), nodes.get(5));
        assertEquals("done", nodes.get(4).get("next").textValue(), "the node before an end links to it");
    }

    @Test
    void theNodeBeforeAnEndLinksToItAndTheEndNeverLinksOn() {
        JsonNode doc = workflow("ending-flow").step("a", "act.a").end("done").step("b", "act.b").build();
        assertEquals("done", doc.get("nodes").get(0).get("next").textValue());
        assertFalse(doc.get("nodes").get(1).has("next"));
        assertFalse(doc.get("nodes").get(2).has("next"));
    }

    @Test
    void triggersLabelsAndSchemasGoWhereTheDocumentKeepsThem() {
        JsonNode doc = workflow("with-meta")
                .label("team", "payments")
                .trigger(Trigger.event("order.created").source("api"))
                .trigger(Trigger.manual())
                .trigger(Trigger.cron("0 3 * * *").timezone("Europe/Lisbon"))
                .trigger(Trigger.webhook("hooks").filter("data.amount > 1"))
                .trigger(Trigger.message("kafka-main").eventType("order.*"))
                .inputSchema(Map.of("type", "object"))
                .outputSchema(Map.of("type", "object"))
                .step("a", "act.a")
                .build();
        same(json("{\"name\":\"with-meta\",\"labels\":{\"team\":\"payments\"}}"), doc.get("metadata"));
        same(json("[{\"kind\":\"event\",\"eventType\":\"order.created\",\"source\":\"api\"},{\"kind\":\"manual\"},"
                        + "{\"kind\":\"cron\",\"cron\":\"0 3 * * *\",\"timezone\":\"Europe/Lisbon\"},"
                        + "{\"kind\":\"webhook\",\"endpoint\":\"hooks\",\"filter\":\"data.amount > 1\"},"
                        + "{\"kind\":\"message\",\"connector\":\"kafka-main\",\"eventType\":\"order.*\"}]"),
                doc.get("triggers"));
        same(json("{\"type\":\"object\"}"), doc.get("inputSchema"));
        same(json("{\"type\":\"object\"}"), doc.get("outputSchema"));
        same(json("{\"kind\":\"event\",\"eventType\":\"x\"}"), Trigger.fromJson(json("{\"kind\":\"event\",\"eventType\":\"x\"}")).toJson());
    }

    private static void refused(Runnable build, String mentions) {
        SagatException error = assertThrows(SagatException.class, build::run);
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(Pattern.compile(mentions).matcher(error.getMessage()).find(), error.getMessage());
    }

    @Test
    void whatTheServerWouldRefuseAndTheBuilderCanSeeIsRefusedHere() {
        refused(() -> workflow("Bad Name"), "workflow name");
        refused(() -> workflow("ab"), "workflow name");
        refused(() -> workflow("empty-flow").build(), "at least one node");
        refused(() -> workflow("dup-ids").step("a", "x").step("a", "y").build(), "used twice");
        refused(() -> workflow("dup-nested").step("a", "x")
                .parallel("p", Flow.Join.ALL, Flow.create().step("a", "y"), Flow.create().step("b", "z")).build(), "used twice");
        refused(() -> workflow("one-branch").parallel("p", Flow.Join.ALL, Flow.create().step("a", "x")), "two branches");
        refused(() -> workflow("empty-branch").parallel("p", Flow.Join.ALL, Flow.create().step("a", "x"), Flow.create()), "empty branch");
        refused(() -> workflow("empty-saga").saga("s", SagaOptions.create(), Flow.create()), "empty body");
        refused(() -> workflow("saga-end").saga("s", SagaOptions.create(), Flow.create().step("a", "x").end("e")), "end node");
        refused(() -> workflow("both").step("a", "x", StepOptions.create().irreversible(true).compensateWith("undo")), "irreversible");
        refused(() -> workflow("bad-secret").step("a", "x", StepOptions.create().secrets(List.of("Bad Secret"))), "secret");
        refused(() -> workflow("bad-timer").sleepMs("t", 0), "duration");
        refused(() -> workflow("bad-timer-2").sleep("t", Duration.ofNanos(1500)), "whole number of milliseconds");
        refused(() -> workflow("dangling").condition("c", "true", "nowhere", "a").step("a", "x").build(), "not a node");
        refused(() -> workflow("dangling-timeout").waitForEvent("w", WaitForEventOptions.of("e", "x", 1).onTimeout("nowhere")).build(), "not a node");
        refused(() -> workflow("no-activity").step("a", ""), "activity name");
        refused(() -> workflow("no-id").step("", "x"), "needs an id");
        refused(() -> workflow("bad-wait").waitForEvent("w", WaitForEventOptions.of("e", "x", 0)), "timeout");
        refused(() -> workflow("bad-schema").inputSchema(List.of(1)), "JSON object");
    }

    @Test
    void theCanonicalTextSortsKeysAtEveryDepthAndLeavesOutWhitespace() {
        String text = workflow("canon-flow").step("a", "act.a", StepOptions.create().retry(RetryOptions.create().maxAttempts(2))).toJson();
        assertTrue(text.startsWith("{\"apiVersion\":"), text);
        assertFalse(text.contains(" "));
        assertEquals(
                "{\"apiVersion\":\"sagat-octopus-orchestrator.io/v1\",\"entrypoint\":\"a\",\"kind\":\"Workflow\",\"metadata\":{\"name\":\"canon-flow\"},"
                        + "\"nodes\":[{\"activity\":\"act.a\",\"id\":\"a\",\"retry\":{\"maxAttempts\":2},\"type\":\"activity\"}]}",
                text);
    }

    @Test
    void buildingTwiceGivesTheSameDocumentAndHasNoSideEffect() {
        WorkflowBuilder builder = workflow("twice-flow").step("a", "act.a").step("b", "act.b");
        assertEquals(builder.toJson(), builder.toJson());
        builder.build().put("tampered", true);
        assertFalse(builder.toJson().contains("tampered"), "the document handed out is a copy");
    }

    @Test
    void aFlowBuiltApartIsAppendedInOrder() {
        Flow tail = Flow.create().step("c", "act.c").step("d", "act.d");
        JsonNode doc = workflow("append-flow").step("a", "act.a").append(tail).build();
        assertEquals(List.of("a", "c", "d"), doc.get("nodes").findValuesAsText("id"));
        assertEquals("c", doc.get("nodes").get(0).get("next").textValue());
        assertEquals("act.c", tail.flatten().get(0).get("activity").textValue(), "the flow handed in is not changed");
    }

    @Test
    void aDuplicateNodeIdIsRaisedWhenTheNodeIsAddedNotWhenTheDocumentIsBuilt() {
        WorkflowBuilder builder = workflow("dup-at-add").step("a", "x");
        SagatException error = assertThrows(SagatException.class, () -> builder.step("a", "y"));
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(error.getMessage().contains("used twice"), error.getMessage());
        // every primitive checks it
        assertThrows(SagatException.class, () -> builder.sleepMs("a", 5));
        assertThrows(SagatException.class, () -> builder.end("a"));
        assertThrows(SagatException.class, () -> builder.condition("a", "true", "a", "a"));
        assertThrows(SagatException.class, () -> builder.waitForEvent("a", WaitForEventOptions.of("e", "x", 1)));
        assertThrows(SagatException.class, () -> builder.invokeChild("a", "child-flow"));
        Flow flow = Flow.create().step("p", "x");
        assertThrows(SagatException.class, () -> flow.step("p", "y"));
        // the failed add did not leave a node behind
        assertEquals(List.of("a"), builder.build().get("nodes").findValuesAsText("id"));
    }

    @Test
    void aNodeIdIsUniqueAcrossNestedFlowsAtTheMomentTheyAreJoined() {
        Flow branchOne = Flow.create().step("card", "pay.card");
        Flow branchTwo = Flow.create().step("a", "x");
        WorkflowBuilder builder = workflow("nested-dups").step("a", "act.a");
        // a branch that holds an id the workflow already has
        SagatException error = assertThrows(SagatException.class, () -> builder.parallel("split", Flow.Join.ALL, branchOne, branchTwo));
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(error.getMessage().contains("used twice"), error.getMessage());
        // two branches that hold the same id
        assertThrows(SagatException.class, () -> workflow("two-branches").parallel("split", Flow.Join.ALL, Flow.create().step("same", "x"), Flow.create().step("same", "y")));
        // the id of the parallel node itself against its branches
        assertThrows(SagatException.class, () -> workflow("self-clash").parallel("same", Flow.Join.ALL, Flow.create().step("same", "x"), Flow.create().step("b", "y")));
        // a saga body against the workflow
        assertThrows(SagatException.class, () -> workflow("saga-clash").step("a", "x").saga("tx", SagaOptions.create(), Flow.create().step("a", "y")));
    }

    @Test
    void joiningAFlowTakesACopyOfItAndTheWorkflowIsCheckedAgainstTheIdsItNowHolds() {
        Flow branch = Flow.create().step("card", "pay.card");
        WorkflowBuilder builder = workflow("late-add").step("first", "x").parallel("split", Flow.Join.ALL, branch, Flow.create().step("stock", "y"));
        assertThrows(SagatException.class, () -> builder.step("card", "z"), "the workflow cannot take a branch's id");
        assertThrows(SagatException.class, () -> builder.step("stock", "z"));
        assertThrows(SagatException.class, () -> builder.step("split", "z"));
        // The branch handed in is the caller's own: changing it afterwards changes nothing in the workflow.
        branch.step("first", "z");
        assertEquals(List.of("first", "split", "card", "stock"), builder.build().get("nodes").findValuesAsText("id"));
        // ... and it can be joined again, to another workflow, as it is now.
        WorkflowBuilder other = workflow("second-use").parallel("fan", Flow.Join.ALL, branch, Flow.create().step("other", "w"));
        assertEquals(List.of("fan", "card", "first", "other"), other.build().get("nodes").findValuesAsText("id"));
    }

    @Test
    void aFlowAppendedToAWorkflowIsCheckedWhenItIsAppended() {
        WorkflowBuilder builder = workflow("appended").step("a", "x");
        assertThrows(SagatException.class, () -> builder.append(Flow.create().step("b", "y").step("a", "z")));
        // a flow can be appended to several workflows: each takes a copy
        Flow shared = Flow.create().step("s", "x");
        workflow("one").append(shared);
        WorkflowBuilder other = workflow("two").step("t", "x");
        other.append(shared);
        assertEquals(List.of("t", "s"), other.build().get("nodes").findValuesAsText("id"));
    }
}
