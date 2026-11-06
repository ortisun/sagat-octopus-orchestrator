package dev.sagatoctopusorchestrator.conformance;

// The feature 018 steps of the conformance runner (schedules, business identifiers, batch jobs, the with-start calls and
// reset), and the references a step may hold. The observations are the JSON that sdks/typescript/conformance/runner.ts
// writes, member by member. Only the public API of the SDK is used.

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.BatchItem;
import dev.sagatoctopusorchestrator.sdk.BatchJob;
import dev.sagatoctopusorchestrator.sdk.BatchListOptions;
import dev.sagatoctopusorchestrator.sdk.BatchPreview;
import dev.sagatoctopusorchestrator.sdk.BatchRequest;
import dev.sagatoctopusorchestrator.sdk.BatchSelector;
import dev.sagatoctopusorchestrator.sdk.BusinessIdBinding;
import dev.sagatoctopusorchestrator.sdk.CallOptions;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.Page;
import dev.sagatoctopusorchestrator.sdk.PageOptions;
import dev.sagatoctopusorchestrator.sdk.Reset;
import dev.sagatoctopusorchestrator.sdk.ResetPlan;
import dev.sagatoctopusorchestrator.sdk.ResetPoint;
import dev.sagatoctopusorchestrator.sdk.ResetRequest;
import dev.sagatoctopusorchestrator.sdk.SagatClient;
import dev.sagatoctopusorchestrator.sdk.Schedule;
import dev.sagatoctopusorchestrator.sdk.ScheduleActionFilter;
import dev.sagatoctopusorchestrator.sdk.ScheduleActionRecord;
import dev.sagatoctopusorchestrator.sdk.ScheduleBackfill;
import dev.sagatoctopusorchestrator.sdk.ScheduleDescription;
import dev.sagatoctopusorchestrator.sdk.ScheduleGetOptions;
import dev.sagatoctopusorchestrator.sdk.ScheduleListFilter;
import dev.sagatoctopusorchestrator.sdk.ScheduleWrite;
import dev.sagatoctopusorchestrator.sdk.SignalWithStartOptions;
import dev.sagatoctopusorchestrator.sdk.SignalWithStartResult;
import dev.sagatoctopusorchestrator.sdk.UpdateWithStartOptions;
import dev.sagatoctopusorchestrator.sdk.UpdateWithStartResult;
import java.time.Duration;
import java.time.Instant;
import java.time.temporal.ChronoUnit;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;

final class Operations {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    /** The step operations this class plays. */
    private static final Set<String> OPS = Set.of(
            "scheduleCreate", "scheduleGet", "scheduleList", "scheduleUpdate", "scheduleDelete", "schedulePause", "scheduleResume",
            "scheduleTrigger", "scheduleBackfill", "scheduleCancelBackfill", "scheduleBackfills", "scheduleActions",
            "awaitSchedule", "awaitBackfill", "businessIdGet", "signalWithStart", "updateWithStart", "batchPreview",
            "batchCreate", "batchGet", "batchList", "batchCancel", "batchItems", "awaitBatch", "resetPoints", "resetPreview",
            "reset", "resets", "getReset");

    private static final Pattern WORKFLOW_REF = Pattern.compile("^\\{\\{workflow:([^}]+)\\}\\}$");
    private static final Pattern STEP_REF = Pattern.compile("^\\{\\{step:([^:}]+):([^}]+)\\}\\}$");
    private static final Pattern NOW_REF = Pattern.compile("^\\{\\{now:(-?\\d+)\\}\\}$");
    private static final Pattern HOUR_REF = Pattern.compile("^\\{\\{hour:(-?\\d+)\\}\\}$");

    private Operations() {}

    static boolean handles(String op) {
        return OPS.contains(op);
    }

    // ---------------------------------------------------------------------------------------- references

