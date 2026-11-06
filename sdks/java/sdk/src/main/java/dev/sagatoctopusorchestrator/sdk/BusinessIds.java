package dev.sagatoctopusorchestrator.sdk;

import java.util.concurrent.CompletableFuture;

/** How a business identifier stands in a namespace (reached from {@link SagatClient#businessIds()}). */
public final class BusinessIds {
    private final Calls calls;

    BusinessIds(Calls calls) {
        this.calls = calls;
    }

    @Override
    public String toString() {
        return "BusinessIds";
    }

    /**
     * How an identifier stands in a namespace. An identifier nobody holds (or that cannot be one) is
     * {@code NOT_FOUND}. The identifier is sent as given, percent-encoded in the path: no trimming and no case folding.
     * The SDK refuses (as {@code VALIDATION_FAILED}, sending nothing) only what cannot be a path segment: a value that is
     * not 1 to 200 visible ASCII characters, and a whole {@code .} or {@code ..}, which every URL resolver takes for a
     * step in the path. An identifier that merely contains dots ({@code ...}, {@code a..b}) is valid.
     */
    public BusinessIdBinding get(String namespaceId, String businessId, CallOptions options) {
        CallOptions opts = options == null ? CallOptions.create() : options;
        String path = "/namespaces/" + Wire.uuid(namespaceId, "a namespace id") + "/business-ids/"
                + Endpoints.encodeComponent(Wire.businessIdForPath(businessId));
        RestClient.Response response = calls.send(new RestClient.Request(
                "GET", path, null, null, null, calls.trace(opts.traceparent()), true));
        return BusinessIdBinding.parse(response.body());
    }

    public CompletableFuture<BusinessIdBinding> getAsync(String namespaceId, String businessId, CallOptions options) {
        return calls.async(() -> get(namespaceId, businessId, options));
    }
}
