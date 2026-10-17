package dev.sagatoctopusorchestrator.conformance;

// The conformance runner of the Java SDK (tests/contract/sdk-conformance/README.md). It uses only the public API
// of the SDK: the builders in `build` mode, the client and the worker host in `run` mode, plus a plain HTTP call
// to the control endpoint.

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.CallOptions;
import dev.sagatoctopusorchestrator.sdk.ApprovalDecision;
import dev.sagatoctopusorchestrator.sdk.ApprovalDecisionOptions;
import dev.sagatoctopusorchestrator.sdk.ApprovalDecisionReceipt;
import dev.sagatoctopusorchestrator.sdk.ApprovalListItem;
import dev.sagatoctopusorchestrator.sdk.ApprovalListOptions;
import dev.sagatoctopusorchestrator.sdk.ApprovalPage;
import dev.sagatoctopusorchestrator.sdk.ClientConfig;
import dev.sagatoctopusorchestrator.sdk.ChainQueryOptions;
import dev.sagatoctopusorchestrator.sdk.EventAccepted;
import dev.sagatoctopusorchestrator.sdk.EventInput;
import dev.sagatoctopusorchestrator.sdk.JdkHttpTransport;
import dev.sagatoctopusorchestrator.sdk.JournalPage;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.ListOptions;
import dev.sagatoctopusorchestrator.sdk.OperateAction;
import dev.sagatoctopusorchestrator.sdk.OperateOptions;
import dev.sagatoctopusorchestrator.sdk.OperationResult;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.Run;
import dev.sagatoctopusorchestrator.sdk.RunStatus;
import dev.sagatoctopusorchestrator.sdk.RunInputBlobOptions;
import dev.sagatoctopusorchestrator.sdk.SagatClient;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.SagatSdk;
import dev.sagatoctopusorchestrator.sdk.SignalResult;
import dev.sagatoctopusorchestrator.sdk.StartOptions;
import dev.sagatoctopusorchestrator.sdk.WaitOptions;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerConfig;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerHost;
import dev.sagatoctopusorchestrator.sdk.workflow.ChildOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.ApprovalOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.ApprovalPolicy;
import dev.sagatoctopusorchestrator.sdk.workflow.ApprovalRole;
import dev.sagatoctopusorchestrator.sdk.workflow.CollectionOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.ChainPolicy;
import dev.sagatoctopusorchestrator.sdk.workflow.Flow;
import dev.sagatoctopusorchestrator.sdk.workflow.LoopOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.QueryDefinition;
import dev.sagatoctopusorchestrator.sdk.workflow.RetryOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.SagaOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.StateDefinition;
import dev.sagatoctopusorchestrator.sdk.workflow.StepOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.SwitchCase;
import dev.sagatoctopusorchestrator.sdk.workflow.Trigger;
import dev.sagatoctopusorchestrator.sdk.workflow.UpdateDefinition;
import dev.sagatoctopusorchestrator.sdk.workflow.WaitForEventOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.WorkflowBuilder;
import java.io.File;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

public final class Runner {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;
    private static final RetryPolicy FAST_RETRY = new RetryPolicy(4, Duration.ofMillis(20), Duration.ofMillis(200));

    private Runner() {}

    // --------------------------------------------------------------------------------------- scenario

    private static JsonNode read(String path) throws IOException {
        return Json.mapper().readTree(new File(path));
    }

    /** Replaces {@code {{unique}}} in every string value of the scenario. */
    private static JsonNode templated(JsonNode value, String unique) {
        if (value.isTextual()) {
            return F.textNode(value.textValue().replace("{{unique}}", unique));
        }
        if (value.isArray()) {
            ArrayNode out = F.arrayNode();
            value.forEach(item -> out.add(templated(item, unique)));
            return out;
        }
        if (value.isObject()) {
            ObjectNode out = F.objectNode();
            value.properties().forEach(entry -> out.set(entry.getKey(), templated(entry.getValue(), unique)));
            return out;
        }
        return value;
    }

    // ---------------------------------------------------------------------------------------- builders

    private static void known(JsonNode options, String what, String... names) {
        if (options == null || options.isNull()) {
            return;
        }
        List<String> allowed = List.of(names);
        for (Iterator<String> it = options.fieldNames(); it.hasNext(); ) {
            String name = it.next();
            if (!allowed.contains(name)) {
                throw new IllegalArgumentException("the runner does not know the option " + name + " of " + what);
            }
        }
    }

    private static <E extends Enum<E>> E enumOf(Class<E> type, JsonNode value) {
        return Enum.valueOf(type, value.asText().toUpperCase(Locale.ROOT));
    }

    private static RetryOptions retryOf(JsonNode node) {
        known(node, "retry", "maxAttempts", "initialDelayMs", "maxDelayMs", "strategy", "jitter");
        RetryOptions retry = RetryOptions.create();
        if (node.has("maxAttempts")) {
            retry.maxAttempts(node.get("maxAttempts").asInt());
        }
        if (node.has("initialDelayMs")) {
            retry.initialDelayMs(node.get("initialDelayMs").asLong());
        }
        if (node.has("maxDelayMs")) {
            retry.maxDelayMs(node.get("maxDelayMs").asLong());
        }
        if (node.has("strategy")) {
            retry.strategy(enumOf(RetryOptions.Strategy.class, node.get("strategy")));
        }
        if (node.has("jitter")) {
            retry.jitter(node.get("jitter").asBoolean());
        }
        return retry;
    }