    /**
     * References inside the parameters of a step: a value an earlier step produced ({@code {{step:<id>:<field>}}}), a
     * workflow id ({@code {{workflow:<key>}}}), the namespace id, and instants computed when the step runs
     * ({@code {{now:<offsetMs>}}}, and {@code {{hour:<offsetHours>}}} for the start of a UTC hour). A string that is
     * exactly one reference becomes its value; a reference that cannot be resolved fails the step.
     */
    static JsonNode refs(JsonNode value, JsonNode config, Map<String, JsonNode> results) {
        if (value.isTextual()) {
            String text = value.textValue();
            if (text.equals("{{namespaceId}}")) {
                return config.get("namespaceId");
            }
            Matcher match = WORKFLOW_REF.matcher(text);
            if (match.matches()) {
                JsonNode workflowId = config.path("workflows").path(match.group(1)).path("workflowId");
                if (!workflowId.isTextual()) {
                    throw new IllegalStateException("no workflow " + match.group(1));
                }
                return workflowId;
            }
            match = STEP_REF.matcher(text);
            if (match.matches()) {
                JsonNode result = results.get(match.group(1));
                JsonNode found = result == null ? null : result.get(match.group(2));
                if (found == null) {
                    throw new IllegalStateException("step " + match.group(1) + " has no " + match.group(2));
                }
                return found;
            }
            match = NOW_REF.matcher(text);
            if (match.matches()) {
                return F.textNode(Json.instant(Instant.now().plusMillis(Long.parseLong(match.group(1)))));
            }
            match = HOUR_REF.matcher(text);
            if (match.matches()) {
                return F.textNode(Json.instant(Instant.now().truncatedTo(ChronoUnit.HOURS).plus(Long.parseLong(match.group(1)), ChronoUnit.HOURS)));
            }
            return value;
        }
        if (value.isArray()) {
            ArrayNode out = F.arrayNode();
            value.forEach(item -> out.add(refs(item, config, results)));
            return out;
        }
        if (value.isObject()) {
            ObjectNode out = F.objectNode();
            for (Map.Entry<String, JsonNode> entry : value.properties()) {
                out.set(entry.getKey(), refs(entry.getValue(), config, results));
            }
            return out;
        }
        return value;
    }

    // ------------------------------------------------------------------------------------------ helpers

    private static String text(JsonNode step, String name) {
        JsonNode value = step.get(name);
        return value == null || value.isNull() ? null : value.asText();
    }

    private static Integer integer(JsonNode step, String name) {
        JsonNode value = step.get(name);
        return value == null || value.isNull() ? null : Integer.valueOf(value.asInt());
    }

    private static CallOptions callOptions(JsonNode step) {
        return CallOptions.create().idempotencyKey(text(step, "idempotencyKey"));
    }

    /** A step id's result member that must be text (an id the step produced). */
    private static String stepResult(Map<String, JsonNode> results, String id, String field) {
        JsonNode result = results.get(id);
        JsonNode found = result == null ? null : result.get(field);
        if (found == null || !found.isTextual()) {
            throw new IllegalStateException("step " + id + " has no " + field);
        }
        return found.textValue();
    }

    /** The id of the step whose result holds this run, or {@code ?} for a run no step of the scenario started. */
    private static String stepOfRun(Map<String, JsonNode> results, String runId) {
        for (Map.Entry<String, JsonNode> entry : results.entrySet()) {
            JsonNode found = entry.getValue();
            if (runId.equals(found.path("runId").asText(null)) || runId.equals(found.path("newRunId").asText(null))) {
                return entry.getKey();
            }
        }
        return "?";
    }

    private interface Check {
        boolean done() throws Exception;
    }

    /** Waits for a decision of the engine: never a sleep that stands for it, only a poll of the thing itself. */
    private static void awaiting(String what, long timeoutMs, Check done) throws Exception {
        long end = System.nanoTime() + Duration.ofMillis(timeoutMs).toNanos();
        while (!done.done()) {
            if (System.nanoTime() > end) {
                throw new IllegalStateException("timed out waiting for " + what);
            }
            Thread.sleep(50);
        }
    }

    /**
     * A duration from the milliseconds a scenario gives: exactly, nanoseconds and all, so that a fraction of a
     * millisecond reaches the SDK, which is the one that drops it.
     */
    static Duration millis(JsonNode value) {
        return Duration.ofNanos(value.decimalValue().movePointRight(6).toBigInteger().longValueExact());
    }

    private static JsonNode iso(Instant value) {
        return value == null ? F.nullNode() : F.textNode(Json.instant(value));
    }

    private static JsonNode epoch(Instant value) {
        return value == null ? F.nullNode() : F.numberNode(value.toEpochMilli());
    }

    private static void putText(ObjectNode out, String name, String value) {
        if (value == null) {
            out.putNull(name);
        } else {
            out.put(name, value);
        }
    }

    private static void putNumber(ObjectNode out, String name, Long value) {
        if (value == null) {
            out.putNull(name);
        } else {
            out.put(name, value.longValue());
        }
    }

    private static ObjectNode counts(Map<String, Long> counts) {
        ObjectNode out = F.objectNode();
        counts.forEach(out::put);
        return out;
    }

    // ---------------------------------------------------------------------------------- the observations

