package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.time.Duration;
import java.time.Instant;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The feature 018 calls that start or reset a run: signal-with-start, update-with-start and the reset of a run. The
 * client delegates to it; every check that can be made without the server is made before anything is sent, and every
 * call that changes something sends one idempotency key that all its retries reuse.
 */
final class RunOperations {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final Calls calls;

    RunOperations(Calls calls) {
        this.calls = calls;
    }

    private static String required(String value, String what) {
        if (value == null) {
            throw SagatException.invalid(what + " is required");
        }
        return value;
    }

    private static String runPath(String runId, String tail) {
        return "/runs/" + Wire.uuid(runId, "a run id") + tail;
    }

    private String workflowId(String workflow) {
        if (workflow == null || workflow.isEmpty()) {
            throw SagatException.invalid("the call needs the name or the id of a workflow");
        }
        return calls.resolveWorkflow(workflow);
    }

    /** The members the two with-start calls share. {@code deadlineIn} is turned into an instant here, once. */
    private static void withStartFields(
            ObjectNode body, String versionId, String correlationId, Instant deadline, Duration deadlineIn, String reusePolicy) {
        if (versionId != null && !versionId.isEmpty()) {
            body.put("versionId", versionId);
        }
        if (correlationId != null) {
            body.put("correlationId", correlationId);
        }
        Instant at = deadlineIn != null ? Instant.now().plus(deadlineIn) : deadline;
        if (at != null) {
            body.put("deadline", Json.instant(at));
        }
        if (reusePolicy != null) {
            body.put("businessIdReusePolicy", reusePolicy);
        }
    }

    // ---------------------------------------------------------------------------------- with start

    SignalWithStartResult signalWithStart(
            String workflow, String businessId, Object input, String signalName, Object payload, SignalWithStartOptions options) {
        SignalWithStartOptions opts = options == null ? SignalWithStartOptions.create() : options;
        String idempotencyKey = Wire.key(opts.idempotencyKey());
        String traceparent = calls.trace(opts.traceparent());
        required(businessId, "a business identifier");
        required(signalName, "a signal name");
        String workflowId = workflowId(workflow);
        ObjectNode body = F.objectNode();
        body.put("workflowId", workflowId);
        // Sent as given: no trimming, no case folding, no local pattern. The server judges the identifier.
        body.put("businessId", businessId);
        body.set("input", input == null ? F.objectNode() : Json.toTree(input));
        ObjectNode signal = body.putObject("signal");
        signal.put("name", signalName);
        signal.set("payload", payload == null ? F.objectNode() : Json.toTree(payload));
        withStartFields(body, opts.versionId(), opts.correlationId(), opts.deadline(), opts.deadlineIn(), opts.businessIdReusePolicy());
        RestClient.Response response = calls.send(new RestClient.Request(
                "POST", "/runs/signal-with-start", null, body, idempotencyKey, traceparent, true));
        return SignalWithStartResult.parse(response.body());
    }

    UpdateWithStartResult updateWithStart(
            String workflow, String businessId, Object input, String updateName, String updateKey, Object args,
            UpdateWithStartOptions options) {
        UpdateWithStartOptions opts = options == null ? UpdateWithStartOptions.create() : options;
        Wire.updateKey(updateKey);
        if (opts.expectedStateRevision() != null && opts.expectedStateRevision() < 0) {
            throw SagatException.invalid("expectedStateRevision is a non-negative integer");
        }
        String idempotencyKey = Wire.key(opts.idempotencyKey());
        String traceparent = calls.trace(opts.traceparent());
        Long startDelayMs = Wire.delayMs(opts.startDelay());
        required(businessId, "a business identifier");
        Wire.operationName(updateName);
        String workflowId = workflowId(workflow);
        ObjectNode body = F.objectNode();
        body.put("workflowId", workflowId);
        body.put("businessId", businessId);
        body.set("input", input == null ? F.objectNode() : Json.toTree(input));
        ObjectNode update = body.putObject("update");
        update.put("name", updateName);
        update.put("updateKey", updateKey);
        update.set("args", Json.toTree(args));
        if (opts.expectedStateRevision() != null) {
            update.put("expectedStateRevision", opts.expectedStateRevision().longValue());
        }
        withStartFields(body, opts.versionId(), opts.correlationId(), opts.deadline(), opts.deadlineIn(), opts.businessIdReusePolicy());
        if (opts.businessIdConflictPolicy() != null) {
            body.put("businessIdConflictPolicy", opts.businessIdConflictPolicy());
        }
        if (startDelayMs != null) {
            body.put("startDelayMs", startDelayMs.longValue());
        }
        RestClient.Response response = calls.send(new RestClient.Request(
                "POST", "/runs/update-with-start", null, body, idempotencyKey, traceparent, true));
        return UpdateWithStartResult.parse(response.body());
    }

    // ------------------------------------------------------------------------------------------ reset

    Page<ResetPoint> resetPoints(String runId, PageOptions options) {
        PageOptions opts = options == null ? PageOptions.create() : options;
        Map<String, String> query = new LinkedHashMap<>();
        query.put("cursor", opts.cursor());
        query.put("limit", opts.limit() == null ? null : Integer.toString(opts.limit()));
        String path = runPath(runId, "/reset-points");
        RestClient.Response response = calls.send(new RestClient.Request(
                "GET", path, query, null, null, calls.trace(opts.traceparent()), true));
        return Wire.page(response.body(), "a page of reset points", ResetPoint::parse);
    }

    ResetPlan previewReset(String runId, long sequence, String reapply, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        if (sequence < 1) {
            throw SagatException.invalid("sequence is a positive integer");
        }
        ObjectNode body = F.objectNode();
        body.put("sequence", sequence);
        if (reapply != null) {
            body.put("reapply", reapply);
        }
        String path = runPath(runId, "/reset-previews");
        RestClient.Response response = calls.send(new RestClient.Request(
                "POST", path, null, body, null, calls.trace(opts.traceparent()), true));
        return ResetPlan.parse(response.body());
    }

    Reset reset(String runId, ResetRequest request, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        if (request == null) {
            throw SagatException.invalid("a reset request is required");
        }
        JsonNode body = request.toJson();
        String path = runPath(runId, "/resets");
        RestClient.Response response = calls.send(new RestClient.Request(
                "POST", path, null, body, Wire.key(opts.idempotencyKey()), calls.trace(opts.traceparent()), true));
        return Reset.parse(response.body());
    }

    List<Reset> listResets(String runId, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = runPath(runId, "/resets");
        RestClient.Response response = calls.send(new RestClient.Request(
                "GET", path, null, null, null, calls.trace(opts.traceparent()), true));
        JsonNode raw = Wire.object(response.body(), "a list of resets");
        return Wire.array(raw.path("items")).stream().map(Reset::parse).toList();
    }

    Reset getReset(String runId, String resetId, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = runPath(runId, "/resets/" + Wire.uuid(resetId, "a reset id"));
        RestClient.Response response = calls.send(new RestClient.Request(
                "GET", path, null, null, null, calls.trace(opts.traceparent()), true));
        return Reset.parse(response.body());
    }
}