    private static StepOptions stepOptionsOf(JsonNode node) {
        StepOptions options = StepOptions.create();
        if (node == null || node.isNull()) {
            return options;
        }
        known(node, "step", "retry", "timeoutMs", "compensateWith", "reconcileWith", "idempotencyTemplate", "secrets",
                "compensationRetry", "compensationTimeoutMs", "irreversible", "highRisk", "onError");
        if (node.has("retry")) {
            options.retry(retryOf(node.get("retry")));
        }
        if (node.has("timeoutMs")) {
            options.timeoutMs(node.get("timeoutMs").asLong());
        }
        if (node.has("compensateWith")) {
            options.compensateWith(node.get("compensateWith").asText());
        }
        if (node.has("reconcileWith")) {
            options.reconcileWith(node.get("reconcileWith").asText());
        }
        if (node.has("idempotencyTemplate")) {
            options.idempotencyTemplate(node.get("idempotencyTemplate").asText());
        }
        if (node.has("secrets")) {
            List<String> secrets = new ArrayList<>();
            node.get("secrets").forEach(s -> secrets.add(s.asText()));
            options.secrets(secrets);
        }
        if (node.has("compensationRetry")) {
            options.compensationRetry(retryOf(node.get("compensationRetry")));
        }
        if (node.has("compensationTimeoutMs")) {
            options.compensationTimeoutMs(node.get("compensationTimeoutMs").asLong());
        }
        if (node.has("irreversible")) {
            options.irreversible(node.get("irreversible").asBoolean());
        }
        if (node.has("highRisk")) {
            options.highRisk(node.get("highRisk").asBoolean());
        }
        if (node.has("onError")) {
            options.onError(node.get("onError").asText());
        }
        return options;
    }

    private static void apply(Flow target, JsonNode ops) {
        for (JsonNode op : ops) {
            String id = op.path("id").asText(null);
            switch (op.path("op").asText()) {
                case "step" -> target.step(id, op.path("activity").asText(null), stepOptionsOf(op.get("options")));
                case "sleep" -> target.sleepMs(id, op.path("durationMs").asLong());
                case "waitForEvent" -> {
                    JsonNode o = op.path("options");
                    known(o, "waitForEvent", "eventType", "correlationExpr", "timeoutMs", "onTimeout", "lookbackMs");
                    WaitForEventOptions options = WaitForEventOptions.of(
                            o.path("eventType").asText(null), o.path("correlationExpr").asText(null), o.path("timeoutMs").asLong());
                    if (o.has("onTimeout")) {
                        options.onTimeout(o.get("onTimeout").asText());
                    }
                    if (o.has("lookbackMs")) {
                        options.lookbackMs(o.get("lookbackMs").asLong());
                    }
                    target.waitForEvent(id, options);
                }
                case "invokeChild" -> {
                    JsonNode o = op.get("options");
                    known(o, "invokeChild", "version", "input", "cancelPolicy");
                    ChildOptions options = ChildOptions.create();
                    if (o != null && o.has("version")) {
                        options.version(o.get("version").asInt());
                    }
                    if (o != null && o.has("input")) {
                        options.input(o.get("input").asText());
                    }
                    if (o != null && o.has("cancelPolicy")) {
                        options.cancelPolicy(enumOf(ChildOptions.CancelPolicy.class, o.get("cancelPolicy")));
                    }
                    target.invokeChild(id, op.path("workflow").asText(null), options);
                }
                case "condition" -> target.condition(
                        id, op.path("expression").asText(null), op.path("ifTrue").asText(null), op.path("ifFalse").asText(null));
                case "switch" -> {
                    List<SwitchCase> cases = new ArrayList<>();
                    for (JsonNode item : op.path("cases")) {
                        cases.add(new SwitchCase(
                                item.path("id").asText(null),
                                item.path("when").asText(null),
                                item.path("target").asText(null)));
                    }
                    target.switchCase(id, cases, op.path("default").asText(null));
                }
                case "forEach" -> target.forEach(
                        id,
                        op.path("items").asText(null),
                        op.path("body").asText(null),
                        op.path("result").asText(null),
                        CollectionOptions.of(op.path("maxItems").asInt(), op.path("maxConcurrency").asInt()));
                case "map" -> target.map(
                        id,
                        op.path("items").asText(null),
                        op.path("body").asText(null),
                        op.path("result").asText(null),
                        CollectionOptions.of(op.path("maxItems").asInt(), op.path("maxConcurrency").asInt()));
                case "while" -> target.whileLoop(
                        id, op.path("condition").asText(null), op.path("body").asText(null),
                        op.path("initial").asText(null), op.path("carry").asText(null), loopOptions(op));
                case "repeatUntil" -> target.repeatUntil(
                        id, op.path("condition").asText(null), op.path("body").asText(null),
                        op.path("initial").asText(null), op.path("carry").asText(null), loopOptions(op));
                case "approval" -> {
                    JsonNode policy = op.path("policy");
                    List<String> subjects = null;
                    if (policy.has("subjects")) {
                        subjects = new ArrayList<>();
                        for (JsonNode subject : policy.path("subjects")) {
                            subjects.add(subject.textValue());
                        }
                    }
                    List<ApprovalRole> roles = null;
                    if (policy.has("roles")) {
                        roles = new ArrayList<>();
                        for (JsonNode role : policy.path("roles")) {
                            roles.add(switch (role.asText()) {
                                case "admin" -> ApprovalRole.ADMIN;
                                case "developer" -> ApprovalRole.DEVELOPER;
                                case "operator" -> ApprovalRole.OPERATOR;
                                default -> null;
                            });
                        }
                    }
                    Boolean exclude = policy.has("excludeInitiator")
                            ? policy.path("excludeInitiator").booleanValue()
                            : null;
                    target.approval(id, new ApprovalOptions(
                            new ApprovalPolicy(subjects, roles, exclude),
                            Duration.ofMillis(op.path("timeoutMs").longValue()),
                            op.path("onApprove").asText(null),
                            op.path("onReject").asText(null),
                            op.path("onExpire").asText(null)));
                }
                case "continueAsNew" -> target.continueAsNew(
                        id,
                        op.path("input").asText(null),
                        op.has("state") ? op.path("state").asText() : null);
                case "return" -> target.returnValue(id, op.path("result").asText(null));
                case "parallel" -> {
                    List<Flow> branches = new ArrayList<>();
                    for (JsonNode branchOps : op.path("branches")) {
                        Flow branch = Flow.create();
                        apply(branch, branchOps);
                        branches.add(branch);
                    }
                    target.parallel(id, enumOf(Flow.Join.class, op.path("join")), branches);
                }
                case "saga" -> {
                    JsonNode o = op.get("options");
                    known(o, "saga", "onCompensationFailure", "cancelPolicy");
                    SagaOptions options = SagaOptions.create();
                    if (o != null && o.has("onCompensationFailure")) {
                        options.onCompensationFailure(enumOf(SagaOptions.OnCompensationFailure.class, o.get("onCompensationFailure")));
                    }
                    if (o != null && o.has("cancelPolicy")) {
                        options.cancelPolicy(enumOf(SagaOptions.CancelPolicy.class, o.get("cancelPolicy")));
                    }
                    Flow body = Flow.create();
                    apply(body, op.path("body"));
                    target.saga(id, options, body);
                }
                case "end" -> {
                    JsonNode o = op.get("options");
                    known(o, "end", "outputExpr");
                    target.end(id, o != null && o.has("outputExpr") ? o.get("outputExpr").asText() : null);
                }
                default -> throw new IllegalArgumentException("unknown builder op " + op.path("op").asText());
            }
        }
    }

