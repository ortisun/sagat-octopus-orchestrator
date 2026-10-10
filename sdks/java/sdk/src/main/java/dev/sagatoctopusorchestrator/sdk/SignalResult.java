package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/** The nodes a signal was delivered to; {@code raw} is the answer as sent. */
public record SignalResult(List<String> deliveredTo, JsonNode raw) {}