    private static ObjectNode scheduleJson(Schedule s) {
        ObjectNode out = F.objectNode();
        out.put("scheduleId", s.scheduleId());
        out.put("name", s.name());
        out.put("origin", s.origin());
        out.put("state", s.state());
        out.put("revision", s.revision());
        out.put("exhausted", s.exhausted());
        ObjectNode action = out.putObject("action");
        putText(action, "workflowId", s.action().workflowId());
        putText(action, "versionPolicy", s.action().versionPolicy());
        putText(action, "versionId", s.action().versionId());
        putText(action, "inputDigest", s.action().inputDigest());
        putText(action, "nominalTimeField", s.action().nominalTimeField());
        putNumber(action, "runTimeoutMs", s.action().runTimeoutMs());
        putText(out, "note", s.note());
        putText(out, "noteBy", s.noteBy());
        putNumber(out, "remainingActions", s.remainingActions());
        out.set("nextActionEpochMs", epoch(s.nextActionAt()));
        out.set("createdAt", iso(s.createdAt()));
        out.set("updatedAt", iso(s.updatedAt()));
        return out;
    }

    private static ObjectNode actionJson(ScheduleActionRecord a) {
        ObjectNode out = F.objectNode();
        out.put("kind", a.kind());
        putText(out, "origin", a.origin());
        out.put("revision", a.revision());
        out.put("outcome", a.outcome());
        putText(out, "reason", a.reason());
        putText(out, "runId", a.runId());
        putText(out, "backfillId", a.backfillId());
        out.set("nominalTime", iso(a.nominalTime()));
        out.set("actualTime", iso(a.actualTime()));
        putNumber(out, "summaryCount", a.summaryCount());
        out.set("summaryLastNominalTime", iso(a.summaryLastNominalTime()));
        out.set("decidedAt", iso(a.decidedAt()));
        return out;
    }

    private static ObjectNode descriptionJson(ScheduleDescription d) {
        ObjectNode out = scheduleJson(d.schedule());
        ArrayNode next = out.putArray("nextActions");
        d.nextActions().forEach(n -> {
            ObjectNode item = next.addObject();
            item.put("nominalTimeEpochMs", n.nominalTime().toEpochMilli());
            item.put("actualTimeEpochMs", n.actualTime().toEpochMilli());
        });
        ArrayNode recent = out.putArray("recentActions");
        d.recentActions().forEach(a -> recent.add(actionJson(a)));
        ArrayNode openRuns = out.putArray("openRuns");
        d.openRuns().forEach(openRuns::add);
        out.put("bufferedCount", d.bufferedCount());
        out.set("counters", counts(d.counters()));
        return out;
    }

    private static ObjectNode backfillJson(ScheduleBackfill b) {
        ObjectNode out = F.objectNode();
        out.put("backfillId", b.backfillId());
        out.set("start", iso(b.start()));
        out.set("end", iso(b.end()));
        out.set("cursor", iso(b.cursor()));
        out.put("state", b.state());
        out.put("overlap", b.overlap());
        out.set("counts", counts(b.counts()));
        return out;
    }

    private static ObjectNode jobJson(BatchJob j) {
        ObjectNode out = F.objectNode();
        out.put("batchId", j.batchId());
        out.put("action", j.action());
        out.put("state", j.state());
        putText(out, "failureReason", j.failureReason());
        out.put("createdBy", j.createdBy());
        out.put("hasPayload", j.hasPayload());
        putNumber(out, "maxItems", j.maxItems());
        putNumber(out, "itemsPerSecond", j.itemsPerSecond());
        out.set("counts", countsOf(j.counts()));
        out.set("createdAt", iso(j.createdAt()));
        out.set("startedAt", iso(j.startedAt()));
        out.set("finishedAt", iso(j.finishedAt()));
        out.set("expiresAt", iso(j.expiresAt()));
        return out;
    }

    private static ObjectNode countsOf(BatchJob.Counts c) {
        ObjectNode out = F.objectNode();
        out.put("selected", c.selected());
        out.put("succeeded", c.succeeded());
        out.put("skipped", c.skipped());
        out.put("failed", c.failed());
        out.put("cancelled", c.cancelled());
        out.put("deferred", c.deferred());
        return out;
    }

    private static ObjectNode resetJson(Reset r) {
        ObjectNode out = F.objectNode();
        out.put("resetId", r.resetId());
        out.put("sourceRunId", r.sourceRunId());
        out.put("sequence", r.sequence());
        out.put("state", r.state());
        putText(out, "failureReason", r.failureReason());
        putText(out, "newRunId", r.newRunId());
        ArrayNode report = out.putArray("reapplyReport");
        r.reapplyReport().forEach(report::add);
        out.set("createdAt", iso(r.createdAt()));
        return out;
    }