    private static LoopOptions loopOptions(JsonNode op) {
        LoopOptions options = LoopOptions.of(
                op.path("maxIterations").intValue(), Duration.ofMillis(op.path("timeoutMs").longValue()));
        if (op.has("delayMs")) {
            options = options.withDelay(Duration.ofMillis(op.path("delayMs").longValue()));
        }
        if (op.has("onError")) {
            options = options.withOnError(op.path("onError").textValue());
        }
        return options;
    }

    private static WorkflowBuilder builderOf(JsonNode definition) {
        WorkflowBuilder builder = WorkflowBuilder.named(definition.path("name").asText(null));
        if (definition.has("description") && !definition.get("description").asText().isEmpty()) {
            builder.description(definition.get("description").asText());
        }
        definition.path("labels").properties().forEach(e -> builder.label(e.getKey(), e.getValue().asText()));
        for (JsonNode trigger : definition.path("triggers")) {
            builder.trigger(Trigger.fromJson(trigger));
        }
        if (definition.has("inputSchema")) {
            builder.inputSchema(definition.get("inputSchema"));
        }
        if (definition.has("outputSchema")) {
            builder.outputSchema(definition.get("outputSchema"));
        }
        if (definition.has("state")) {
            JsonNode state = definition.path("state");
            Map<String, QueryDefinition> queries = new LinkedHashMap<>();
            state.path("queries").properties().forEach(entry -> {
                JsonNode query = entry.getValue();
                queries.put(entry.getKey(), new QueryDefinition(
                        query.get("argsSchema"), query.get("resultSchema"),
                        query.path("expression").textValue(), query.path("disclosure").textValue()));
            });
            Map<String, UpdateDefinition> updates = new LinkedHashMap<>();
            state.path("updates").properties().forEach(entry -> {
                JsonNode update = entry.getValue();
                updates.put(entry.getKey(), new UpdateDefinition(
                        update.get("argsSchema"), update.path("validate").textValue(),
                        update.path("state").textValue(), update.path("result").textValue(),
                        update.get("resultSchema"), update.path("disclosure").textValue()));
            });
            builder.state(new StateDefinition(state.get("schema"), state.path("initial").textValue(), queries, updates));
        }
        if (definition.has("chainPolicy")) {
            JsonNode policy = definition.path("chainPolicy");
            builder.chainPolicy(ChainPolicy.of(
                    policy.path("maxSegments").intValue(),
                    Duration.ofMillis(policy.path("timeoutMs").longValue())));
        }
        definition.path("bodies").properties().forEach(e -> {
            Flow nested = Flow.body();
            apply(nested, e.getValue().path("program"));
            builder.body(e.getKey(), nested);
        });
        Flow program = Flow.create();
        apply(program, definition.path("program"));
        builder.append(program);
        return builder;
    }

    private static ObjectNode build(JsonNode scenario) {
        ObjectNode workflows = F.objectNode();
        ObjectNode canonical = F.objectNode();
        scenario.path("workflows").properties().forEach(e -> {
            WorkflowBuilder builder = builderOf(e.getValue());
            workflows.set(e.getKey(), builder.build());
            // The canonical text is part of the contract (SPEC section 7): the same bytes in every language.
            canonical.put(e.getKey(), builder.toJson());
        });
        ObjectNode errors = F.objectNode();
        scenario.path("invalidWorkflows").properties().forEach(e -> {
            JsonNode invalid = e.getValue();
            String contains = invalid.path("contains").asText("");
            ObjectNode result = F.objectNode();
            try {
                // "Building" a workflow is its document and its canonical text (SPEC sections 7 and 8): the text refuses an
                // integer beyond 2^53 - 1, which the document alone does not.
                WorkflowBuilder builder = builderOf(invalid.path("workflow"));
                builder.build();
                builder.toJson();
                result.put("raised", false);
            } catch (SagatException error) {
                result.put("raised", true);
                result.put("code", error.code().name());
                result.put("mentions", contains.isEmpty() || error.getMessage().contains(contains));
            } catch (RuntimeException error) {
                result.put("raised", true);
                result.put("code", "UNEXPECTED");
                result.put("mentions", false);
            }
            errors.set(e.getKey(), result);
        });
        ObjectNode out = F.objectNode();
        out.set("workflows", workflows);
        out.set("canonical", canonical);
        if (errors.size() > 0) {
            out.set("errors", errors);
        }
        return out;
    }

    // ---------------------------------------------------------------------------------- observations

    private static JsonNode instant(Instant value) {
        return value == null ? F.nullNode() : F.textNode(value.toString());
    }

