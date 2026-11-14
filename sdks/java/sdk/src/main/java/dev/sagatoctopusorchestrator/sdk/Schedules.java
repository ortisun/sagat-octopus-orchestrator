package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.time.Instant;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Stream;

/**
 * The schedules of a namespace: create, describe, list, update, delete, pause, resume, trigger, backfill and their
 * history (reached from {@link SagatClient#schedules()}). Every call that changes something carries one
 * {@code Idempotency-Key}: the caller's ({@link CallOptions#idempotencyKey}) or one generated once for the call and
 * reused by every retry of it. A value the SDK can see is wrong (an id that is not a UUID, a revision below 1, an empty
 * note, text that is not an instant) is {@code VALIDATION_FAILED} and nothing is sent. Every call has a blocking form
 * and an {@code ...Async} form.
 */
public final class Schedules {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final Calls calls;

    Schedules(Calls calls) {
        this.calls = calls;
    }

    @Override
    public String toString() {
        return "Schedules";
    }

    private static String path(String namespaceId) {
        return "/namespaces/" + Wire.uuid(namespaceId, "a namespace id") + "/schedules";
    }

    private static String path(String namespaceId, String scheduleId, String tail) {
        return path(namespaceId) + "/" + Wire.uuid(scheduleId, "a schedule id") + tail;
    }