    private static JsonNode receiptJson(UpdateWithStartResult.Receipt r) {
        if (r == null) {
            return F.nullNode();
        }
        ObjectNode out = F.objectNode();
        out.put("operation", r.operation());
        out.put("updateKey", r.updateKey());
        out.put("status", r.status());
        if (r.result() != null) {
            out.set("result", r.result());
        }
        out.put("beforeStateRevision", r.beforeStateRevision());
        out.put("afterStateRevision", r.afterStateRevision());
        out.put("runVersion", r.runVersion());
        out.put("replayed", r.replayed());
        return out;
    }

    private static JsonNode cursor(String nextCursor) {
        return nextCursor == null ? F.nullNode() : F.textNode("<cursor>");
    }

    // ----------------------------------------------------------------------------------- the requests

    /** Fails on a member of the scenario the runner does not map, so that a request is never silently narrowed. */
    private static void known(JsonNode node, String what, String... names) {
        List<String> allowed = List.of(names);
        for (Iterator<String> it = node.fieldNames(); it.hasNext(); ) {
            String name = it.next();
            if (!allowed.contains(name)) {
                throw new IllegalArgumentException("the runner does not know the member " + name + " of " + what);
            }
        }
    }

    private static ScheduleWrite.Range[] rangesOf(JsonNode array) {
        List<ScheduleWrite.Range> ranges = new ArrayList<>();
        for (JsonNode item : array) {
            known(item, "range", "start", "end", "step");
            ranges.add(new ScheduleWrite.Range(
                    item.get("start").asLong(),
                    item.has("end") ? Long.valueOf(item.get("end").asLong()) : null,
                    item.has("step") ? Long.valueOf(item.get("step").asLong()) : null));
        }
        return ranges.toArray(new ScheduleWrite.Range[0]);
    }

    private static ScheduleWrite.Calendar calendarOf(JsonNode node) {
        known(node, "calendar", "second", "minute", "hour", "dayOfMonth", "month", "year", "dayOfWeek", "comment");
        ScheduleWrite.Calendar calendar = ScheduleWrite.Calendar.create();
        if (node.has("second")) calendar.second(rangesOf(node.get("second")));
        if (node.has("minute")) calendar.minute(rangesOf(node.get("minute")));
        if (node.has("hour")) calendar.hour(rangesOf(node.get("hour")));
        if (node.has("dayOfMonth")) calendar.dayOfMonth(rangesOf(node.get("dayOfMonth")));
        if (node.has("month")) calendar.month(rangesOf(node.get("month")));
        if (node.has("year")) calendar.year(rangesOf(node.get("year")));
        if (node.has("dayOfWeek")) calendar.dayOfWeek(rangesOf(node.get("dayOfWeek")));
        if (node.has("comment")) calendar.comment(node.get("comment").asText());
        return calendar;
    }