    private static ObjectNode runJson(Run run) {
        ObjectNode out = F.objectNode();
        out.put("runId", run.runId());
        out.put("workflowId", run.workflowId());
        out.put("versionId", run.versionId());
        out.put("status", run.statusText());
        out.put("version", run.version());
        out.put("paused", run.paused());
        out.put("startedBy", run.startedBy());
        out.set("createdAt", instant(run.createdAt()));
        out.set("updatedAt", instant(run.updatedAt()));
        out.set("finishedAt", instant(run.finishedAt()));
        out.set("deadline", instant(run.deadline()));
        out.put("traceId", run.traceId());
        out.put("correlationId", run.correlationId());
        out.put("depth", run.depth());
        if (run.parent() == null) {
            out.putNull("parent");
        } else {
            ObjectNode parent = out.putObject("parent");
            parent.put("runId", run.parent().runId());
            parent.put("node", run.parent().node());
            parent.put("cancelPolicy", run.parent().cancelPolicy());
        }
        return out;
    }

    private static ObjectNode chainJson(dev.sagatoctopusorchestrator.sdk.ChainInspection chain) {
        ObjectNode result = F.objectNode();
        result.put("chainId", chain.chainId()); result.put("currentRunId", chain.currentRunId());
        result.put("revision", chain.revision()); result.put("cancelRequested", chain.cancelRequested());
        if (chain.finalOutcome() == null) result.putNull("finalOutcome"); else result.put("finalOutcome", chain.finalOutcome());
        ArrayNode segments = result.putArray("segments");
        chain.segments().forEach(item -> {
            ObjectNode value = segments.addObject(); value.put("index", item.index()); value.put("runId", item.runId());
            if (item.predecessorRunId() == null) value.putNull("predecessorRunId"); else value.put("predecessorRunId", item.predecessorRunId());
            if (item.successorRunId() == null) value.putNull("successorRunId"); else value.put("successorRunId", item.successorRunId());
        });
        return result;
    }

    private static ObjectNode errorJson(Throwable error) {
        ObjectNode out = F.objectNode();
        if (error instanceof SagatException e) {
            out.put("code", e.code().name());
            if (e.httpStatus().isPresent()) {
                out.put("status", e.httpStatus().getAsInt());
            } else if (e.grpcStatus().isPresent()) {
                out.put("status", e.grpcStatus().get());
            } else {
                out.putNull("status");
            }
            out.put("retryable", e.retryable());
            out.put("traceId", e.traceId());
            out.put("reason", e.reason().orElse(null));
            if (e.retryAfterMs().isPresent()) {
                out.put("retryAfterMs", e.retryAfterMs().getAsLong());
            } else {
                out.putNull("retryAfterMs");
            }
            out.put("remediation", e.remediation());
            out.put("message", e.getMessage());
            return out;
        }
        out.put("code", "UNEXPECTED");
        out.put("message", error.getClass().getSimpleName());
        return out;
    }

    // ----------------------------------------------------------------------------------------- workers

    private static final class WorkerEntry {
        final JsonNode def;
        final List<ObjectNode> log = Collections.synchronizedList(new ArrayList<>());
        final Behaviors.Measurements measurements = new Behaviors.Measurements();
        WorkerHost host;

        WorkerEntry(JsonNode def) {
            this.def = def;
        }
    }

    private static WorkerHost hostOf(
            JsonNode def, JsonNode config, FaultPlan plan, List<ObjectNode> log, Behaviors.Measurements measurements) {
        WorkerConfig.Builder builder = WorkerConfig.builder()
                .grpcUrl(config.get("grpcUrl").asText())
                .namespace(config.get("namespace").asText())
                .token("unbound".equals(def.path("token").asText()) ? config.get("unboundWorkerToken").asText() : config.get("workerToken").asText())
                .workerName(def.get("workerName").asText())
                .maxInflight(def.has("maxInflight") ? def.get("maxInflight").asInt() : 4)
                .retry(FAST_RETRY);
        if (def.has("protocolVersion")) {
            builder.protocolVersion(def.get("protocolVersion").asText());
        }
        if (def.has("transport")) {
            builder.transport(dev.sagatoctopusorchestrator.sdk.worker.WorkerTransport.valueOf(def.get("transport").asText().toUpperCase(java.util.Locale.ROOT)));
        }
        if (def.has("heartbeatIntervalMs")) {
            builder.heartbeatInterval(Duration.ofMillis(def.get("heartbeatIntervalMs").asLong()));
        }
        plan.interceptors().forEach(builder::interceptor);
        WorkerHost host = new WorkerHost(builder.build());
        def.path("activities").properties().forEach(e -> {
            String name = e.getKey();
            String kind = e.getValue().path("as").asText("activity");
            var handler = Behaviors.handler(name, kind, e.getValue().path("behavior"), log, measurements);
            switch (kind) {
                case "compensation" -> host.registerCompensation(name, handler);
                case "reconciler" -> host.registerReconciler(name, (input, ctx) -> (dev.sagatoctopusorchestrator.sdk.worker.Verdict) handler.handle(input, ctx));
                default -> host.registerActivity(name, handler);
            }
        });
        return host;
    }

    // ------------------------------------------------------------------------------------------- run

    private static ObjectNode run(JsonNode scenario, JsonNode config) throws Exception {
        String unique = config.get("unique").asText();
        FaultPlan plan = new FaultPlan(scenario.path("faults"));
        String apiPath = URI.create(config.get("apiUrl").asText()).getPath().replaceAll("/$", "");

        Map<String, SagatClient> clients = new LinkedHashMap<>();
        for (Map.Entry<String, String> token : Map.of(
                "default", "clientToken", "bad", "badToken", "other", "otherTenantToken",
                // Members of the tenant with one role each, the worker identity, and an operator the scenario revokes.
                "worker", "workerToken", "operator", "operatorToken", "viewer", "viewerToken",
                "runner", "runnerToken", "revoked", "revokedToken").entrySet()) {
            clients.put(token.getKey(), SagatClient.create(ClientConfig.builder()
                    .apiUrl(config.get("apiUrl").asText())
                    .namespace(config.get("namespace").asText())
                    .token(config.get(token.getValue()).asText())
                    .retry(FAST_RETRY)
                    .transport(plan.wrap(new JdkHttpTransport(Duration.ofSeconds(30)), apiPath))
                    .build()));
        }

        Map<String, WorkerEntry> hosts = new LinkedHashMap<>();
        scenario.path("workers").properties().forEach(e -> hosts.put(e.getKey(), new WorkerEntry(e.getValue())));

        HttpClient control = HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1).build();
        ArrayNode observations = F.arrayNode();
        Map<String, JsonNode> results = new LinkedHashMap<>();

