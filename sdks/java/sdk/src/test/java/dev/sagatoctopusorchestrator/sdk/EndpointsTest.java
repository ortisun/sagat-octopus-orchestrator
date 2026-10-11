package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

/** Which hosts are loopback for the plain-text warning of SPEC section 1. */
class EndpointsTest {
    @Test
    void onlyARealLoopbackAddressOrLocalhostIsLoopback() {
        for (String loopback : new String[] {"127.0.0.1", "127.255.255.254", "127.0.0.2", "localhost", "LOCALHOST", "LocalHost", "::1", "[::1]", "0:0:0:0:0:0:0:1", "[0:0:0:0:0:0:0:1]"}) {
            assertTrue(Endpoints.isLoopback(loopback), loopback);
        }
        for (String other : new String[] {
            "127.attacker.example", "127.0.0.1.attacker.test", "127.0.0.1.nip.io", "127.evil.example.com", "128.0.0.1", "126.255.255.255", "10.0.0.1", "192.0.2.1",
            "127.0.0", "127.0.0.1.1", "127.0.0.256", "127.0.0.01", "0127.0.0.1", "127.0.0.-1", "127.0.0.1 ", " 127.0.0.1", "", "localhost.attacker.example", "xlocalhost",
            "::2", "[::2]", "2001:db8::1", "::ffff:8.8.8.8", "example.com"}) {
            assertFalse(Endpoints.isLoopback(other), "[" + other + "]");
        }
        assertFalse(Endpoints.isLoopback(null));
    }

    @Test
    void theWarningIsLoggedForALookAlikeAndNamesOnlyTheHost() {
        CapturingLogger log = new CapturingLogger();
        Endpoints.warnIfPlainText(log, "http://127.attacker.example/v1", "apiUrl");
        Endpoints.warnIfPlainText(log, "http://127.0.0.1.nip.io/v1", "apiUrl");
        Endpoints.warnIfPlainText(log, "http://alice:s3cret@api.example.com:8080/v1?apikey=Q123", "apiUrl");
        Endpoints.warnIfPlainText(log, "http://127.0.0.1:8080/v1", "apiUrl");
        Endpoints.warnIfPlainText(log, "http://localhost/v1", "apiUrl");
        Endpoints.warnIfPlainText(log, "https://api.example.com/v1", "apiUrl");
        assertEquals(3, log.warnings("plain text").size(), log.lines().toString());
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("s3cret") || l.message().contains("Q123") || l.message().contains("alice")));
    }
}