    private static ScheduleWrite scheduleWriteOf(JsonNode node) {
        known(node, "schedule", "name", "spec", "policies", "action", "paused", "note", "remainingActions");
        JsonNode spec = node.path("spec");
        known(spec, "spec", "calendars", "cron", "intervals", "exclusions", "startAt", "endAt", "timezone", "jitterMs", "dst");
        ScheduleWrite.Spec s = ScheduleWrite.Spec.create();
        spec.path("calendars").forEach(c -> s.calendar(calendarOf(c)));
        spec.path("cron").forEach(c -> s.cron(c.asText()));
        spec.path("intervals").forEach(i -> {
            known(i, "interval", "everyMs", "offsetMs");
            s.interval(new ScheduleWrite.Interval(i.get("everyMs").asLong(), i.has("offsetMs") ? Long.valueOf(i.get("offsetMs").asLong()) : null));
        });
        spec.path("exclusions").forEach(c -> s.exclusion(calendarOf(c)));
        // The instants are handed over as the RFC 3339 text the scenario holds: the SDK is the one that reads and writes them.
        if (spec.has("startAt")) s.startAt(spec.get("startAt").asText());
        if (spec.has("endAt")) s.endAt(spec.get("endAt").asText());
        if (spec.has("timezone")) s.timezone(spec.get("timezone").asText());
        if (spec.has("jitterMs")) s.jitterMs(spec.get("jitterMs").asLong());
        if (spec.has("dst")) {
            known(spec.get("dst"), "dst", "gap", "ambiguous");
            s.dst(new ScheduleWrite.Dst(text(spec.get("dst"), "gap"), text(spec.get("dst"), "ambiguous")));
        }

        JsonNode action = node.path("action");
        known(action, "action", "workflowId", "versionPolicy", "versionId", "input", "nominalTimeField", "runTimeoutMs");
        ScheduleWrite.Action a = ScheduleWrite.Action.of(text(action, "workflowId"), action.get("input"));
        if (action.has("versionPolicy")) a.versionPolicy(action.get("versionPolicy").asText());
        if (action.has("versionId")) a.versionId(action.get("versionId").asText());
        if (action.has("nominalTimeField")) a.nominalTimeField(action.get("nominalTimeField").asText());
        if (action.has("runTimeoutMs")) a.runTimeoutMs(action.get("runTimeoutMs").asLong());

        ScheduleWrite write = ScheduleWrite.of(text(node, "name"), s, a);
        if (node.has("policies")) {
            JsonNode policies = node.get("policies");
            known(policies, "policies", "overlap", "catchupWindowMs", "catchupMode", "pauseOnFailure", "bufferLimit");
            ScheduleWrite.Policies p = ScheduleWrite.Policies.create();
            if (policies.has("overlap")) p.overlap(policies.get("overlap").asText());
            if (policies.has("catchupWindowMs")) p.catchupWindowMs(policies.get("catchupWindowMs").asLong());
            if (policies.has("catchupMode")) p.catchupMode(policies.get("catchupMode").asText());
            if (policies.has("pauseOnFailure")) p.pauseOnFailure(policies.get("pauseOnFailure").asBoolean());
            if (policies.has("bufferLimit")) p.bufferLimit(policies.get("bufferLimit").asInt());
            write.policies(p);
        }
        if (node.has("paused")) write.paused(node.get("paused").asBoolean());
        if (node.has("note")) write.note(node.get("note").asText());
        if (node.has("remainingActions")) write.remainingActions(node.get("remainingActions").asLong());
        return write;
    }

    private static List<String> strings(JsonNode array) {
        List<String> out = new ArrayList<>();
        array.forEach(item -> out.add(item.asText()));
        return out;
    }

    private static BatchSelector selectorOf(JsonNode node) {
        known(node, "selector", "filter", "runIds", "query");
        if (node.has("filter")) {
            JsonNode f = node.get("filter");
            known(f, "filter", "workflowId", "versionId", "status", "origin", "scheduleId", "createdAfter", "createdBefore", "businessIdPrefix");
            BatchSelector.Filter filter = BatchSelector.Filter.create();
            if (f.has("workflowId")) filter.workflowId(f.get("workflowId").asText());
            if (f.has("versionId")) filter.versionId(f.get("versionId").asText());
            if (f.has("status")) filter.status(strings(f.get("status")));
            if (f.has("origin")) filter.origin(f.get("origin").asText());
            if (f.has("scheduleId")) filter.scheduleId(f.get("scheduleId").asText());
            if (f.has("createdAfter")) filter.createdAfter(f.get("createdAfter").asText());
            if (f.has("createdBefore")) filter.createdBefore(f.get("createdBefore").asText());
            if (f.has("businessIdPrefix")) filter.businessIdPrefix(f.get("businessIdPrefix").asText());
            return BatchSelector.filter(filter);
        }
        if (node.has("runIds")) {
            return BatchSelector.runIds(strings(node.get("runIds")));
        }
        if (node.has("query")) {
            return BatchSelector.query(node.get("query").asText());
        }
        throw new IllegalArgumentException("the runner does not know this selector");
    }

    private static BatchRequest batchRequestOf(JsonNode node) {
        known(node, "batch request", "selector", "action", "signal", "update", "stepId", "businessId", "reason", "maxItems", "itemsPerSecond");
        BatchRequest request = BatchRequest.of(selectorOf(node.get("selector")), text(node, "action"), text(node, "reason"));
        if (node.has("signal")) request.signal(node.get("signal").path("name").asText(), node.get("signal").get("payload"));
        if (node.has("update")) request.update(node.get("update").path("name").asText(), node.get("update").get("args"));
        if (node.has("stepId")) request.stepId(node.get("stepId").asText());
        if (node.has("businessId")) request.businessId(node.get("businessId").asText());
        if (node.has("maxItems")) request.maxItems(node.get("maxItems").asInt());
        if (node.has("itemsPerSecond")) request.itemsPerSecond(node.get("itemsPerSecond").asInt());
        return request;
    }

    // -------------------------------------------------------------------------------------------- steps

