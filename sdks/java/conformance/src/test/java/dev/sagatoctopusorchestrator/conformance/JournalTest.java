package dev.sagatoctopusorchestrator.conformance;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.ObjectMapper;
import dev.sagatoctopusorchestrator.sdk.JournalEvent;
import dev.sagatoctopusorchestrator.sdk.JournalPage;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.junit.jupiter.api.Test;

/**
 * The conformance runner's {@code awaitJournal} reads the whole journal of a run: an event on a later page than the
 * first is found, and each page is asked for with the cursor the page before it gave.
 */
class JournalTest {
    private static final ObjectMapper JSON = new ObjectMapper();

    private final List<String> asked = new ArrayList<>();

    private JournalPage page(String cursor) throws Exception {
        asked.add(cursor);
        if ("second".equals(cursor)) {
            return new JournalPage(
                    List.of(event("{\"sequence\":2,\"eventType\":\"step.started\",\"stepId\":\"held\",\"attempt\":1}")),
                    null,
                    JSON.createObjectNode());
        }
        return new JournalPage(
                List.of(event("{\"sequence\":1,\"eventType\":\"step.leased\",\"stepId\":\"held\",\"attempt\":1}")),
                "second",
                JSON.createObjectNode());
    }

    private static JournalEvent event(String json) throws Exception {
        return new JournalEvent(JSON.readTree(json));
    }

    @Test
    void awaitJournalFindsAnEventOnTheSecondPageAskedForWithTheCursorOfTheFirst() throws Exception {
        assertTrue(Runner.journalHas(this::page, "step.started", "held"));
        assertEquals(Arrays.asList(null, "second"), asked);
    }

    @Test
    void awaitJournalReadsEveryPageBeforeItSaysTheJournalHasNoSuchEvent() throws Exception {
        assertFalse(Runner.journalHas(this::page, "step.completed", "held"));
        assertEquals(Arrays.asList(null, "second"), asked);
    }
}