    private RestClient.Response send(
            String method, String path, Map<String, String> query, JsonNode body, CallOptions options, boolean write) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String key = write ? Wire.key(opts.idempotencyKey()) : null;
        return calls.send(new RestClient.Request(method, path, query, body, key, calls.trace(opts.traceparent()), true));
    }

    // ------------------------------------------------------------------------------------------ create

    /** Creates a schedule. A name in use is {@code CONFLICT}; the same key and body answers what the first call did. */
    public Schedule create(String namespaceId, ScheduleWrite write, CallOptions options) {
        String path = path(namespaceId);
        if (write == null) {
            throw SagatException.invalid("a schedule is an object");
        }
        return Schedule.parse(send("POST", path, null, write.toJson(), options, true).body());
    }

    public CompletableFuture<Schedule> createAsync(String namespaceId, ScheduleWrite write, CallOptions options) {
        return calls.async(() -> create(namespaceId, write, options));
    }

    // ---------------------------------------------------------------------------------------------- get

    /** A schedule with its next times, its latest decisions, the runs it has open and its counters. */
    public ScheduleDescription get(String namespaceId, String scheduleId, ScheduleGetOptions options) {
        ScheduleGetOptions opts = options == null ? ScheduleGetOptions.create() : options;
        Map<String, String> query = new LinkedHashMap<>();
        query.put("nextCount", opts.nextCount() == null ? null : Integer.toString(opts.nextCount()));
        query.put("recentCount", opts.recentCount() == null ? null : Integer.toString(opts.recentCount()));
        String path = path(namespaceId, scheduleId, "");
        return ScheduleDescription.parse(
                send("GET", path, query, null, CallOptions.create().traceparent(opts.traceparent()), false).body());
    }

    public CompletableFuture<ScheduleDescription> getAsync(String namespaceId, String scheduleId, ScheduleGetOptions options) {
        return calls.async(() -> get(namespaceId, scheduleId, options));
    }

    // --------------------------------------------------------------------------------------------- list

    /** One page of schedules. */
    public Page<Schedule> list(String namespaceId, ScheduleListFilter filter) {
        ScheduleListFilter f = filter == null ? ScheduleListFilter.create() : filter;
        Map<String, String> query = new LinkedHashMap<>();
        query.put("cursor", f.cursor());
        query.put("limit", f.limit() == null ? null : Integer.toString(f.limit()));
        query.put("state", f.state());
        query.put("origin", f.origin());
        query.put("name", f.name());
        query.put("workflowId", f.workflowId() == null ? null : Wire.uuid(f.workflowId(), "a workflow id"));
        String path = path(namespaceId);
        return Wire.page(
                send("GET", path, query, null, CallOptions.create().traceparent(f.traceparent()), false).body(),
                "a page of schedules", Schedule::parse);
    }

    public CompletableFuture<Page<Schedule>> listAsync(String namespaceId, ScheduleListFilter filter) {
        return calls.async(() -> list(namespaceId, filter));
    }

    /**
     * Every schedule the filter selects, following the opaque cursors. The stream is lazy: a page is read when the
     * previous one has been consumed. A failure while paging is raised from the stream.
     */
    public Stream<Schedule> listAll(String namespaceId, ScheduleListFilter filter) {
        ScheduleListFilter first = filter == null ? ScheduleListFilter.create() : filter;
        return Wire.all(cursor -> list(namespaceId, ScheduleListFilter.create()
                .cursor(cursor).limit(first.limit()).traceparent(first.traceparent()).state(first.state())
                .origin(first.origin()).workflowId(first.workflowId()).name(first.name())));
    }

    /** Every schedule the filter selects, collected. */
    public CompletableFuture<List<Schedule>> listAllAsync(String namespaceId, ScheduleListFilter filter) {
        return calls.async(() -> {
            try (Stream<Schedule> schedules = listAll(namespaceId, filter)) {
                return schedules.toList();
            }
        });
    }

    // ------------------------------------------------------------------------------- update and delete

    /**
     * Replaces the spec, the policies and the action as a new revision. A stale {@code expectedRevision} is
     * {@code CONFLICT} with the reason {@code REVISION_CONFLICT}.
     *
     * @param expectedRevision the revision the caller read, 1 or more
     */
    public Schedule update(String namespaceId, String scheduleId, ScheduleWrite write, long expectedRevision, CallOptions options) {
        Wire.expectedRevision(expectedRevision);
        String path = path(namespaceId, scheduleId, "");
        if (write == null) {
            throw SagatException.invalid("a schedule is an object");
        }
        ObjectNode body = write.toJson();
        body.put("expectedRevision", expectedRevision);
        return Schedule.parse(send("PUT", path, null, body, options, true).body());
    }

    public CompletableFuture<Schedule> updateAsync(
            String namespaceId, String scheduleId, ScheduleWrite write, long expectedRevision, CallOptions options) {
        return calls.async(() -> update(namespaceId, scheduleId, write, expectedRevision, options));
    }

    /** Deletes a schedule, which frees its name. The answer is empty. */
    public void delete(String namespaceId, String scheduleId, long expectedRevision, CallOptions options) {
        Wire.expectedRevision(expectedRevision);
        String path = path(namespaceId, scheduleId, "");
        Map<String, String> query = new LinkedHashMap<>();
        query.put("expectedRevision", Long.toString(expectedRevision));
        send("DELETE", path, query, null, options, true);
    }

    public CompletableFuture<Void> deleteAsync(String namespaceId, String scheduleId, long expectedRevision, CallOptions options) {
        return calls.async(() -> {
            delete(namespaceId, scheduleId, expectedRevision, options);
            return null;
        });
    }

    // ---------------------------------------------------------------------------------- pause, resume

    private Schedule note(String namespaceId, String scheduleId, String tail, String note, CallOptions options) {
        String path = path(namespaceId, scheduleId, tail);
        ObjectNode body = F.objectNode();
        body.put("note", Wire.text(note, "a note", true));
        return Schedule.parse(send("POST", path, null, body, options, true).body());
    }

    /** Stops the schedule from acting by itself. {@code note} says why and is not empty. */
    public Schedule pause(String namespaceId, String scheduleId, String note, CallOptions options) {
        return note(namespaceId, scheduleId, "/pause", note, options);
    }

    public CompletableFuture<Schedule> pauseAsync(String namespaceId, String scheduleId, String note, CallOptions options) {
        return calls.async(() -> pause(namespaceId, scheduleId, note, options));
    }

    /** Lets a paused schedule act again. {@code note} says why and is not empty. */
    public Schedule resume(String namespaceId, String scheduleId, String note, CallOptions options) {
        return note(namespaceId, scheduleId, "/resume", note, options);
    }

    public CompletableFuture<Schedule> resumeAsync(String namespaceId, String scheduleId, String note, CallOptions options) {
        return calls.async(() -> resume(namespaceId, scheduleId, note, options));
    }

    // ------------------------------------------------------------------------- trigger and backfills

    /**
     * Acts on the schedule now, also while it is paused. The answer says what became of the action.
     *
     * @param overlap the overlap policy of this action only (see {@link OverlapPolicy}); null leaves the schedule's own
     */
    public ScheduleActionRecord trigger(String namespaceId, String scheduleId, String overlap, CallOptions options) {
        String path = path(namespaceId, scheduleId, "/trigger");
        ObjectNode body = F.objectNode();
        if (overlap != null) {
            body.put("overlap", overlap);
        }
        return ScheduleActionRecord.parse(send("POST", path, null, body, options, true).body());
    }

    public CompletableFuture<ScheduleActionRecord> triggerAsync(
            String namespaceId, String scheduleId, String overlap, CallOptions options) {
        return calls.async(() -> trigger(namespaceId, scheduleId, overlap, options));
    }

    private ScheduleBackfill backfillOf(
            String namespaceId, String scheduleId, String start, String end, String overlap, CallOptions options) {
        String path = path(namespaceId, scheduleId, "/backfills");
        ObjectNode body = F.objectNode();
        body.put("start", start);
        body.put("end", end);
        if (overlap != null) {
            body.put("overlap", overlap);
        }
        return ScheduleBackfill.parse(send("POST", path, null, body, options, true).body());
    }

    /**
     * Acts on every nominal time of the past range {@code [start, end)} that has no action yet. The instants are sent
     * in UTC, as instants with a {@code Z}.
     *
     * @param overlap the overlap policy of these actions only; null leaves the schedule's own
     */
    public ScheduleBackfill backfill(
            String namespaceId, String scheduleId, Instant start, Instant end, String overlap, CallOptions options) {
        return backfillOf(namespaceId, scheduleId, Wire.when(start, "start"), Wire.when(end, "end"), overlap, options);
    }

    /**
     * The same, with the range as RFC 3339 text with any offset. Text that is not an instant (or has no offset) is
     * {@code VALIDATION_FAILED} and nothing is sent.
     */
    public ScheduleBackfill backfill(
            String namespaceId, String scheduleId, String start, String end, String overlap, CallOptions options) {
        return backfillOf(namespaceId, scheduleId, Wire.when(start, "start"), Wire.when(end, "end"), overlap, options);
    }

    public CompletableFuture<ScheduleBackfill> backfillAsync(
            String namespaceId, String scheduleId, Instant start, Instant end, String overlap, CallOptions options) {
        return calls.async(() -> backfill(namespaceId, scheduleId, start, end, overlap, options));
    }

    public CompletableFuture<ScheduleBackfill> backfillAsync(
            String namespaceId, String scheduleId, String start, String end, String overlap, CallOptions options) {
        return calls.async(() -> backfill(namespaceId, scheduleId, start, end, overlap, options));
    }

    /** Stops a backfill from acting on the times it has not reached. */
    public ScheduleBackfill cancelBackfill(String namespaceId, String scheduleId, String backfillId, CallOptions options) {
        String path = path(namespaceId, scheduleId, "/backfills/"
                + Wire.uuid(backfillId, "a backfill id") + "/cancel");
        return ScheduleBackfill.parse(send("POST", path, null, null, options, true).body());
    }

    public CompletableFuture<ScheduleBackfill> cancelBackfillAsync(
            String namespaceId, String scheduleId, String backfillId, CallOptions options) {
        return calls.async(() -> cancelBackfill(namespaceId, scheduleId, backfillId, options));
    }

    /** At most the latest 100 backfills of the schedule. */
    public List<ScheduleBackfill> listBackfills(String namespaceId, String scheduleId, CallOptions options) {
        String path = path(namespaceId, scheduleId, "/backfills");
        JsonNode raw = Wire.object(send("GET", path, null, null, options, false).body(), "a list of backfills");
        return Wire.array(raw.path("items")).stream().map(ScheduleBackfill::parse).toList();
    }

    public CompletableFuture<List<ScheduleBackfill>> listBackfillsAsync(String namespaceId, String scheduleId, CallOptions options) {
        return calls.async(() -> listBackfills(namespaceId, scheduleId, options));
    }

    /** The decisions the schedule has made, newest first. */
    public Page<ScheduleActionRecord> listActions(String namespaceId, String scheduleId, ScheduleActionFilter filter) {
        ScheduleActionFilter f = filter == null ? ScheduleActionFilter.create() : filter;
        String path = path(namespaceId, scheduleId, "/actions");
        Map<String, String> query = new LinkedHashMap<>();
        query.put("cursor", f.cursor());
        query.put("limit", f.limit() == null ? null : Integer.toString(f.limit()));
        query.put("outcome", f.outcome());
        return Wire.page(
                send("GET", path, query, null, CallOptions.create().traceparent(f.traceparent()), false).body(),
                "a page of schedule actions", ScheduleActionRecord::parse);
    }

    public CompletableFuture<Page<ScheduleActionRecord>> listActionsAsync(
            String namespaceId, String scheduleId, ScheduleActionFilter filter) {
        return calls.async(() -> listActions(namespaceId, scheduleId, filter));
    }
}
