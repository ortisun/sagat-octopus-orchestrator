package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * The feature 018 client surface against a recording server (mirrors operations.test.ts): what each call sends, what it
 * keeps across a retry, what it refuses before sending, and how it reads an answer that is newer than the SDK.
 */
@Timeout(60)
class OperationsTest {
    private static final String NS = "0190aaaa-0000-7000-8000-00000000aaaa";
    private static final String WF = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
    private static final String SCHEDULE = "0190aaaa-0000-7000-8000-0000000000a1";
    private static final String RUN = "0190aaaa-0000-7000-8000-000000000001";
    private static final String BATCH = "0190aaaa-0000-7000-8000-0000000000b1";
    private static final String UNAVAILABLE =
            "{\"code\":\"UNAVAILABLE\",\"message\":\"busy\",\"retryable\":true,\"traceId\":\"" + "e".repeat(32) + "\"}";
    private static FakeRestServer server;

    @BeforeAll
    static void startServer() throws IOException {
        server = new FakeRestServer();
    }

    @AfterAll
    static void stopServer() {
        server.close();
    }

    // ------------------------------------------------------------------------------------------ fixtures

    /** JSON written with single quotes, for readability. */
    private static String j(String text) {
        return text.replace('\'', '"');
    }

    private static String fields(Map<String, String> base, String... overrides) {
        Map<String, String> all = new LinkedHashMap<>(base);
        for (int i = 0; i + 1 < overrides.length; i += 2) {
            all.put(overrides[i], overrides[i + 1]);
        }
        StringBuilder out = new StringBuilder("{");
        all.forEach((k, v) -> out.append(out.length() > 1 ? "," : "").append('"').append(k).append("\":").append(v));
        return out.append('}').toString();
    }

    private static String run(String... overrides) {
        Map<String, String> base = new LinkedHashMap<>();
        base.put("runId", "'" + RUN + "'");
        base.put("workflowId", "'" + WF + "'");
        base.put("versionId", "'0190aaaa-0000-7000-8000-000000000002'");
        base.put("status", "'RUNNING'");
        base.put("version", "3");
        base.put("paused", "false");
        base.put("startedBy", "'alice'");
        base.put("createdAt", "'2026-10-10T10:00:00Z'");
        base.put("updatedAt", "'2026-10-10T10:00:01Z'");
        base.put("finishedAt", "null");
        base.put("deadline", "null");
        base.put("traceId", "'abcdefabcdefabcdefabcdefabcdefab'");
        base.put("correlationId", "null");
        base.put("depth", "0");
        base.put("parent", "null");
        return j(fields(base, overrides));
    }

    private static String schedule(String... overrides) {
        Map<String, String> base = new LinkedHashMap<>();
        base.put("scheduleId", "'" + SCHEDULE + "'");
        base.put("name", "'nightly'");
        base.put("origin", "'API'");
        base.put("state", "'ACTIVE'");
        base.put("revision", "2");
        base.put("createdAt", "'2026-10-10T10:00:00Z'");
        base.put("action", "{'workflowId':'" + WF + "','versionPolicy':'ACTIVE','inputDigest':'" + "a".repeat(64) + "'}");
        base.put("note", "null");
        base.put("remainingActions", "null");
        base.put("nextActionAt", "'2099-01-01T09:00:00Z'");
        base.put("updatedAt", "'2026-10-10T10:00:00Z'");
        return j(fields(base, overrides));
    }

    private static String action(String... overrides) {
        Map<String, String> base = new LinkedHashMap<>();
        base.put("nominalTime", "'2026-10-10T10:00:00Z'");
        base.put("actualTime", "'2026-10-10T10:00:01Z'");
        base.put("kind", "'TRIGGERED'");
        base.put("origin", "'MANUAL'");
        base.put("revision", "2");
        base.put("outcome", "'STARTED'");
        base.put("reason", "null");
        base.put("runId", "'" + RUN + "'");
        base.put("backfillId", "null");
        base.put("decidedAt", "'2026-10-10T10:00:01Z'");
        return j(fields(base, overrides));
    }

    private static SagatClient client(Consumer<ClientConfig.Builder> tweak) {
        ClientConfig.Builder builder = ClientConfig.builder()
                .apiUrl(server.base())
                .namespace("payments")
                .token("tok-1")
                .retry(new RetryPolicy(4, Duration.ofMillis(5), Duration.ofMillis(20)));
        tweak.accept(builder);
        return new SagatClient(builder.build());
    }

    private static SagatClient client() {
        return client(b -> {});
    }

    private static FakeRestServer.Seen only() {
        assertEquals(1, server.seen.size(), "exactly one request");
        return server.seen.get(0);
    }

    private static CallOptions key(String value) {
        return CallOptions.create().idempotencyKey(value);
    }

    private static void refused(Runnable call) {
        SagatException error = assertThrows(SagatException.class, call::run);
        assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        assertTrue(server.seen.isEmpty(), "a refusal sends nothing");
    }

    private static String body(FakeRestServer.Seen request) {
        return request.rawBody();
    }

    // ------------------------------------------------------------------------------------ start options

    @Test
    void aStartSendsTheBusinessIdentifierItsPoliciesAndTheDelayOnceAndSaysWhetherItCreatedTheRun() {
        server.prepare((req, reply) -> reply.send(202, run("businessId", "'order-1'", "startDelayedUntil", "'2026-10-10T10:00:05Z'")));
        StartedRun started = client().startResult(WF, Map.of("a", 1), StartOptions.create()
                .idempotencyKey("key-12345678").businessId("Order/1 ")
                .businessIdConflictPolicy(BusinessIdConflictPolicy.USE_EXISTING)
                .businessIdReusePolicy(BusinessIdReusePolicy.REJECT_DUPLICATE)
                .startDelay(Duration.ofNanos(5_000_900_000L)));
        assertEquals(j("{'workflowId':'" + WF + "','input':{'a':1},'businessId':'Order/1 ','businessIdConflictPolicy':'USE_EXISTING',"
                + "'businessIdReusePolicy':'REJECT_DUPLICATE','startDelayMs':5000}"), body(only()),
                "the identifier is sent as given and the delay is whole milliseconds, truncated");
        assertTrue(started.started());
        assertEquals("order-1", started.run().businessId());
        assertEquals(Instant.parse("2026-10-10T10:00:05Z"), started.run().startDelayedUntil());

        server.prepare((req, reply) -> reply.send(202, run()));
        StartedRun plain = client().startResult(WF, Map.of(), StartOptions.create().idempotencyKey("key-12345678"));
        assertNull(plain.run().businessId());
        assertNull(plain.run().startDelayedUntil());
    }

