package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.time.Instant;

/** Options of {@link SagatClient#signalWithStart}. A fluent, mutable holder: build one per call. */
public final class SignalWithStartOptions {
    private String idempotencyKey;
    private String traceparent;
    private String versionId;
    private String correlationId;
    private Instant deadline;
    private Duration deadlineIn;
    private String businessIdReusePolicy;

    public static SignalWithStartOptions create() {
        return new SignalWithStartOptions();
    }

    /**
     * 8 to 160 printable ASCII characters. A fresh one per call when absent, reused by every retry of that call, so two
     * calls with no key are two requests.
     */
    public SignalWithStartOptions idempotencyKey(String value) {
        this.idempotencyKey = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public SignalWithStartOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** Pins a version of the workflow for a run this call creates; the active one when absent. */
    public SignalWithStartOptions versionId(String value) {
        this.versionId = value;
        return this;
    }

    public SignalWithStartOptions correlationId(String value) {
        this.correlationId = value;
        return this;
    }

    /** The instant at which a run this call creates times out. Sent in UTC. */
    public SignalWithStartOptions deadline(Instant value) {
        this.deadline = value;
        return this;
    }

    /** The same, as a duration from now, computed once before the first attempt. */
    public SignalWithStartOptions deadlineIn(Duration value) {
        this.deadlineIn = value;
        return this;
    }

    /** What to do when a closed execution still remembers the identifier; see {@link BusinessIdReusePolicy}. */
    public SignalWithStartOptions businessIdReusePolicy(String value) {
        this.businessIdReusePolicy = value;
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
}
