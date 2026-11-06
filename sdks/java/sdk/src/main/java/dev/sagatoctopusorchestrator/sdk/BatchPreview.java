package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/**
 * What a selector matches, counted up to the cap of the preview. A preview changes nothing.
 *
 * @param matched how many runs the selector matches (at most the cap)
 * @param capped  whether there are more than the cap
 * @param sample  a few of the runs, in the order the server chose
 * @param raw     the answer as sent
 */
public record BatchPreview(long matched, boolean capped, List<String> sample, JsonNode raw) {

    static BatchPreview parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a batch preview");
        return new BatchPreview(
                Wire.revision(raw, "matched", 0),
                raw.path("capped").isBoolean() && raw.path("capped").booleanValue(),
                Wire.strings(raw.path("sample")),
                raw);
    }
}
