package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.time.Instant;

/** Options of {@link SagatClient#start}. A fluent, mutable holder: build one per call. */
public final class StartOptions {
    private String versionId;
    private String idempotencyKey;
    private String correlationId;
    private Instant deadline;
    private Duration deadlineIn;
    private String traceparent;

    public static StartOptions create() {
        return new StartOptions();
    }

    /** Pins a version of the workflow; the active one when absent. */
    public StartOptions versionId(String value) {
        this.versionId = value;
        return this;
    }

    /**
     * 8 to 160 printable ASCII characters. A fresh one per call when absent, reused by every retry of that call, so
     * two calls with no key are two runs.
     */
    public StartOptions idempotencyKey(String value) {
        this.idempotencyKey = value;
        return this;
    }

    public StartOptions correlationId(String value) {
        this.correlationId = value;
        return this;
    }

    /** The instant at which the run times out; must be in the future and within a year. Sent in UTC. */
    public StartOptions deadline(Instant value) {
        this.deadline = value;
        return this;
    }

    /** The same, as a duration from now, computed once before the first attempt. */
    public StartOptions deadlineIn(Duration value) {
        this.deadlineIn = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public StartOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    public String versionId() {
        return versionId;
    }

    public String idempotencyKey() {
        return idempotencyKey;
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

    public String traceparent() {
        return traceparent;
    }
}
