package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/** One committed snapshot of an execution chain. */
public record ChainInspection(
        String chainId, String currentRunId, long revision, boolean cancelRequested, String finalOutcome,
        List<ChainSegment> segments, JsonNode raw) {}
