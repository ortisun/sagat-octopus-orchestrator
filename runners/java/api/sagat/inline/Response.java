package sagat.inline;

import java.util.Map;

public record Response(int status, Map<String, String> headers, String body) {}
