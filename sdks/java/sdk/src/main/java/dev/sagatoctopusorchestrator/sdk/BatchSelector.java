package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.time.Instant;
import java.util.List;

/**
 * Which runs a batch job (or a preview) is about: the runs a filter matches, runs named by id, or the runs a query
 * selects. Made with {@link #filter}, {@link #runIds} or {@link #query}.
 */
public final class BatchSelector {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final Filter filter;
    private final List<String> runIds;
    private final String query;

    private BatchSelector(Filter filter, List<String> runIds, String query) {
        this.filter = filter;
        this.runIds = runIds;
        this.query = query;
    }

    /** The runs a filter matches. */
    public static BatchSelector filter(Filter filter) {
        return new BatchSelector(filter, null, null);
    }

    /** These runs, by id. An empty list is sent as it is, and the server refuses it. */
    public static BatchSelector runIds(List<String> runIds) {
        return new BatchSelector(null, runIds == null ? null : List.copyOf(runIds), null);
    }

    /** The runs a query selects. */
    public static BatchSelector query(String query) {
        return new BatchSelector(null, null, query);
    }

    /** The selector on the wire; the instants of a filter are sent as {@code Z} instants. */
    ObjectNode toJson() {
        ObjectNode out = F.objectNode();
        if (filter != null) {
            out.set("filter", filter.toJson());
        } else if (runIds != null) {
            ArrayNode array = out.putArray("runIds");
            runIds.forEach(array::add);
        } else if (query != null) {
            out.put("query", query);
        } else {
            throw SagatException.invalid("a selector is a filter, run ids or a query");
        }
        return out;
    }

    /** The filter of a selector; every member is optional. A fluent, mutable holder. */
    public static final class Filter {
        private String workflowId;
        private String versionId;
        private List<String> status;
        private String origin;
        private String scheduleId;
        private Object createdAfter;
        private Object createdBefore;
        private String businessIdPrefix;

        public static Filter create() {
            return new Filter();
        }

        public Filter workflowId(String value) {
            this.workflowId = value;
            return this;
        }

        public Filter versionId(String value) {
            this.versionId = value;
            return this;
        }

        /** Runs in any of these statuses. */
        public Filter status(List<String> value) {
            this.status = value == null ? null : List.copyOf(value);
            return this;
        }

        public Filter origin(String value) {
            this.origin = value;
            return this;
        }

        /** Runs a schedule started. */
        public Filter scheduleId(String value) {
            this.scheduleId = value;
            return this;
        }

        /** Runs created after this instant. Sent in UTC, as an instant with a {@code Z}. */
        public Filter createdAfter(Instant value) {
            this.createdAfter = value;
            return this;
        }

        /** The same, as RFC 3339 text with any offset; text that is not an instant is refused when the call is made. */
        public Filter createdAfter(String value) {
            this.createdAfter = value;
            return this;
        }

        /** Runs created before this instant. Sent in UTC, as an instant with a {@code Z}. */
        public Filter createdBefore(Instant value) {
            this.createdBefore = value;
            return this;
        }

        /** The same, as RFC 3339 text; see {@link #createdAfter(String)}. */
        public Filter createdBefore(String value) {
            this.createdBefore = value;
            return this;
        }

        /** Runs whose business identifier starts with this text. */
        public Filter businessIdPrefix(String value) {
            this.businessIdPrefix = value;
            return this;
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            if (workflowId != null) {
                out.put("workflowId", workflowId);
            }
            if (versionId != null) {
                out.put("versionId", versionId);
            }
            if (status != null) {
                ArrayNode array = out.putArray("status");
                status.forEach(array::add);
            }
            if (origin != null) {
                out.put("origin", origin);
            }
            if (scheduleId != null) {
                out.put("scheduleId", scheduleId);
            }
            if (createdAfter != null) {
                out.put("createdAfter", Wire.whenGiven(createdAfter, "createdAfter"));
            }
            if (createdBefore != null) {
                out.put("createdBefore", Wire.whenGiven(createdBefore, "createdBefore"));
            }
            if (businessIdPrefix != null) {
                out.put("businessIdPrefix", businessIdPrefix);
            }
            return out;
        }
    }
}
