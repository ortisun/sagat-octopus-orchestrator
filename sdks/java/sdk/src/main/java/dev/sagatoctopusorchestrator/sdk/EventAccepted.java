package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * The answer to a published event.
 *
 * @param eventId    the event's id
 * @param source     the source the event was admitted under
 * @param status     {@code ACCEPTED}, or {@code DUPLICATE} when the same event id was admitted before
 * @param statusText the status exactly as the server wrote it
 * @param raw        the answer as sent
 */
public record EventAccepted(String eventId, String source, Status status, String statusText, JsonNode raw) {
    /** What the server did with the event. */
    public enum Status {
        ACCEPTED,
        DUPLICATE,
        UNKNOWN
    }
}
