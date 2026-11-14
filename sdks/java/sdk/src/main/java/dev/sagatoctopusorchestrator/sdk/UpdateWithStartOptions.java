package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.time.Instant;

/** Options of {@link SagatClient#updateWithStart}. A fluent, mutable holder: build one per call. */
public final class UpdateWithStartOptions {
    private String idempotencyKey;
    private String traceparent;
    private String versionId;
    private String correlationId;
    private Instant deadline;
    private Duration deadlineIn;
    private String businessIdReusePolicy;
    private String businessIdConflictPolicy;
    private Duration startDelay;
    private Long expectedStateRevision;

    public static UpdateWithStartOptions create() {
        return new UpdateWithStartOptions();
    }

    /**
     * 8 to 160 printable ASCII characters. A fresh one per call when absent, reused by every retry of that call, so two
     * calls with no key are two requests. (The update has its own key, which the caller always gives.)
     */
    public UpdateWithStartOptions idempotencyKey(String value) {
        this.idempotencyKey = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public UpdateWithStartOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** Pins a version of the workflow for a run this call creates; the active one when absent. */
    public UpdateWithStartOptions versionId(String value) {
        this.versionId = value;
        return this;
    }

    public UpdateWithStartOptions correlationId(String value) {
        this.correlationId = value;
        return this;
    }

    /** The instant at which a run this call creates times out. Sent in UTC. */
    public UpdateWithStartOptions deadline(Instant value) {
        this.deadline = value;
        return this;
    }

    /** The same, as a duration from now, computed once before the first attempt. */
    public UpdateWithStartOptions deadlineIn(Duration value) {
        this.deadlineIn = value;
        return this;
    }

    /** What to do when a closed execution still remembers the identifier; see {@link BusinessIdReusePolicy}. */
    public UpdateWithStartOptions businessIdReusePolicy(String value) {
        this.businessIdReusePolicy = value;
        return this;
    }

    /** What to do when an open execution holds the identifier; see {@link BusinessIdConflictPolicy}. */
    public UpdateWithStartOptions businessIdConflictPolicy(String value) {
        this.businessIdConflictPolicy = value;
        return this;
    }

    /**
     * Holds the entry of a run this call creates for this long. Converted once, before the first attempt, to whole
     * milliseconds (a fraction of a millisecond is dropped, toward zero); the range (1 ms to 366 days) is the server's to
     * judge.
     */
    public UpdateWithStartOptions startDelay(Duration value) {
        this.startDelay = value;
        return this;
    }

    /** The state revision the update expects, 0 or more; a stale one is an answer ({@code REJECTED}), not an error. */
    public UpdateWithStartOptions expectedStateRevision(Long value) {
        this.expectedStateRevision = value;
        return this;
    }

    public String idempotencyKey() {
        return idempotencyKey;
    }

    public String traceparent() {
        return traceparent;
    }

    public String versionId() {
        return versionId;
    }

    public String correlationId() {
        return correlationId;
    }

    public Instant deadline() {
        return deadline;
    }

    public Duration deadlineIn() {
        return deadlineIn;
    }

    public String businessIdReusePolicy() {
        return businessIdReusePolicy;
    }

    public String businessIdConflictPolicy() {
        return businessIdConflictPolicy;
    }

    public Duration startDelay() {
        return startDelay;
    }

    public Long expectedStateRevision() {
        return expectedStateRevision;
    }
}