        for (Map.Entry<String, WorkerEntry> entry : hosts.entrySet()) {
            if (entry.getValue().def.path("autostart").asBoolean(true)) {
                startWorker(entry.getValue(), config, plan);
            }
        }

        for (JsonNode step : scenario.path("steps")) {
            SagatClient client = clients.get(step.path("token").asText("default"));
            ObjectNode observation = F.objectNode();
            observation.put("id", step.get("id").asText());
            observation.put("op", step.get("op").asText());
            if (System.getenv("SAGAT_RUNNER_TRACE") != null) {
                System.err.println("step " + step.get("id").asText());
            }
            try {
                JsonNode result = play(step, client, config, hosts, results, plan, control, unique);
                observation.put("ok", true);
                if (result != null) {
                    observation.set("result", result);
                    results.put(step.get("id").asText(), result);
                }
            } catch (Exception error) {
                observation.put("ok", false);
                observation.set("error", errorJson(error));
            }
            observations.add(observation);
        }

        for (WorkerEntry entry : hosts.values()) {
            if (entry.host != null) {
                try {
                    entry.host.stop(Duration.ofSeconds(2));
                } catch (RuntimeException ignored) {
                    // The scenario is over.
                }
            }
        }
        for (SagatClient client : clients.values()) {
            client.close();
        }