    /** Plays one of the steps {@link #handles}. A result of null is a step that observes nothing. */
    static JsonNode play(JsonNode step, SagatClient client, JsonNode config, Map<String, JsonNode> results) throws Exception {
        String namespaceId = config.get("namespaceId").asText();
        switch (step.get("op").asText()) {
            case "scheduleCreate":
                return scheduleJson(client.schedules().create(namespaceId, scheduleWriteOf(step.get("write")), callOptions(step)));
            case "scheduleGet": {
                ScheduleGetOptions options = ScheduleGetOptions.create()
                        .nextCount(integer(step, "nextCount")).recentCount(integer(step, "recentCount"));
                return descriptionJson(client.schedules().get(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"), options));
            }
            case "scheduleList": {
                ScheduleListFilter filter = ScheduleListFilter.create().limit(integer(step, "limit"));
                JsonNode given = step.path("filter");
                known(given, "filter", "name", "state", "origin", "workflowId");
                filter.name(text(given, "name")).state(text(given, "state")).origin(text(given, "origin")).workflowId(text(given, "workflowId"));
                ArrayNode items = F.arrayNode();
                String nextCursor = null;
                if (step.path("all").asBoolean(false)) {
                    try (Stream<Schedule> stream = client.schedules().listAll(namespaceId, filter)) {
                        stream.forEach(s -> items.add(listed(s)));
                    }
                } else {
                    Page<Schedule> page = client.schedules().list(namespaceId, filter);
                    page.items().forEach(s -> items.add(listed(s)));
                    nextCursor = page.nextCursor();
                }
                ObjectNode result = F.objectNode();
                result.set("items", items);
                result.set("nextCursor", cursor(nextCursor));
                return result;
            }
            case "scheduleUpdate":
                return scheduleJson(client.schedules().update(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        scheduleWriteOf(step.get("write")), step.path("expectedRevision").asLong(0), callOptions(step)));
            case "scheduleDelete": {
                client.schedules().delete(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        step.path("expectedRevision").asLong(0), callOptions(step));
                ObjectNode result = F.objectNode();
                result.put("deleted", true);
                return result;
            }
            case "schedulePause":
                return scheduleJson(client.schedules().pause(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        text(step, "note"), callOptions(step)));
            case "scheduleResume":
                return scheduleJson(client.schedules().resume(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        text(step, "note"), callOptions(step)));
            case "scheduleTrigger":
                return actionJson(client.schedules().trigger(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        text(step, "overlap"), callOptions(step)));
            case "scheduleBackfill":
                return backfillJson(client.schedules().backfill(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        text(step, "start"), text(step, "end"), text(step, "overlap"), callOptions(step)));
            case "scheduleCancelBackfill":
                return backfillJson(client.schedules().cancelBackfill(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        stepResult(results, step.get("backfill").asText(), "backfillId"), callOptions(step)));
            case "scheduleBackfills": {
                ObjectNode result = F.objectNode();
                ArrayNode items = result.putArray("items");
                client.schedules().listBackfills(namespaceId, stepResult(results, step.get("schedule").asText(), "scheduleId"), null)
                        .forEach(b -> items.add(backfillJson(b)));
                return result;
            }
            case "scheduleActions": {
                Page<ScheduleActionRecord> page = client.schedules().listActions(namespaceId,
                        stepResult(results, step.get("schedule").asText(), "scheduleId"),
                        ScheduleActionFilter.create().outcome(text(step, "outcome")).limit(integer(step, "limit")));
                ObjectNode result = F.objectNode();
                ArrayNode items = result.putArray("items");
                page.items().forEach(a -> items.add(actionJson(a)));
                result.set("nextCursor", cursor(page.nextCursor()));
                return result;
            }
            case "awaitSchedule": {
                // The scheduler decides the times of a backfill: wait for its counters, never for a delay.
                String id = stepResult(results, step.get("schedule").asText(), "scheduleId");
                String counter = step.get("counter").asText();
                long atLeast = step.path("atLeast").asLong(1);
                awaiting("schedule counter " + counter, step.path("timeoutMs").asLong(30_000), () -> client.schedules()
                        .get(namespaceId, id, ScheduleGetOptions.create().nextCount(0).recentCount(0))
                        .counters().getOrDefault(counter, 0L) >= atLeast);
                return null;
            }
            case "awaitBackfill": {
                // A backfill counts the times it found already acted on in its own counts, not in the schedule's counters.
                String schedule = stepResult(results, step.get("schedule").asText(), "scheduleId");
                String id = stepResult(results, step.get("backfill").asText(), "backfillId");
                String state = step.get("state").asText();
                awaiting("backfill state " + state, step.path("timeoutMs").asLong(30_000), () -> client.schedules()
                        .listBackfills(namespaceId, schedule, null).stream()
                        .anyMatch(b -> b.backfillId().equals(id) && b.state().equals(state)));
                return null;
            }
            case "businessIdGet": {
                BusinessIdBinding b = client.businessIds().get(namespaceId, text(step, "businessId"), null);
                ObjectNode result = F.objectNode();
                result.put("businessId", b.businessId());
                result.put("executionId", b.executionId());
                result.put("currentRunId", b.currentRunId());
                result.put("open", b.open());
                putText(result, "lastStatus", b.lastStatus());
                result.put("workflowId", b.workflowId());
                result.set("boundAt", iso(b.boundAt()));
                result.set("reusableAfter", iso(b.reusableAfter()));
                return result;
            }
            case "signalWithStart": {
                String workflow = step.has("workflow")
                        ? config.path("workflows").path(step.get("workflow").asText()).path("workflowId").asText()
                        : text(step, "workflowId");
                JsonNode signal = step.get("signal");
                SignalWithStartResult r = client.signalWithStart(workflow, text(step, "businessId"),
                        step.has("input") ? step.get("input") : F.objectNode(), signal.path("name").asText(),
                        signal.has("payload") ? signal.get("payload") : F.objectNode(),
                        SignalWithStartOptions.create().idempotencyKey(text(step, "idempotencyKey"))
                                .businessIdReusePolicy(text(step, "businessIdReusePolicy")));
                ObjectNode result = F.objectNode();
                result.put("runId", r.runId());
                result.put("started", r.started());
                ArrayNode delivered = result.putArray("deliveredTo");
                r.deliveredTo().forEach(delivered::add);
                return result;
            }
            case "updateWithStart": {
                JsonNode u = step.get("update");
                UpdateWithStartOptions options = UpdateWithStartOptions.create()
                        .idempotencyKey(text(step, "idempotencyKey")).businessIdConflictPolicy(text(step, "businessIdConflictPolicy"))
                        .businessIdReusePolicy(text(step, "businessIdReusePolicy"));
                if (step.has("startDelayMs")) options.startDelay(millis(step.get("startDelayMs")));
                if (u.has("expectedStateRevision")) options.expectedStateRevision(u.get("expectedStateRevision").asLong());
                UpdateWithStartResult r = client.updateWithStart(
                        config.path("workflows").path(step.get("workflow").asText()).path("workflowId").asText(),
                        text(step, "businessId"), step.has("input") ? step.get("input") : F.objectNode(),
                        u.path("name").asText(), u.path("updateKey").asText(), u.has("args") ? u.get("args") : F.nullNode(), options);
                ObjectNode result = F.objectNode();
                result.put("runId", r.runId());
                result.put("started", r.started());
                ObjectNode update = result.putObject("update");
                update.put("outcome", r.update().outcome());
                putText(update, "reason", r.update().reason());
                update.set("receipt", receiptJson(r.update().receipt()));
                return result;
            }
            case "batchPreview": {
                BatchPreview p = client.batch().preview(namespaceId, selectorOf(step.get("selector")), integer(step, "maxItems"), null);
                // The sample is a set of runs in the order the server chose; the scenario compares which of its own runs it holds.
                ObjectNode result = F.objectNode();
                result.put("matched", p.matched());
                result.put("capped", p.capped());
                result.put("sampleSize", p.sample().size());
                ArrayNode steps = result.putArray("sampleSteps");
                p.sample().stream().map(id -> stepOfRun(results, id)).sorted().forEach(steps::add);
                return result;
            }
            case "batchCreate": {
                // What does not change while the job runs (its state and counters do): the job as it was accepted.
                BatchJob j = client.batch().create(namespaceId, batchRequestOf(step.get("request")), callOptions(step));
                ObjectNode result = F.objectNode();
                result.put("batchId", j.batchId());
                result.put("action", j.action());
                result.put("createdBy", j.createdBy());
                result.put("hasPayload", j.hasPayload());
                putNumber(result, "maxItems", j.maxItems());
                putNumber(result, "itemsPerSecond", j.itemsPerSecond());
                return result;
            }
            case "batchGet":
                return jobJson(client.batch().get(namespaceId, stepResult(results, step.get("batch").asText(), "batchId"), null));
            case "batchList": {
                Page<BatchJob> page = client.batch().list(namespaceId, BatchListOptions.create().state(text(step, "state")).limit(integer(step, "limit")));
                ObjectNode result = F.objectNode();
                ArrayNode items = result.putArray("items");
                page.items().forEach(j -> {
                    ObjectNode item = items.addObject();
                    item.put("action", j.action());
                    item.put("state", j.state());
                    item.set("counts", countsOf(j.counts()));
                });
                result.set("nextCursor", cursor(page.nextCursor()));
                return result;
            }
            case "batchCancel":
                return jobJson(client.batch().cancel(namespaceId, stepResult(results, step.get("batch").asText(), "batchId"),
                        text(step, "reason"), callOptions(step)));
            case "batchItems": {
                Page<BatchItem> page = client.batch().listItems(namespaceId, stepResult(results, step.get("batch").asText(), "batchId"),
                        BatchListOptions.create().state(text(step, "state")).limit(integer(step, "limit")));
                // The order of the items is the server's; which of the scenario's runs sits at which ordinal is not part of
                // the contract, so the items are named by the step that started the run and sorted by it.
                List<ObjectNode> items = new ArrayList<>();
                for (BatchItem i : page.items()) {
                    ObjectNode item = F.objectNode();
                    item.put("runStep", stepOfRun(results, i.runId()));
                    item.put("state", i.state());
                    putText(item, "reason", i.reason());
                    putText(item, "errorCode", i.errorCode());
                    item.put("attempts", i.attempts());
                    putText(item, "newRunId", i.newRunId());
                    items.add(item);
                }
                items.sort(Comparator.comparing(item -> item.get("runStep").textValue()));
                ObjectNode result = F.objectNode();
                ArrayNode array = result.putArray("items");
                items.forEach(array::add);
                result.set("nextCursor", cursor(page.nextCursor()));
                return result;
            }
            case "awaitBatch": {
                String id = stepResult(results, step.get("batch").asText(), "batchId");
                String state = step.get("state").asText();
                awaiting("batch state " + state, step.path("timeoutMs").asLong(30_000),
                        () -> client.batch().get(namespaceId, id, null).state().equals(state));
                return null;
            }
            case "resetPoints": {
                Page<ResetPoint> page = client.resetPoints(Runner.runOf(step, results), PageOptions.create().limit(integer(step, "limit")));
                ObjectNode result = F.objectNode();
                ArrayNode items = result.putArray("items");
                page.items().forEach(p -> {
                    ObjectNode item = items.addObject();
                    item.put("sequence", p.sequence());
                    item.put("kind", p.kind());
                    putText(item, "node", p.node());
                    item.set("occurredAt", iso(p.occurredAt()));
                });
                result.set("nextCursor", cursor(page.nextCursor()));
                return result;
            }
            case "resetPreview": {
                ResetPlan plan = client.previewReset(Runner.runOf(step, results), step.path("sequence").asLong(0), text(step, "reapply"), null);
                ObjectNode result = F.objectNode();
                result.put("sequence", plan.sequence());
                ArrayNode restored = result.putArray("restored");
                plan.restored().forEach(r -> {
                    ObjectNode item = restored.addObject();
                    item.put("node", r.node());
                    item.put("kind", r.kind());
                });
                ArrayNode reexecuted = result.putArray("reexecuted");
                plan.reexecuted().forEach(r -> {
                    ObjectNode item = reexecuted.addObject();
                    item.put("node", r.node());
                    item.put("completedInSource", r.completedInSource());
                    ArrayNode markers = item.putArray("markers");
                    r.markers().forEach(markers::add);
                });
                ArrayNode reapply = result.putArray("reapply");
                plan.reapply().forEach(reapply::add);
                result.put("requiresAcknowledgement", plan.requiresAcknowledgement());
                result.put("prefixEvents", plan.prefixEvents());
                return result;
            }
            case "reset": {
                ResetRequest request = ResetRequest.of(step.path("sequence").asLong(0), text(step, "reason"))
                        .reapply(text(step, "reapply")).sourcePolicy(text(step, "sourcePolicy")).businessId(text(step, "businessId"));
                if (step.has("acknowledgeReexecution")) request.acknowledgeReexecution(step.get("acknowledgeReexecution").asBoolean());
                return resetJson(client.reset(Runner.runOf(step, results), request, callOptions(step)));
            }
            case "resets": {
                ObjectNode result = F.objectNode();
                ArrayNode items = result.putArray("items");
                client.listResets(Runner.runOf(step, results), null).forEach(r -> items.add(resetJson(r)));
                return result;
            }
            case "getReset":
                return resetJson(client.getReset(Runner.runOf(step, results), stepResult(results, step.get("reset").asText(), "resetId"), null));
            default:
                throw new IllegalArgumentException("unknown step op " + step.get("op").asText());
        }
    }

    private static ObjectNode listed(Schedule s) {
        ObjectNode item = F.objectNode();
        item.put("name", s.name());
        item.put("state", s.state());
        item.put("origin", s.origin());
        item.put("revision", s.revision());
        return item;
    }
}
