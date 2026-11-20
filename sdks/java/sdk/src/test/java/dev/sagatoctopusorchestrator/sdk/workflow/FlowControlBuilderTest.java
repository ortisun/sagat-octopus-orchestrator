package dev.sagatoctopusorchestrator.sdk.workflow;

import static org.junit.jupiter.api.Assertions.*;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

/**
 * Feature 022 (FR-022-035, FR-022-050): {@code concurrency} and {@code rateLimit} at the workflow level and on activity
 * and script steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication.
 */
class FlowControlBuilderTest {
    private static final String DIGEST = "sha256:" + "0".repeat(64);

    private static WorkflowBuilder providerSync() {
        return WorkflowBuilder.named("provider-sync")
            .concurrency(ConcurrencyPolicy.of(1).key("input.accountId"))
            .rateLimit(RateLimitPolicy.of(100, RatePeriod.MINUTE).group("provider-starts").key("input.providerId").queueTimeoutMs(900000))
            .step("fetch", "provider.fetch", StepOptions.create().rateLimit(RateLimitPolicy.of(10, RatePeriod.SECOND).key("input.providerId")))
            .end("done");
    }

    private static WorkflowBuilder accountSerial() {
        return WorkflowBuilder.named("account-serial")
            .concurrency(ConcurrencyPolicy.of(1).group("orders-by-account").key("input.accountId").queueTimeoutMs(600000))
            .step("price", "catalog.price")
            .step("notify", "crm.notify", StepOptions.create().concurrency(ConcurrencyPolicy.of(2).key("steps.price.output.region")))
            .end("done");
    }

    private static JsonNode fixture(String name) throws Exception {
        return Json.parse(Files.readString(Path.of("../../../tests/contract/flow-control-step-policies/fixtures/" + name + ".workflow.json")));
    }

    @Test
    void theBuildersWriteTheSharedFixturesByteForByte() throws Exception {
        assertEquals(Json.canonical(fixture("provider-sync")), providerSync().toJson());
        assertEquals(Json.canonical(fixture("account-serial")), accountSerial().toJson());
    }

    @Test
    void noPolicyWritesNoFieldSoEarlierDocumentsAreUnchanged() {
        JsonNode document = WorkflowBuilder.named("plain-flow").step("a", "act.a").script("s", DIGEST, ScriptOptions.create()).end("done").build();
        for (String key : new String[] {"concurrency", "rateLimit"}) {
            assertFalse(document.has(key), key);
            for (JsonNode node : document.path("nodes")) {
                assertFalse(node.has(key), key);
            }
        }
    }

    @Test
    void aScriptCarriesBothPoliciesWithEveryMember() {
        JsonNode node = WorkflowBuilder.named("gated-script")
            .script("s", DIGEST, ScriptOptions.create()
                .concurrency(ConcurrencyPolicy.of(10000).key("input.tenant").group("scripts.a-b").queueTimeoutMs(86400000))
                .rateLimit(RateLimitPolicy.of(3, RatePeriod.HOUR).key("input.tenant").group("scripts.a-b").queueTimeoutMs(1)))
            .end("done").build().path("nodes").get(0);
        assertEquals("{\"group\":\"scripts.a-b\",\"key\":\"input.tenant\",\"limit\":10000,\"queueTimeoutMs\":86400000}", Json.canonical(node.path("concurrency")));
        assertEquals("{\"group\":\"scripts.a-b\",\"key\":\"input.tenant\",\"limit\":3,\"per\":\"hour\",\"queueTimeoutMs\":1}", Json.canonical(node.path("rateLimit")));
    }

    @Test
    void thePolicyIsReadWhenGivenAndTheLastWorkflowLevelCallWins() {
        ConcurrencyPolicy policy = ConcurrencyPolicy.of(2);
        RateLimitPolicy rate = RateLimitPolicy.of(4, RatePeriod.SECOND);
        WorkflowBuilder builder = WorkflowBuilder.named("replaced-flow")
            .concurrency(ConcurrencyPolicy.of(9)).concurrency(policy)
            .rateLimit(RateLimitPolicy.of(9, RatePeriod.HOUR)).rateLimit(rate)
            .step("a", "act.a", StepOptions.create().concurrency(policy).rateLimit(rate)).end("done");
        policy.key("input.later");
        rate.group("later");
        JsonNode document = builder.build();
        assertEquals("{\"limit\":2}", Json.canonical(document.path("concurrency")));
        assertEquals("{\"limit\":4,\"per\":\"second\"}", Json.canonical(document.path("rateLimit")));
        assertEquals("{\"limit\":2}", Json.canonical(document.path("nodes").get(0).path("concurrency")));
        assertEquals("{\"limit\":4,\"per\":\"second\"}", Json.canonical(document.path("nodes").get(0).path("rateLimit")));
    }

    @Test
    void theBuildersOnlyAssembleTheBoundsAreTheServers() {
        JsonNode document = WorkflowBuilder.named("unchecked-flow")
            .concurrency(ConcurrencyPolicy.of(0).group("Not A Group"))
            .rateLimit(RateLimitPolicy.of(101, RatePeriod.SECOND).queueTimeoutMs(86400001))
            .step("a", "act.a", StepOptions.create().concurrency(ConcurrencyPolicy.of(10001).key("")))
            .end("done").build();
        assertEquals("{\"group\":\"Not A Group\",\"limit\":0}", Json.canonical(document.path("concurrency")));
        assertEquals("{\"limit\":101,\"per\":\"second\",\"queueTimeoutMs\":86400001}", Json.canonical(document.path("rateLimit")));
        assertEquals("{\"key\":\"\",\"limit\":10001}", Json.canonical(document.path("nodes").get(0).path("concurrency")));
    }

    @Test
    void aRateLimitNeedsItsWindow() {
        SagatException error = assertThrows(SagatException.class, () -> RateLimitPolicy.of(1, null));
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(error.getMessage().contains("rateLimit per"), error.getMessage());
    }
}