        ObjectNode workerLog = F.objectNode();
        ObjectNode workerMeasurements = F.objectNode();
        for (Map.Entry<String, WorkerEntry> entry : hosts.entrySet()) {
            ArrayNode log = workerLog.putArray(entry.getKey());
            synchronized (entry.getValue().log) {
                entry.getValue().log.forEach(log::add);
            }
            workerMeasurements.set(entry.getKey(), entry.getValue().measurements.snapshot());
        }
        ObjectNode out = F.objectNode();
        out.put("scenario", scenario.get("name").asText());
        out.put("language", "java");
        out.put("sdkVersion", SagatSdk.VERSION);
        out.set("observations", observations);
        out.set("workerLog", workerLog);
        out.set("workerMeasurements", workerMeasurements);
        ArrayNode applied = out.putArray("faultsApplied");
        for (long count : plan.applied()) {
            applied.add(count);
        }
        return out;
    }

    private static ObjectNode startWorker(WorkerEntry entry, JsonNode config, FaultPlan plan) {
        entry.host = hostOf(entry.def, config, plan, entry.log, entry.measurements);
        entry.host.start();
        ObjectNode result = F.objectNode();
        result.put("protocolVersion", entry.host.protocolVersion());
        return result;
    }

    private static String runOf(JsonNode step, Map<String, JsonNode> results) {
        JsonNode result = results.get(step.path("run").asText());
        JsonNode id = result == null ? null : result.get("runId");
        if (id == null || !id.isTextual()) {
            throw new IllegalStateException("step " + step.path("run").asText() + " has no run");
        }
        return id.textValue();
    }

    private static String approvalOf(JsonNode step, Map<String, JsonNode> results) {
        if (step.has("approvalId")) return step.get("approvalId").asText();
        JsonNode source = results.get(step.get("approval").asText());
        JsonNode items = source == null ? null : source.path("items");
        if (items == null || !items.isArray() || items.isEmpty() || !items.get(0).path("approvalId").isTextual()) {
            throw new IllegalArgumentException("step " + step.get("approval").asText() + " has no approval");
        }
        return items.get(0).get("approvalId").asText();
    }

    private static String chainRef(JsonNode value, JsonNode config, Map<String, JsonNode> results) {
        String text = value.asText();
        if (text.equals("{{namespaceId}}")) return config.get("namespaceId").asText();
        String field;
        String key;
        if (text.startsWith("{{chainId:") && text.endsWith("}}")) {
            field = "runId";
            key = text.substring(10, text.length() - 2);
        } else if (text.startsWith("{{currentRunId:") && text.endsWith("}}")) {
            field = "currentRunId";
            key = text.substring(15, text.length() - 2);
        } else {
            return text;
        }
        JsonNode source = results.get(key);
        JsonNode resolved = source == null ? null : source.get(field);
        if (resolved == null || !resolved.isTextual()) throw new IllegalStateException("step " + key + " has no " + field);
        return resolved.asText();
    }

    private static void pause(long millis) throws InterruptedException {
        Thread.sleep(millis);
    }

    private static JsonNode play(
            JsonNode step,
            SagatClient client,
            JsonNode config,
            Map<String, WorkerEntry> hosts,
            Map<String, JsonNode> results,
            FaultPlan plan,
            HttpClient control,
            String unique)
            throws Exception {
        switch (step.get("op").asText()) {
            case "start": {
                JsonNode workflow = config.path("workflows").path(step.get("workflow").asText());
                StartOptions options = StartOptions.create();
                if (step.has("idempotencyKey")) {
                    options.idempotencyKey(step.get("idempotencyKey").asText());
                }
                if (step.has("correlationId")) {
                    options.correlationId(step.get("correlationId").asText());
                }
                if (step.has("traceparent")) {
                    options.traceparent(step.get("traceparent").asText());
                }
                if (step.has("deadlineInMs")) {
                    options.deadlineIn(Duration.ofMillis(step.get("deadlineInMs").asLong()));
                }
                return runJson(client.start(workflow.get("workflowId").asText(), step.has("input") ? step.get("input") : F.objectNode(), options));
            }
            case "startWithBlob": {
                JsonNode workflow = config.path("workflows").path(step.get("workflow").asText());
                byte[] body = new byte[step.get("inputBlobBytes").asInt()];
                java.util.Arrays.fill(body, (byte) 'i');
                String sha256 = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(body));
                var descriptor = client.uploadRunInput(new ByteArrayInputStream(body),
                        new RunInputBlobOptions("application/octet-stream", (long) body.length, sha256));
                StartOptions options = StartOptions.create();
                if (step.has("idempotencyKey")) options.idempotencyKey(step.get("idempotencyKey").asText());
                if (step.has("correlationId")) options.correlationId(step.get("correlationId").asText());
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                if (step.has("deadlineInMs")) options.deadlineIn(Duration.ofMillis(step.get("deadlineInMs").asLong()));
                return runJson(client.startWithBlob(workflow.get("workflowId").asText(), descriptor, options));
            }
            case "workflowByName": {
                String name = config.path("workflows").path(step.get("workflow").asText()).get("name").asText();
                StartOptions options = StartOptions.create();
                if (step.has("idempotencyKey")) {
                    options.idempotencyKey(step.get("idempotencyKey").asText());
                }
                return runJson(client.start(name, step.has("input") ? step.get("input") : F.objectNode(), options));
            }
            case "get":
                return runJson(client.get(runOf(step, results)));
            case "list": {
                ListOptions options = ListOptions.create();
                if (step.has("status")) {
                    options.status(RunStatus.valueOf(step.get("status").asText()));
                }
                if (step.has("limit")) {
                    options.limit(step.get("limit").asInt());
                }
                ArrayNode ids = F.arrayNode();
                if (step.path("all").asBoolean(false)) {
                    client.listAll(options).forEach(run -> ids.add(run.runId()));
                } else {
                    // In the order the server gave (newest first): the SDK keeps it, also across pages.
                    client.list(options).items().forEach(run -> ids.add(run.runId()));
                }
                ObjectNode result = F.objectNode();
                result.set("items", ids);
                return result;
            }
            case "listApprovals": {
                ApprovalListOptions options = ApprovalListOptions.create();
                if (step.has("cursor")) options.cursor(step.get("cursor").asText());
                if (step.has("limit")) options.limit(step.get("limit").asInt());
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                ApprovalPage page = client.listApprovals(runOf(step, results), options);
                for (int attempt = 0; step.path("waitForItem").asBoolean(false) && page.items().isEmpty() && attempt < 100; attempt++) {
                    try {
                        Thread.sleep(50);
                    } catch (InterruptedException error) {
                        Thread.currentThread().interrupt();
                        throw new IllegalStateException("interrupted while waiting for an approval");
                    }
                    page = client.listApprovals(runOf(step, results), options);
                }
                ArrayNode items = F.arrayNode();
                for (ApprovalListItem item : page.items()) {
                    ObjectNode value = F.objectNode();
                    value.put("approvalId", item.approvalId());
                    value.put("runId", item.runId());
                    value.put("nodeId", item.nodeId());
                    ArrayNode path = F.arrayNode();
                    item.instancePath().forEach(scope -> {
                        ObjectNode segment = F.objectNode();
                        segment.put("scopeNodeId", scope.scopeNodeId());
                        segment.put("iterationIndex", scope.iterationIndex());
                        path.add(segment);
                    });
                    value.set("instancePath", path);
                    value.put("status", item.status().name());
                    value.put("deadline", java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'")
                            .withZone(java.time.ZoneOffset.UTC).format(item.deadline()));
                    if (item.settledAt() == null) value.putNull("settledAt"); else value.put("settledAt",
                            java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'")
                                    .withZone(java.time.ZoneOffset.UTC).format(item.settledAt()));
                    items.add(value);
                }
                ObjectNode result = F.objectNode();
                result.set("items", items);
                if (page.nextCursor() == null) result.putNull("nextCursor"); else result.put("nextCursor", page.nextCursor());
                return result;
            }
            case "cancel": {
                OperateOptions options = OperateOptions.create();
                if (step.has("expectedRunVersion")) {
                    options.expectedRunVersion(step.get("expectedRunVersion").asLong());
                }
                return operation(client.cancel(runOf(step, results), step.get("reason").asText(), options));
            }
            case "operate": {
                OperateOptions options = OperateOptions.create();
                if (step.has("stepId")) {
                    options.stepId(step.get("stepId").asText());
                }
                if (step.has("outcome")) {
                    options.outcome(OperateOptions.Outcome.valueOf(step.get("outcome").asText()));
                }
                if (step.has("errorCode")) {
                    options.errorCode(step.get("errorCode").asText());
                }
                return operation(client.operate(
                        runOf(step, results), OperateAction.valueOf(step.get("action").asText()), step.get("reason").asText(), options));
            }
            case "decideApproval": {
                ApprovalDecisionOptions options = ApprovalDecisionOptions.create(step.get("decisionKey").asText());
                if (step.has("reasonCode")) {
                    options.reasonCode(step.get("reasonCode").asText());
                }
                if (step.has("traceparent")) {
                    options.traceparent(step.get("traceparent").asText());
                }
                ApprovalDecisionReceipt receipt = client.decideApproval(
                        approvalOf(step, results), ApprovalDecision.valueOf(step.get("decision").asText()), options);
                ObjectNode result = F.objectNode();
                result.put("receiptId", receipt.receiptId());
                result.put("approvalId", receipt.approvalId());
                result.put("decisionKey", receipt.decisionKey());
                result.put("decision", receipt.decision().name());
                if (receipt.reasonCode() == null) result.putNull("reasonCode"); else result.put("reasonCode", receipt.reasonCode());
                result.put("actor", receipt.actor());
                result.put("decidedAt", java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'")
                        .withZone(java.time.ZoneOffset.UTC).format(receipt.decidedAt()));
                result.put("outcomeSequence", receipt.outcomeSequence());
                result.put("replayed", receipt.replayed());
                return result;
            }
            case "query": {
                dev.sagatoctopusorchestrator.sdk.QueryResult queried = client.query(
                        runOf(step, results), step.get("name").asText(), step.get("args"), CallOptions.create());
                ObjectNode result = F.objectNode();
                result.put("runVersion", queried.runVersion());
                result.put("stateRevision", queried.stateRevision());
                result.set("result", queried.result());
                return result;
            }
            case "queryChain": {
                ChainQueryOptions options = ChainQueryOptions.create();
                if (step.has("expectedRunId")) {
                    options.expectedRunId(chainRef(step.get("expectedRunId"), config, results));
                }
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                dev.sagatoctopusorchestrator.sdk.QueryResult queried = client.queryChain(
                        chainRef(step.get("namespaceId"), config, results),
                        chainRef(step.get("chainId"), config, results),
                        step.get("name").asText(), step.get("args"), options);
                ObjectNode result = F.objectNode();
                result.put("runId", queried.runId());
                result.put("runVersion", queried.runVersion());
                result.put("stateRevision", queried.stateRevision());
                result.set("result", queried.result());
                return result;
            }
            case "update": {
                dev.sagatoctopusorchestrator.sdk.UpdateOptions options =
                        dev.sagatoctopusorchestrator.sdk.UpdateOptions.create(step.get("updateKey").asText());
                if (step.has("expectedStateRevision")) options.expectedStateRevision(step.get("expectedStateRevision").longValue());
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                dev.sagatoctopusorchestrator.sdk.UpdateReceipt updated = client.update(
                        runOf(step, results), step.get("name").asText(), step.get("args"), options);
                ObjectNode result = F.objectNode();
                result.put("operation", updated.operation()); result.put("updateKey", updated.updateKey());
                result.put("status", updated.status()); result.set("result", updated.result());
                result.put("beforeStateRevision", updated.beforeStateRevision());
                result.put("afterStateRevision", updated.afterStateRevision());
                result.put("runVersion", updated.runVersion()); result.put("replayed", updated.replayed());
                return result;
            }
            case "updateChain": {
                dev.sagatoctopusorchestrator.sdk.ChainUpdateOptions options =
                        dev.sagatoctopusorchestrator.sdk.ChainUpdateOptions.create(step.get("updateKey").asText());
                if (step.has("expectedRunId")) {
                    options.expectedRunId(chainRef(step.get("expectedRunId"), config, results));
                }
                if (step.has("expectedStateRevision")) {
                    options.expectedStateRevision(step.get("expectedStateRevision").longValue());
                }
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                dev.sagatoctopusorchestrator.sdk.UpdateReceipt updated = client.updateChain(
                        chainRef(step.get("namespaceId"), config, results),
                        chainRef(step.get("chainId"), config, results),
                        step.get("name").asText(), step.get("args"), options);
                ObjectNode result = F.objectNode();
                result.put("runId", updated.runId()); result.put("operation", updated.operation());
                result.put("updateKey", updated.updateKey()); result.put("status", updated.status());
                result.set("result", updated.result());
                result.put("beforeStateRevision", updated.beforeStateRevision());
                result.put("afterStateRevision", updated.afterStateRevision());
                result.put("runVersion", updated.runVersion()); result.put("replayed", updated.replayed());
                return result;
            }
            case "inspectChain": {
                CallOptions options = CallOptions.create();
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                String namespaceId = chainRef(step.get("namespaceId"), config, results);
                String chainId = chainRef(step.get("chainId"), config, results);
                dev.sagatoctopusorchestrator.sdk.ChainInspection chain = client.inspectChain(
                        namespaceId, chainId, options);
                if (step.has("waitForHeadChange")) {
                    String previous = chainRef(step.get("waitForHeadChange"), config, results);
                    for (int attempt = 0; chain.currentRunId().equals(previous) && attempt < 100; attempt++) {
                        pause(50);
                        chain = client.inspectChain(namespaceId, chainId, options);
                    }
                }
                return chainJson(chain);
            }
            case "signalChain": {
                dev.sagatoctopusorchestrator.sdk.ChainCommandOptions options =
                        dev.sagatoctopusorchestrator.sdk.ChainCommandOptions.create(chainRef(step.get("expectedRunId"), config, results));
                if (step.has("idempotencyKey")) options.idempotencyKey(step.get("idempotencyKey").asText());
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                SignalResult signal = client.signalChain(chainRef(step.get("namespaceId"), config, results), chainRef(step.get("chainId"), config, results),
                        step.get("name").asText(), step.get("payload"), options);
                ObjectNode result = F.objectNode(); ArrayNode delivered = result.putArray("deliveredTo"); signal.deliveredTo().forEach(delivered::add);
                return result;
            }
            case "cancelChain": {
                dev.sagatoctopusorchestrator.sdk.ChainCommandOptions options =
                        dev.sagatoctopusorchestrator.sdk.ChainCommandOptions.create(chainRef(step.get("expectedRunId"), config, results));
                if (step.has("idempotencyKey")) options.idempotencyKey(step.get("idempotencyKey").asText());
                if (step.has("traceparent")) options.traceparent(step.get("traceparent").asText());
                client.cancelChain(chainRef(step.get("namespaceId"), config, results), chainRef(step.get("chainId"), config, results), step.get("reason").asText(), options);
                ObjectNode result = F.objectNode(); result.put("accepted", true); return result;
            }
            case "signal": {
                SignalResult signalled = client.signal(
                        runOf(step, results), step.get("name").asText(), step.has("payload") ? step.get("payload") : F.objectNode(), CallOptions.create());
                ObjectNode result = F.objectNode();
                ArrayNode delivered = result.putArray("deliveredTo");
                signalled.deliveredTo().forEach(delivered::add);
                return result;
            }
            case "publishEvent": {
                EventInput event = EventInput.of(step.get("type").asText(), step.has("data") ? step.get("data") : F.objectNode());
                if (step.has("eventId")) {
                    event.eventId(step.get("eventId").asText());
                }
                if (step.has("correlationKey")) {
                    event.correlationKey(step.get("correlationKey").asText());
                }
                EventAccepted accepted = client.publishEvent(event, CallOptions.create());
                ObjectNode result = F.objectNode();
                result.put("eventId", accepted.eventId());
                result.put("status", accepted.statusText());
                return result;
            }
            case "waitForCompletion":
                return runJson(client.waitForCompletion(
                        runOf(step, results),
                        WaitOptions.create()
                                .timeout(Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 60_000))
                                .pollInterval(Duration.ofMillis(100))));
            case "waitForChainCompletion": {
                var chain = client.waitForChainCompletion(
                        chainRef(step.get("namespaceId"), config, results), chainRef(step.get("chainId"), config, results),
                        WaitOptions.create().timeout(Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 60_000))
                                .pollInterval(Duration.ofMillis(100)));
                return chainJson(chain);
            }
            case "events": {
                JournalPage page = client.events(runOf(step, results), null, 200);
                ObjectNode result = F.objectNode();
                ArrayNode events = result.putArray("events");
                page.items().forEach(e -> {
                    ObjectNode item = events.addObject();
                    item.put("type", e.eventType());
                    item.put("stepId", e.stepId());
                    if (e.attempt() == null) {
                        item.putNull("attempt");
                    } else {
                        item.put("attempt", e.attempt().intValue());
                    }
                });
                return result;
            }
            case "sleep":
                pause(step.get("ms").asLong());
                return null;
            case "control": {
                ObjectNode body = F.objectNode();
                body.put("action", step.get("action").asText());
                if (step.has("run")) {
                    body.put("run", runOf(step, results));
                }
                if (step.has("node")) {
                    body.put("node", step.get("node").asText());
                }
                HttpResponse<String> response = control.send(
                        HttpRequest.newBuilder(URI.create(config.get("controlUrl").asText() + "/control"))
                                .header("content-type", "application/json")
                                .POST(HttpRequest.BodyPublishers.ofString(Json.encode(body), StandardCharsets.UTF_8))
                                .build(),
                        HttpResponse.BodyHandlers.ofString());
                if (response.statusCode() < 200 || response.statusCode() >= 300) {
                    throw new IllegalStateException("control " + step.get("action").asText() + " answered " + response.statusCode());
                }
                return null;
            }
            case "startWorker":
                return startWorker(hosts.get(step.get("worker").asText()), config, plan);
            case "stopWorker": {
                WorkerEntry entry = hosts.get(step.get("worker").asText());
                if (entry != null && entry.host != null) {
                    entry.host.stop(Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 5000));
                }
                return null;
            }
            case "restartWorker": {
                // The same host object, stopped and started again (SPEC section 6.8: the host is restartable).
                WorkerEntry entry = hosts.get(step.get("worker").asText());
                if (entry == null || entry.host == null) {
                    throw new IllegalStateException("worker " + step.get("worker").asText() + " was never started");
                }
                entry.host.stop(Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 5000));
                entry.host.start();
                ObjectNode result = F.objectNode();
                result.put("protocolVersion", entry.host.protocolVersion());
                return result;
            }
            case "probeWorker": {
                WorkerConfig.Builder builder = WorkerConfig.builder()
                        .grpcUrl(config.get("grpcUrl").asText())
                        .namespace(config.get("namespace").asText())
                        .token(config.get("workerToken").asText())
                        .workerName("probe-" + unique)
                        .protocolVersion(step.get("protocolVersion").asText())
                        .retry(new RetryPolicy(1, Duration.ofMillis(20), Duration.ofMillis(200)));
                WorkerHost probe = new WorkerHost(builder.build());
                probe.registerActivity("conformance.probe", (input, ctx) -> null);
                try {
                    probe.start();
                    ObjectNode result = F.objectNode();
                    result.put("accepted", probe.protocolVersion());
                    return result;
                } finally {
                    try {
                        probe.stop(Duration.ofMillis(500));
                    } catch (RuntimeException ignored) {
                        // A probe that never started has nothing to stop.
                    }
                }
            }
            case "awaitJournal": {
                String id = runOf(step, results);
                long end = System.nanoTime() + Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 20_000).toNanos();
                for (;;) {
                    JournalPage page = client.events(id, null, 200);
                    boolean found = page.items().stream().anyMatch(e -> step.get("eventType").asText().equals(e.eventType())
                            && (!step.has("stepId") || step.get("stepId").asText().equals(e.stepId())));
                    if (found) {
                        return null;
                    }
                    if (System.nanoTime() > end) {
                        throw new IllegalStateException("the journal of " + id + " has no " + step.get("eventType").asText());
                    }
                    pause(25);
                }
            }
            case "awaitWorkerLog": {
                WorkerEntry entry = hosts.get(step.get("worker").asText());
                long end = System.nanoTime() + Duration.ofMillis(step.has("timeoutMs") ? step.get("timeoutMs").asLong() : 20_000).toNanos();
                int count = step.get("count").asInt();
                while (entry.log.size() < count && System.nanoTime() < end) {
                    pause(20);
                }
                if (entry.log.size() < count) {
                    throw new IllegalStateException("the worker log has " + entry.log.size() + " entries, not " + count);
                }
                return null;
            }
            default:
                throw new IllegalArgumentException("unknown step op " + step.get("op").asText());
        }
    }

    private static ObjectNode operation(OperationResult r) {
        ObjectNode result = F.objectNode();
        result.put("accepted", r.accepted());
        result.put("runStatus", r.runStatus());
        return result;
    }

    // ------------------------------------------------------------------------------------------- main

    public static void main(String[] args) {
        int code = 1;
        try {
            if (args.length < 2) {
                throw new IllegalArgumentException("usage: Runner build <scenario> | run <scenario> <config> <out>");
            }
            String mode = args[0];
            JsonNode raw = read(args[1]);
            if (mode.equals("build")) {
                System.out.write(Json.canonical(build(raw)).getBytes(StandardCharsets.UTF_8));
                System.out.flush();
                code = 0;
            } else if (mode.equals("run") && args.length >= 4) {
                JsonNode config = read(args[2]);
                ObjectNode out = run(templated(raw, config.get("unique").asText()), config);
                // The output is written only when the scenario finished.
                Files.writeString(new File(args[3]).toPath(), Json.encode(out), StandardCharsets.UTF_8);
                code = 0;
            } else {
                throw new IllegalArgumentException("unknown mode " + mode);
            }
        } catch (Throwable error) {
            System.err.println("the runner failed: " + error);
            error.printStackTrace();
        }
        System.exit(code);
    }
}
