package sagat.inline;

import java.util.Map;

/** The only mediated route to network requests and leased secrets. */
public interface Octopus {
    String idempotencyKey();
    Response fetch(String url, String method, Map<String, String> headers, String body) throws Exception;
    String secret(String name) throws Exception;
    void log(Object... values);
}
