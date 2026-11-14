package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * A run, as the server describes it. {@code raw} is the answer as the server sent it, so a field added by a newer
 * server is reachable. Components marked nullable are null when the server sent none.
 *
 * @param runId         the run's id
 * @param workflowId    the workflow's id
 * @param versionId     the id of the workflow version the run executes
 * @param status        the status; {@link RunStatus#UNKNOWN} for one this SDK does not know
 * @param statusText    the status exactly as the server wrote it
 * @param version       number of journal events applied; usable as {@code expectedRunVersion}
 * @param paused        whether the run is paused
 * @param startedBy     the identity that started it (nullable)
 * @param createdAt     when it was created (UTC)
 * @param updatedAt     when it last changed (nullable)
 * @param finishedAt    when it ended (nullable)
 * @param deadline      the instant at which it times out (nullable)
 * @param traceId       the run's trace id (nullable)
 * @param correlationId the caller's correlation id (nullable)
 * @param depth         0 for a top-level run, the nesting depth for a child
 * @param parent        the parent run and node of a child run (nullable)
 * @param chain         the run's segment in its continuation chain (nullable: a run outside any chain has none). A
 *                      segment that is {@code SUCCEEDED} with completion kind {@code CONTINUED} ended into its
 *                      successor: it is not the process outcome, which is the chain's final outcome
 * @param rerunOf       the ended run this one was started again from ({@link OperateAction#RERUN_AS_NEW}); null for a
 *                      run that is not a rerun
 * @param businessId    the business identifier the run was started under, exactly as it was given; null for a run that
 *                      has none
 * @param startDelayedUntil the instant (UTC) the entry of a run started with a delay is held until; null for a run that
 *                      was never held
 * @param schedule      the schedule action that started the run; null for a run no schedule started
 * @param resetFrom     the run a reset made this one from, and the journal sequence of it that this run keeps; null
 *                      for a run no reset made (a rerun has {@code rerunOf} and not this)
 * @param raw           the answer as sent
 */
public record Run(
        String runId,
        String workflowId,
        String versionId,
        RunStatus status,
        String statusText,
        long version,
        boolean paused,
        String startedBy,
        Instant createdAt,
        Instant updatedAt,
        Instant finishedAt,
        Instant deadline,
        String traceId,
        String correlationId,
        int depth,
        Parent parent,
        ChainSegmentOfRun chain,
        String rerunOf,
        String businessId,
        Instant startDelayedUntil,
        ScheduleOfRun schedule,
        ResetFromOfRun resetFrom,
        JsonNode raw) {

    /** A run no schedule started and no reset made. */
    public Run(
            String runId,
            String workflowId,
            String versionId,
            RunStatus status,
            String statusText,
            long version,
            boolean paused,
            String startedBy,
            Instant createdAt,
            Instant updatedAt,
            Instant finishedAt,
            Instant deadline,
            String traceId,
            String correlationId,
            int depth,
            Parent parent,
            ChainSegmentOfRun chain,
            String rerunOf,
            String businessId,
            Instant startDelayedUntil,
            JsonNode raw) {
        this(runId, workflowId, versionId, status, statusText, version, paused, startedBy, createdAt, updatedAt,
                finishedAt, deadline, traceId, correlationId, depth, parent, chain, rerunOf, businessId,
                startDelayedUntil, null, null, raw);
    }

    /** A run with no business identifier and no start delay. */
    public Run(
            String runId,
            String workflowId,
            String versionId,
            RunStatus status,
            String statusText,
            long version,
            boolean paused,
            String startedBy,
            Instant createdAt,
            Instant updatedAt,
            Instant finishedAt,
            Instant deadline,
            String traceId,
            String correlationId,
            int depth,
            Parent parent,
            ChainSegmentOfRun chain,
            String rerunOf,
            JsonNode raw) {
        this(runId, workflowId, versionId, status, statusText, version, paused, startedBy, createdAt, updatedAt,
                finishedAt, deadline, traceId, correlationId, depth, parent, chain, rerunOf, null, null, raw);
    }

    /** A run that is not a rerun. */
    public Run(
            String runId,
            String workflowId,
            String versionId,
            RunStatus status,
            String statusText,
            long version,
            boolean paused,
            String startedBy,
            Instant createdAt,
            Instant updatedAt,
            Instant finishedAt,
            Instant deadline,
            String traceId,
            String correlationId,
            int depth,
            Parent parent,
            ChainSegmentOfRun chain,
            JsonNode raw) {
        this(runId, workflowId, versionId, status, statusText, version, paused, startedBy, createdAt, updatedAt,
                finishedAt, deadline, traceId, correlationId, depth, parent, chain, null, raw);
    }

    /**
     * Which schedule started a run, for which nominal time, by which kind of action and under which revision of the
     * schedule.
     *
     * @param scheduleId  the schedule
     * @param nominalTime the nominal time of the action: the time the schedule calculated, or for a trigger the instant
     *                    of the action (UTC; {@code raw()} holds the exact text)
     * @param kind        {@code SCHEDULED}, {@code TRIGGERED} or {@code BACKFILL}, or another string: the set only
     *                    grows, and a value this SDK does not know is kept as the server sent it
     * @param revision    the revision of the schedule the action ran under (1 or more)
     */
    public record ScheduleOfRun(String scheduleId, Instant nominalTime, String kind, long revision) {}

    /**
     * The run a reset made a run from.
     *
     * @param runId    the run that was reset
     * @param sequence the journal sequence (1 or more) of that run the new run keeps as its own
     */
    public record ResetFromOfRun(String runId, long sequence) {}

    /** The parent of a child run. */
    public record Parent(String runId, String node, String cancelPolicy) {}

    /**
     * Where a run sits in its continuation chain.
     *
     * @param chainId        the chain, which is its first run's id
     * @param segmentIndex   0-based position of the run in the chain
     * @param completionKind {@code CONTINUED} or {@code FINAL}; null while the segment is open
     * @param successorRunId the next segment when the run was continued (nullable)
     */
    public record ChainSegmentOfRun(String chainId, int segmentIndex, String completionKind, String successorRunId) {}

    /** Whether the run has ended. */
    public boolean isTerminal() {
        return status.isTerminal();
    }
}