    @Test
    void the200OfUseExistingIsARunThatWasNotStartedHereAndAStartWithoutTheNewOptionsSendsNoneOfThem() {
        server.prepare((req, reply) -> reply.send(200, run()));
        StartedRun existing = client().startResult(WF, Map.of(), StartOptions.create().idempotencyKey("key-12345678")
                .businessId("o-1").businessIdConflictPolicy(BusinessIdConflictPolicy.USE_EXISTING));
        assertFalse(existing.started());
        assertEquals(RUN, existing.runId());
        server.prepare((req, reply) -> reply.send(202, run()));
        // `start` still answers a Run (a caller that holds it by that type keeps compiling: Run is a final record)
        Run old = client().start(WF, Map.of(), StartOptions.create().idempotencyKey("key-12345678"));
        assertEquals(RUN, old.runId());
        assertEquals("{\"workflowId\":\"" + WF + "\",\"input\":{}}", body(only()), "an old call is byte for byte what it was");
    }

    @Test
    void aRetryKeepsTheOneDelayItComputedAndTheKeyAndADurationTooBigForAnIntegerIsLeftToTheServer() {
        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 2) {
                reply.send(503, UNAVAILABLE);
            } else {
                reply.send(202, run());
            }
        });
        client().start(WF, Map.of(), StartOptions.create().businessId("o-2").startDelay(Duration.ofMillis(1500)));
        assertEquals(2, server.seen.size());
        assertEquals(body(server.seen.get(0)), body(server.seen.get(1)), "the retry is the same body");
        assertTrue(body(server.seen.get(0)).contains("\"startDelayMs\":1500"));
        assertEquals(server.seen.get(0).header("idempotency-key"), server.seen.get(1).header("idempotency-key"));

        // Truncation is toward zero, also below zero; the range is the server's to judge, so nothing is refused here.
        server.prepare((req, reply) -> reply.send(202, run()));
        client().start(WF, Map.of(), StartOptions.create().startDelay(Duration.ofNanos(-1_500_000)));
        assertTrue(body(only()).contains("\"startDelayMs\":-1"), body(only()));
        server.prepare((req, reply) -> reply.send(202, run()));
        client().start(WF, Map.of(), StartOptions.create().startDelay(Duration.ofSeconds(Long.MAX_VALUE)));
        assertTrue(body(only()).contains("\"startDelayMs\":" + Long.MAX_VALUE), body(only()));
    }

    // ---------------------------------------------------------------------------------- where a run came from

    @Test
    void aRunSaysWhichScheduleStartedItByWhichKindOfActionForWhichNominalTimeAndUnderWhichRevisionAndARunNoScheduleStartedHasNone() {
        server.prepare((req, reply) -> reply.send(200, run(
                "startedBy", "'system:schedule:" + SCHEDULE + "'",
                "schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00.123456Z','kind':'BACKFILL','revision':3}")));
        Run scheduled = client().get(RUN);
        assertEquals(new Run.ScheduleOfRun(SCHEDULE, Instant.parse("2026-11-04T02:00:00.123456Z"), "BACKFILL", 3), scheduled.schedule());
        // the instant is read, not carried: 2026-11-04T02:00:00Z is second 1793757600, and the microseconds are kept
        assertEquals(Instant.ofEpochSecond(1_793_757_600L, 123_456_000L), scheduled.schedule().nominalTime());
        assertEquals(123_456_000, scheduled.schedule().nominalTime().getNano(), "the type holds more than a millisecond, so the microseconds stay");
        assertEquals("2026-11-04T02:00:00.123456Z", scheduled.raw().path("schedule").path("nominalTime").textValue(), "the exact text stays in raw()");
        assertNull(scheduled.resetFrom());

        // a time with an offset is the same instant
        server.prepare((req, reply) -> reply.send(200, run(
                "schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T03:00:00+01:00','kind':'SCHEDULED','revision':1}")));
        assertEquals(Instant.ofEpochSecond(1_793_757_600L), client().get(RUN).schedule().nominalTime());

        // a kind this SDK does not know is kept as the server sent it, and a plain run has neither field
        server.prepare((req, reply) -> reply.send(200, run(
                "schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00Z','kind':'SOMETHING_NEW','revision':1}")));
        assertEquals("SOMETHING_NEW", client().get(RUN).schedule().kind());
        server.prepare((req, reply) -> reply.send(200, run()));
        Run plain = client().get(RUN);
        assertNull(plain.schedule());
        assertNull(plain.resetFrom());
        // a JSON null is an absent field
        server.prepare((req, reply) -> reply.send(200, run("schedule", "null", "resetFrom", "null")));
        Run nulls = client().get(RUN);
        assertNull(nulls.schedule());
        assertNull(nulls.resetFrom());
    }

    @Test
    void aRunAResetMadeSaysTheRunItCameFromAndTheJournalSequenceItKeeps() {
        String source = "0190aaaa-0000-7000-8000-0000000000aa";
        server.prepare((req, reply) -> reply.send(200, run(
                "rerunOf", "'" + source + "'", "resetFrom", "{'runId':'" + source + "','sequence':4}")));
        Run reset = client().get(RUN);
        assertEquals(new Run.ResetFromOfRun(source, 4), reset.resetFrom());
        assertEquals(source, reset.rerunOf());
        assertNull(reset.schedule());
        // started again, not reset: rerunOf and nothing else
        server.prepare((req, reply) -> reply.send(200, run("rerunOf", "'" + source + "'")));
        Run again = client().get(RUN);
        assertEquals(source, again.rerunOf());
        assertNull(again.resetFrom());
    }

    @Test
    void anAnswerWhoseScheduleOrResetIsNotWhatTheContractSaysIsAProtocolErrorNotARunWithHalfOfIt() {
        String[][] broken = {
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'yesterday','kind':'SCHEDULED','revision':1}"},
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','kind':'SCHEDULED','revision':1}"},
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00Z','kind':'SCHEDULED','revision':0}"},
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00Z','kind':'SCHEDULED','revision':'1'}"},
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00Z','kind':'SCHEDULED'}"},
            {"schedule", "{'nominalTime':'2026-11-04T02:00:00Z','kind':'SCHEDULED','revision':1}"},
            {"schedule", "{'scheduleId':'" + SCHEDULE + "','nominalTime':'2026-11-04T02:00:00Z','revision':1}"},
            {"schedule", "'nightly'"},
            {"resetFrom", "{'runId':'" + RUN + "'}"},
            {"resetFrom", "{'runId':'" + RUN + "','sequence':0}"},
            {"resetFrom", "{'sequence':4}"},
            {"resetFrom", "'run'"},
        };
        for (String[] field : broken) {
            server.prepare((req, reply) -> reply.send(200, run(field)));
            SagatException error = assertThrows(SagatException.class, () -> client().get(RUN), field[0] + " " + field[1]);
            assertEquals(ErrorCode.PROTOCOL, error.code(), field[0] + " " + field[1]);
        }
    }

    // ----------------------------------------------------------------------------------------- schedules

    private static ScheduleWrite nightly() {
        return ScheduleWrite.of("nightly",
                        ScheduleWrite.Spec.create()
                                .calendar(ScheduleWrite.Calendar.create()
                                        .hour(ScheduleWrite.Range.at(9)).year(ScheduleWrite.Range.at(2099)))
                                .startAt("2099-01-01T01:00:00+01:00").timezone("UTC"),
                        ScheduleWrite.Action.of(WF, Map.of("n", 1)))
                .policies(ScheduleWrite.Policies.create().overlap(OverlapPolicy.SKIP));
    }

    @Test
    void createSendsTheScheduleWithItsInstantsInUtcAndOneKeyAndReadsTheScheduleItGetsBack() {
        server.prepare((req, reply) -> reply.send(201, schedule("futureField", "1")));
        Schedule created = client().schedules().create(NS, nightly(), key("sched-key-1"));
        FakeRestServer.Seen request = only();
        assertEquals("POST", request.method());
        assertEquals("/v1/namespaces/" + NS + "/schedules", request.path());
        assertEquals("sched-key-1", request.header("idempotency-key"));
        assertEquals(j("{'name':'nightly','spec':{'calendars':[{'hour':[{'start':9}],'year':[{'start':2099}]}],"
                + "'startAt':'2099-01-01T00:00:00.000Z','timezone':'UTC'},'policies':{'overlap':'SKIP'},"
                + "'action':{'workflowId':'" + WF + "','input':{'n':1}}}"), body(request));
        assertEquals(2, created.revision());
        assertEquals(Instant.parse("2099-01-01T09:00:00Z"), created.nextActionAt());
        assertEquals("a".repeat(64), created.action().inputDigest(), "the input is never returned, only its digest");
        assertEquals(1, created.raw().get("futureField").intValue());
    }

    @Test
    void describeAsksForTheCountsItWasGivenAndKeepsAnOutcomeItDoesNotKnowAsTheStringItCameAs() {
        server.prepare((req, reply) -> reply.send(200, j(schedule().substring(0, schedule().length() - 1)
                + ",'nextActions':[{'nominalTime':'2099-01-01T09:00:00Z','actualTime':'2099-01-01T09:00:00Z'}],"
                + "'recentActions':[" + action() + "," + action("outcome", "'SOMETHING_NEWER'") + "],"
                + "'openRuns':['" + RUN + "'],'bufferedCount':1,'counters':{'STARTED':1,'SOMETHING_NEWER':1}}")));
        ScheduleDescription described = client().schedules().get(NS, SCHEDULE, ScheduleGetOptions.create().nextCount(3).recentCount(5));
        FakeRestServer.Seen request = only();
        assertEquals("GET", request.method());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "?nextCount=3&recentCount=5", request.uri());
        assertEquals(Instant.parse("2099-01-01T09:00:00Z").toEpochMilli(), described.nextActions().get(0).nominalTime().toEpochMilli());
        assertEquals("SOMETHING_NEWER", described.recentActions().get(1).outcome());
        assertEquals(List.of(RUN), described.openRuns());
        assertEquals(Map.of("STARTED", 1L, "SOMETHING_NEWER", 1L), described.counters());
        assertEquals(1, described.bufferedCount());
        assertEquals(2, described.schedule().revision());
    }

    @Test
    void listSendsTheFiltersItWasGivenAndListAllFollowsTheOpaqueCursorsToTheEnd() {
        AtomicInteger page = new AtomicInteger();
        server.prepare((req, reply) -> reply.send(200, page.incrementAndGet() == 1
                ? "{\"items\":[" + schedule("name", "\"a-1\"") + "],\"nextCursor\":\"c1\"}"
                : "{\"items\":[" + schedule("name", "\"a-2\"") + "],\"nextCursor\":null}"));
        List<String> names = new ArrayList<>();
        try (var stream = client().schedules().listAll(NS, ScheduleListFilter.create()
                .state("PAUSED").origin("API").name("a-").limit(1))) {
            stream.forEach(item -> names.add(item.name()));
        }
        assertEquals(List.of("a-1", "a-2"), names);
        assertEquals("/v1/namespaces/" + NS + "/schedules?limit=1&state=PAUSED&origin=API&name=a-", server.seen.get(0).uri());
        assertEquals("/v1/namespaces/" + NS + "/schedules?cursor=c1&limit=1&state=PAUSED&origin=API&name=a-", server.seen.get(1).uri());

        server.prepare((req, reply) -> reply.send(200, "{\"items\":[],\"nextCursor\":\"next\"}"));
        Page<Schedule> one = client().schedules().list(NS, ScheduleListFilter.create().workflowId(WF.toUpperCase()));
        assertEquals("next", one.nextCursor());
        assertEquals("/v1/namespaces/" + NS + "/schedules?workflowId=" + WF, only().uri(), "a workflow id is a UUID, in lower case");
        server.seen.clear();
        refused(() -> client().schedules().list(NS, ScheduleListFilter.create().workflowId("not-a-uuid")));
    }

    @Test
    void updateCarriesTheRevisionItExpectsInTheBodyDeleteInTheQueryAndBothCarryAKey() {
        ScheduleWrite write = ScheduleWrite.of("nightly", ScheduleWrite.Spec.create().cron("0 9 * * *"),
                ScheduleWrite.Action.of(WF, Map.of()));
        server.prepare((req, reply) -> reply.send(200, schedule("revision", "3")));
        Schedule updated = client().schedules().update(NS, SCHEDULE, write, 2, key("upd-key-0001"));
        assertEquals(3, updated.revision());
        assertEquals("PUT", only().method());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE, only().path());
        assertEquals("upd-key-0001", only().header("idempotency-key"));
        assertEquals(j("{'name':'nightly','spec':{'cron':['0 9 * * *']},'action':{'workflowId':'" + WF + "','input':{}},'expectedRevision':2}"),
                body(only()));
        server.prepare((req, reply) -> reply.send(204, null));
        client().schedules().delete(NS, SCHEDULE, 3, key("del-key-0001"));
        assertEquals("DELETE", only().method());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "?expectedRevision=3", only().uri());
        assertEquals("del-key-0001", only().header("idempotency-key"));
        assertNull(only().body(), "a delete has no body");
    }

    @Test
    void aStaleRevisionIsAConflictWithItsReasonAndARevisionThatCannotBeOneIsRefusedBeforeItIsSent() {
        server.prepare((req, reply) -> reply.send(409, "{\"code\":\"CONFLICT\",\"message\":\"stale\",\"retryable\":false,\"traceId\":\""
                + "e".repeat(32) + "\",\"details\":{\"reason\":\"REVISION_CONFLICT\",\"currentRevision\":4}}"));
        ScheduleWrite write = ScheduleWrite.of("n", ScheduleWrite.Spec.create().cron("0 9 * * *"), ScheduleWrite.Action.of(WF, Map.of()));
        SagatException conflict = assertThrows(SagatException.class, () -> client().schedules().update(NS, SCHEDULE, write, 2, null));
        assertEquals(ErrorCode.CONFLICT, conflict.code());
        assertEquals("REVISION_CONFLICT", conflict.reason().orElse(null));
        assertEquals(1, server.seen.size(), "a conflict is never retried");
        server.seen.clear();
        refused(() -> client().schedules().delete(NS, SCHEDULE, 0, null));
        refused(() -> client().schedules().update(NS, SCHEDULE, write, -3, null));
        refused(() -> client().schedules().get(NS, "../x", null));
        refused(() -> client().schedules().get("payments", SCHEDULE, null));
        refused(() -> client().schedules().get(NS, null, null));
        refused(() -> client().schedules().create(NS, null, null));
        refused(() -> client().schedules().create(NS, nightly(), key("short")));
        refused(() -> client().schedules().create(NS, ScheduleWrite.of("n", ScheduleWrite.Spec.create().startAt("not a time"),
                ScheduleWrite.Action.of(WF, Map.of())), null));
        refused(() -> client().schedules().create(NS, ScheduleWrite.of("n", ScheduleWrite.Spec.create().endAt("2099-01-01T00:00:00"),
                ScheduleWrite.Action.of(WF, Map.of())), null));
    }

    @Test
    void pauseAndResumeSendTheNoteTriggerSendsTheOverlapForThisActionOnlyAndATriggerRetryKeepsItsKey() {
        server.prepare((req, reply) -> reply.send(200, schedule("state", "'PAUSED'", "note", "'maintenance'")));
        Schedule paused = client().schedules().pause(NS, SCHEDULE, "maintenance", key("pause-key-01"));
        assertEquals("PAUSED", paused.state());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/pause", only().path());
        assertEquals("{\"note\":\"maintenance\"}", body(only()));
        server.prepare((req, reply) -> reply.send(200, schedule()));
        client().schedules().resume(NS, SCHEDULE, "done", key("resume-key-1"));
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/resume", only().path());
        server.seen.clear();
        refused(() -> client().schedules().pause(NS, SCHEDULE, "", null));
        refused(() -> client().schedules().resume(NS, SCHEDULE, null, null));

        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 2) {
                reply.send(503, UNAVAILABLE);
            } else {
                reply.send(202, action("outcome", "\"SKIPPED_OVERLAP\""));
            }
        });
        ScheduleActionRecord triggered = client().schedules().trigger(NS, SCHEDULE, OverlapPolicy.SKIP, null);
        assertEquals("SKIPPED_OVERLAP", triggered.outcome());
        assertEquals(2, server.seen.size());
        assertEquals(server.seen.get(0).header("idempotency-key"), server.seen.get(1).header("idempotency-key"));
        assertEquals("{\"overlap\":\"SKIP\"}", body(server.seen.get(1)));
        assertEquals("POST", server.seen.get(1).method());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/trigger", server.seen.get(1).path());

        server.prepare((req, reply) -> reply.send(202, action()));
        client().schedules().trigger(NS, SCHEDULE, null, key("trigger-key-1"));
        assertEquals("{}", body(only()), "no overlap, no member: never null");
        server.prepare((req, reply) -> reply.send(202, action()));
        client().schedules().trigger(NS, SCHEDULE, "WHENEVER", key("trigger-key-2"));
        assertEquals("{\"overlap\":\"WHENEVER\"}", body(only()), "a policy this SDK does not know is sent as given");
    }

    @Test
    void aBackfillSendsItsRangeAsUtcInstantsACancelNamesTheBackfillAndTheHistoryCanBeFilteredByOutcome() {
        server.prepare((req, reply) -> reply.send(202, j("{'backfillId':'" + BATCH + "','start':'2026-10-10T08:00:00Z','end':'2026-10-10T10:00:00Z',"
                + "'cursor':null,'state':'PENDING','overlap':'ALLOW_ALL','counts':{'STARTED':2}}")));
        ScheduleBackfill backfill = client().schedules().backfill(NS, SCHEDULE, Instant.parse("2026-10-10T08:00:00Z"),
                Instant.parse("2026-10-10T10:00:00Z"), OverlapPolicy.ALLOW_ALL, key("back-key-001"));
        assertEquals("{\"start\":\"2026-10-10T08:00:00.000Z\",\"end\":\"2026-10-10T10:00:00.000Z\",\"overlap\":\"ALLOW_ALL\"}", body(only()));
        assertEquals("POST", only().method());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/backfills", only().path());
        assertEquals("back-key-001", only().header("idempotency-key"));
        assertNull(backfill.cursor());
        assertEquals(Map.of("STARTED", 2L), backfill.counts());

        // The same range as RFC 3339 text with an offset: sent in UTC, never in the offset it was given.
        server.prepare((req, reply) -> reply.send(202, j("{'backfillId':'" + BATCH + "','start':'2026-10-10T08:00:00Z','end':'2026-10-10T10:00:00Z',"
                + "'state':'PENDING','overlap':'SKIP','counts':{}}")));
        client().schedules().backfill(NS, SCHEDULE, "2026-10-10T13:30:00+05:30", "2026-10-10T10:00:00+00:00", null, null);
        assertTrue(body(only()).startsWith("{\"start\":\"2026-10-10T08:00:00.000Z\",\"end\":\"2026-10-10T10:00:00.000Z\"}"), body(only()));
        server.seen.clear();
        refused(() -> client().schedules().backfill(NS, SCHEDULE, "not a time", "2026-10-10T10:00:00Z", null, null));
        refused(() -> client().schedules().backfill(NS, SCHEDULE, "2026-10-10T10:00:00", "2026-10-10T11:00:00Z", null, null));
        refused(() -> client().schedules().backfill(NS, SCHEDULE, (Instant) null, Instant.now(), null, null));

        server.prepare((req, reply) -> reply.send(200, j("{'backfillId':'" + BATCH + "','start':'2026-10-10T08:00:00Z','end':'2026-10-10T10:00:00Z',"
                + "'state':'CANCELLED','overlap':'ALLOW_ALL','counts':{}}")));
        ScheduleBackfill cancelled = client().schedules().cancelBackfill(NS, SCHEDULE, BATCH, null);
        assertEquals("CANCELLED", cancelled.state());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/backfills/" + BATCH + "/cancel", only().path());
        assertNull(only().body());
        server.seen.clear();
        refused(() -> client().schedules().cancelBackfill(NS, SCHEDULE, "not-a-uuid", null));

        server.prepare((req, reply) -> reply.send(200, "{\"items\":[" + action("outcome", "\"ALREADY_ACTED\"") + "],\"nextCursor\":null}"));
        Page<ScheduleActionRecord> actions = client().schedules().listActions(NS, SCHEDULE,
                ScheduleActionFilter.create().outcome("ALREADY_ACTED").limit(10));
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/actions?limit=10&outcome=ALREADY_ACTED", only().uri());
        assertEquals("ALREADY_ACTED", actions.items().get(0).outcome());

        server.prepare((req, reply) -> reply.send(200, "{\"items\":[{\"backfillId\":\"" + BATCH + "\",\"start\":\"2026-10-10T08:00:00Z\","
                + "\"end\":\"2026-10-10T10:00:00Z\",\"state\":\"COMPLETED\",\"overlap\":\"SKIP\",\"counts\":{}}]}"));
        assertEquals(1, client().schedules().listBackfills(NS, SCHEDULE, null).size());
        assertEquals("/v1/namespaces/" + NS + "/schedules/" + SCHEDULE + "/backfills", only().path());
    }

    // ---------------------------------------------------------------------------------- business ids

    @Test
    void aBusinessIdentifierIsPercentEncodedAsGivenAndOneThatCannotBeAPathSegmentIsRefusedLocally() {
        server.prepare((req, reply) -> reply.send(200, j("{'businessId':'Ord/1#x?y','executionId':'" + RUN + "','currentRunId':'" + RUN
                + "','open':false,'lastStatus':'SUCCEEDED','workflowId':'" + WF
                + "','boundAt':'2026-10-10T10:00:00Z','reusableAfter':'2026-11-09T10:00:00Z'}")));
        BusinessIdBinding binding = client().businessIds().get(NS, "Ord/1#x?y", null);
        assertEquals("/v1/namespaces/" + NS + "/business-ids/Ord%2F1%23x%3Fy", only().uri(), "no trimming and no case folding");
        assertFalse(binding.open());
        assertEquals(Instant.parse("2026-11-09T10:00:00Z"), binding.reusableAfter());
        server.seen.clear();
        for (String bad : new String[] {"", " padded", "with space", "x".repeat(201), "ünï"}) {
            refused(() -> client().businessIds().get(NS, bad, null));
        }
        refused(() -> client().businessIds().get(NS, null, null));
        refused(() -> client().businessIds().get("payments", "order-1", null));
    }

    @Test
    void aBusinessIdentifierThatIsAWholeDotOrDotDotIsRefusedLocallyBecauseItWouldBeAStepInThePathButOneWithDotsInsideIsSentAsGiven() {
        server.seen.clear();
        for (String step : new String[] {".", ".."}) {
            refused(() -> client().businessIds().get(NS, step, null));
        }
        server.prepare((req, reply) -> reply.send(404, "{\"code\":\"NOT_FOUND\",\"message\":\"none\",\"retryable\":false,\"traceId\":\""
                + "e".repeat(32) + "\",\"details\":{\"reason\":\"BUSINESS_ID_NOT_FOUND\"}}"));
        for (String dotted : new String[] {"...", "a..b", ".a", "a."}) {
            server.seen.clear();
            assertThrows(SagatException.class, () -> client().businessIds().get(NS, dotted, null));
            assertEquals("/v1/namespaces/" + NS + "/business-ids/" + dotted, only().uri(), dotted + " is sent as given");
        }
    }

    // ----------------------------------------------------------------------------------------- batch

    private static String job(String... overrides) {
        Map<String, String> base = new LinkedHashMap<>();
        base.put("batchId", "'" + BATCH + "'");
        base.put("action", "'CANCEL'");
        base.put("state", "'RUNNING'");
        base.put("createdBy", "'alice'");
        base.put("createdAt", "'2026-10-10T10:00:00Z'");
        base.put("counts", "{'selected':2,'succeeded':1,'skipped':0,'failed':0,'cancelled':0,'deferred':1}");
        return j(fields(base, overrides));
    }

    @Test
    void aBatchIsPreviewedCreatedWithOneKeyReadCancelledAndItsItemsFilteredByState() {
        server.prepare((req, reply) -> reply.send(200, j("{'matched':2,'capped':false,'sample':['" + RUN + "']}")));
        BatchPreview preview = client().batch().preview(NS, BatchSelector.runIds(List.of(RUN)), 50, null);
        assertEquals("/v1/namespaces/" + NS + "/batch-operations/preview", only().path());
        assertEquals(j("{'selector':{'runIds':['" + RUN + "']},'maxItems':50}"), body(only()));
        assertEquals(2, preview.matched());
        assertFalse(preview.capped());
        assertEquals(List.of(RUN), preview.sample());

        server.prepare((req, reply) -> reply.send(202, job()));
        BatchJob created = client().batch().create(NS, BatchRequest.of(
                BatchSelector.filter(BatchSelector.Filter.create().status(List.of("RUNNING")).createdAfter("2026-10-10T02:00:00+02:00")),
                "CANCEL", "clean up the day").maxItems(10), key("batch-key-01"));
        assertEquals(j("{'selector':{'filter':{'status':['RUNNING'],'createdAfter':'2026-10-10T00:00:00.000Z'}},'action':'CANCEL',"
                + "'reason':'clean up the day','maxItems':10}"), body(only()));
        assertEquals("batch-key-01", only().header("idempotency-key"));
        assertEquals("POST", only().method());
        assertEquals("/v1/namespaces/" + NS + "/batch-operations", only().path());
        assertEquals(new BatchJob.Counts(2, 1, 0, 0, 0, 1), created.counts());
        assertFalse(created.hasPayload());
        assertNull(created.maxItems());

        server.prepare((req, reply) -> reply.send(202, job("state", "\"CANCELLING\"")));
        BatchJob cancelling = client().batch().cancel(NS, BATCH, "stop the clean up", key("batch-cancel-1"));
        assertEquals("/v1/namespaces/" + NS + "/batch-operations/" + BATCH + "/cancel", only().path());
        assertEquals("{\"reason\":\"stop the clean up\"}", body(only()));
        assertEquals("CANCELLING", cancelling.state());

        server.prepare((req, reply) -> reply.send(200, j("{'items':[{'ordinal':0,'runId':'" + RUN
                + "','state':'SKIPPED','reason':'ALREADY_TERMINAL'},{'ordinal':1,'runId':'" + RUN + "','state':'NEWER_STATE'}],'nextCursor':'n'}")));
        Page<BatchItem> items = client().batch().listItems(NS, BATCH, BatchListOptions.create().state("SKIPPED").limit(2));
        assertEquals("/v1/namespaces/" + NS + "/batch-operations/" + BATCH + "/items?limit=2&state=SKIPPED", only().uri());
        assertEquals("ALREADY_TERMINAL", items.items().get(0).reason());
        assertEquals("NEWER_STATE", items.items().get(1).state(), "a state of a newer server is kept as a string");
        assertEquals("n", items.nextCursor());

        server.prepare((req, reply) -> reply.send(200, job()));
        assertEquals(BATCH, client().batch().get(NS, BATCH, null).batchId());
        assertEquals("/v1/namespaces/" + NS + "/batch-operations/" + BATCH, only().path());
        server.prepare((req, reply) -> reply.send(200, "{\"items\":[" + job() + "],\"nextCursor\":null}"));
        client().batch().list(NS, BatchListOptions.create().state("COMPLETED").limit(10));
        assertEquals("/v1/namespaces/" + NS + "/batch-operations?limit=10&state=COMPLETED", only().uri());

        server.seen.clear();
        refused(() -> client().batch().get(NS, "nope", null));
        refused(() -> client().batch().create(NS, BatchRequest.of(BatchSelector.query("x"), "CANCEL", "reasons"), key("bad")));
        refused(() -> client().batch().create(NS, BatchRequest.of(BatchSelector.filter(BatchSelector.Filter.create().createdBefore("yesterday")),
                "CANCEL", "reasons"), null));
        refused(() -> client().batch().listItems("payments", BATCH, null));
    }

    // ------------------------------------------------------------------------------------- with start

    @Test
    void signalWithStartNamesTheWorkflowByIdCarriesTheSignalAndOneKeyAndSaysWhatTheSignalWoke() {
        server.prepare((req, reply) -> reply.send(202, j("{'runId':'" + RUN + "','started':true,'deliveredTo':['wait']}")));
        SignalWithStartResult result = client().signalWithStart(WF, "order-9", Map.of("a", 1), "go", Map.of("b", 2),
                SignalWithStartOptions.create().idempotencyKey("sws-key-0001")
                        .businessIdReusePolicy(BusinessIdReusePolicy.ALLOW_DUPLICATE_FAILED_ONLY));
        assertEquals("/v1/runs/signal-with-start", only().path());
        assertEquals(j("{'workflowId':'" + WF + "','businessId':'order-9','input':{'a':1},'signal':{'name':'go','payload':{'b':2}},"
                + "'businessIdReusePolicy':'ALLOW_DUPLICATE_FAILED_ONLY'}"), body(only()));
        assertEquals("sws-key-0001", only().header("idempotency-key"));
        assertEquals(RUN, result.runId());
        assertTrue(result.started());
        assertEquals(List.of("wait"), result.deliveredTo());

        // An identifier is sent as given, spaces and all: the server is the one that judges it.
        server.prepare((req, reply) -> reply.send(202, j("{'runId':'" + RUN + "','started':false,'deliveredTo':[]}")));
        client().signalWithStart(WF, " Has A Space ", null, "go", null, null);
        assertTrue(body(only()).contains("\"businessId\":\" Has A Space \""), body(only()));
        assertTrue(body(only()).contains("\"input\":{}") && body(only()).contains("\"payload\":{}"), body(only()));
        server.seen.clear();
        refused(() -> client().signalWithStart(WF, null, Map.of(), "go", Map.of(), null));
        refused(() -> client().signalWithStart(WF, "order-9", Map.of(), null, Map.of(), null));
        refused(() -> client().signalWithStart(WF, "order-9", Map.of(), "go", Map.of(), SignalWithStartOptions.create().idempotencyKey("short")));
    }

    @Test
    void updateWithStartRequiresTheCallersKeyKeepsItAcrossARetryAndReadsARejectionAsAnAnswer() {
        String receipt = "{'runId':'" + RUN + "','operation':'set','updateKey':'u-1','status':'APPLIED','result':7,"
                + "'beforeStateRevision':0,'afterStateRevision':1,'runVersion':4,'replayed':true}";
        server.prepare((req, reply) -> reply.send(200, j("{'runId':'" + RUN + "','started':true,'update':{'outcome':'APPLIED','receipt':" + receipt + "}}")));
        UpdateWithStartResult applied = client().updateWithStart(WF, "order-9", Map.of(), "set", "u-1", Map.of("value", 7),
                UpdateWithStartOptions.create().idempotencyKey("uws-key-0001")
                        .businessIdConflictPolicy(BusinessIdConflictPolicy.FAIL).startDelay(Duration.ofSeconds(2)).expectedStateRevision(0L));
        assertEquals("/v1/runs/update-with-start", only().path());
        assertEquals(j("{'workflowId':'" + WF + "','businessId':'order-9','input':{},"
                + "'update':{'name':'set','updateKey':'u-1','args':{'value':7},'expectedStateRevision':0},"
                + "'businessIdConflictPolicy':'FAIL','startDelayMs':2000}"), body(only()));
        assertEquals("APPLIED", applied.update().outcome());
        assertEquals(1, applied.update().receipt().afterStateRevision());
        assertEquals(7, applied.update().receipt().result().intValue());
        assertTrue(applied.update().receipt().replayed(), "a replay says so");

        server.prepare((req, reply) -> reply.send(200, j("{'runId':'" + RUN + "','started':false,'update':{'outcome':'REJECTED','reason':'REVISION_CONFLICT'}}")));
        UpdateWithStartResult rejected = client().updateWithStart(WF, "order-9", Map.of(), "set", "u-2", Map.of(),
                UpdateWithStartOptions.create().idempotencyKey("uws-key-0002"));
        assertFalse(rejected.started());
        assertEquals("REJECTED", rejected.update().outcome());
        assertEquals("REVISION_CONFLICT", rejected.update().reason());
        assertNull(rejected.update().receipt());

        server.seen.clear();
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "set", "", Map.of(), null));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "set", null, Map.of(), null));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "set", "has space", Map.of(), null));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "set", "k".repeat(129), Map.of(), null));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "set", "u-3", Map.of(),
                UpdateWithStartOptions.create().expectedStateRevision(-1L)));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of(), "bad name", "u-3", Map.of(), null));
        refused(() -> client().updateWithStart(WF, "order-9", Map.of("nan", Double.NaN), "set", "u-3", Map.of(), null));

        AtomicInteger attempts = new AtomicInteger();
        server.prepare((req, reply) -> {
            if (attempts.incrementAndGet() < 2) {
                reply.send(503, UNAVAILABLE);
            } else {
                reply.send(200, j("{'runId':'" + RUN + "','started':true,'update':{'outcome':'APPLIED','receipt':" + receipt + "}}"));
            }
        });
        client().updateWithStart(WF, "order-9", Map.of(), "set", "u-4", Map.of(), null);
        assertEquals(2, server.seen.size());
        assertEquals(body(server.seen.get(0)), body(server.seen.get(1)));
        assertEquals(server.seen.get(0).header("idempotency-key"), server.seen.get(1).header("idempotency-key"));
    }

    // ---------------------------------------------------------------------------------------- reset

    @Test
    void theResetCallsListThePointsPreviewThePlanResetWithAKeyAndReadTheRegister() {
        server.prepare((req, reply) -> reply.send(200, j("{'items':[{'sequence':1,'kind':'RUN_STARTED','node':null,'occurredAt':'2026-10-10T10:00:00Z'},"
                + "{'sequence':4,'kind':'SOMETHING_NEWER','node':'a','occurredAt':'2026-10-10T10:00:01Z'}],'nextCursor':null}")));
        Page<ResetPoint> points = client().resetPoints(RUN, PageOptions.create().limit(5));
        assertEquals("/v1/runs/" + RUN + "/reset-points?limit=5", only().uri());
        assertEquals(List.of(1L, 4L), points.items().stream().map(ResetPoint::sequence).toList());
        assertEquals("SOMETHING_NEWER", points.items().get(1).kind());
        assertNull(points.items().get(0).node());

        server.prepare((req, reply) -> reply.send(200, j("{'sequence':4,'restored':[{'node':'a','kind':'STEP'}],"
                + "'reexecuted':[{'node':'b','completedInSource':true,'markers':['irreversible']}],'reapply':[],'requiresAcknowledgement':true,'prefixEvents':6}")));
        ResetPlan plan = client().previewReset(RUN, 4, "NONE", null);
        assertEquals("{\"sequence\":4,\"reapply\":\"NONE\"}", body(only()));
        assertEquals("POST", only().method());
        assertEquals("/v1/runs/" + RUN + "/reset-previews", only().path());
        assertNull(only().header("idempotency-key"), "a preview changes nothing: it carries no key");
        assertTrue(plan.requiresAcknowledgement());
        assertEquals(List.of(new ResetPlan.Reexecuted("b", true, List.of("irreversible"))), plan.reexecuted());
        assertEquals(List.of(new ResetPlan.Restored("a", "STEP")), plan.restored());
        server.seen.clear();
        refused(() -> client().previewReset(RUN, 0, null, null));
        refused(() -> client().previewReset("run-1", 1, null, null));

        server.prepare((req, reply) -> reply.send(202, j("{'resetId':'" + BATCH + "','sourceRunId':'" + RUN + "','sequence':4,'state':'CREATED','newRunId':'"
                + SCHEDULE + "','reapplyReport':[],'createdAt':'2026-10-10T10:00:02Z'}")));
        Reset made = client().reset(RUN, ResetRequest.of(4, "start over from a").acknowledgeReexecution(true).businessId("DROP"), key("reset-key-001"));
        assertEquals("/v1/runs/" + RUN + "/resets", only().path());
        assertEquals("reset-key-001", only().header("idempotency-key"));
        assertEquals("{\"sequence\":4,\"reason\":\"start over from a\",\"businessId\":\"DROP\",\"acknowledgeReexecution\":true}", body(only()));
        assertEquals("CREATED", made.state());
        assertEquals(SCHEDULE, made.newRunId());
        assertNull(made.failureReason());
        server.prepare((req, reply) -> reply.send(202, j("{'resetId':'" + BATCH + "','sourceRunId':'" + RUN + "','sequence':1,'state':'PENDING_SOURCE'}")));
        client().reset(RUN, ResetRequest.of(1, "start over from the top"), null);
        assertEquals("{\"sequence\":1,\"reason\":\"start over from the top\"}", body(only()), "what was not asked is not sent");
        server.seen.clear();
        refused(() -> client().reset(RUN, ResetRequest.of(0, "start over from the top"), null));
        refused(() -> client().reset(RUN, ResetRequest.of(1, "start over from the top"), key("short")));
        refused(() -> client().reset(RUN, null, null));

        server.prepare((req, reply) -> reply.send(200, j("{'items':[{'resetId':'" + BATCH + "','sourceRunId':'" + RUN + "','sequence':4,'state':'CREATED'}]}")));
        assertEquals(1, client().listResets(RUN, null).size());
        assertEquals("/v1/runs/" + RUN + "/resets", only().path());
        server.prepare((req, reply) -> reply.send(200, j("{'resetId':'" + BATCH + "','sourceRunId':'" + RUN + "','sequence':4,'state':'CREATED'}")));
        assertEquals(BATCH, client().getReset(RUN, BATCH, null).resetId());
        assertEquals("/v1/runs/" + RUN + "/resets/" + BATCH, only().path());
        server.seen.clear();
        refused(() -> client().getReset(RUN, "../x", null));
    }

    // ------------------------------------------------------------------------------------------ misc

    @Test
    void everyCallHasAnAsynchronousFormAndAFailureIsRaisedFromTheFuture() throws Exception {
        server.prepare((req, reply) -> reply.send(200, schedule()));
        try (SagatClient c = client()) {
            ScheduleDescription described = c.schedules().getAsync(NS, SCHEDULE, null).get();
            assertEquals(SCHEDULE, described.schedule().scheduleId());
            server.prepare((req, reply) -> reply.send(202, run()));
            assertTrue(c.startResultAsync(WF, Map.of(), StartOptions.create().businessId("o-1")).get().started());
            server.prepare((req, reply) -> reply.send(202, run()));
            assertEquals(RUN, c.startAsync(WF, Map.of(), null).get().runId());
            var failure = assertThrows(java.util.concurrent.ExecutionException.class,
                    () -> c.schedules().deleteAsync(NS, SCHEDULE, 0, null).get());
            assertEquals(ErrorCode.VALIDATION_FAILED, ((SagatException) failure.getCause()).code());
        }
    }

    @Test
    void anAnswerThatIsNotWhatTheContractPromisesIsAProtocolErrorAndNotAGuess() {
        server.prepare((req, reply) -> reply.send(200, "{\"items\":\"nope\"}"));
        assertEquals(ErrorCode.PROTOCOL, assertThrows(SagatException.class,
                () -> client().schedules().list(NS, null)).code());
        server.prepare((req, reply) -> reply.send(200, "{\"scheduleId\":\"x\"}"));
        assertEquals(ErrorCode.PROTOCOL, assertThrows(SagatException.class,
                () -> client().schedules().get(NS, SCHEDULE, null)).code());
        server.prepare((req, reply) -> reply.send(200, j("{'resetId':'" + BATCH + "','sourceRunId':'" + RUN + "','sequence':0,'state':'CREATED'}")));
        assertEquals(ErrorCode.PROTOCOL, assertThrows(SagatException.class,
                () -> client().getReset(RUN, BATCH, null)).code());
    }

    @Test
    void theTokenDoesNotAppearInAnythingAGroupOfTheClientShows() {
        SagatClient c = client();
        for (Object group : new Object[] {c.schedules(), c.batch(), c.businessIds(), c}) {
            assertFalse(group.toString().contains("tok-1"), group.toString());
        }
    }
}
