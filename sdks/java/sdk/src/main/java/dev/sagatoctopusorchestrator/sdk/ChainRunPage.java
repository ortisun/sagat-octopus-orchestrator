package dev.sagatoctopusorchestrator.sdk;

import java.util.List;

/** One bounded page of a chain's runs in segment order. {@code nextCursor} is null on the last page. */
public record ChainRunPage(List<ChainRun> items, String nextCursor) {}
