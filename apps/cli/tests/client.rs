//! Where a token may be sent: to an `https` address, or in the clear only to this machine, and to
//! the address that is meant, not to the one a clever address hides.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::client::Client;

fn accepted(server: &str) -> bool {
    Client::new(server, "a-token-0123456789", "payments").is_ok()
}

#[test]
fn a_token_goes_over_https_or_to_this_machine_and_nowhere_else() {
    for ok in [
        "https://sagat.example.com/v1",
        "https://sagat.example.com:8443/v1/",
        "http://localhost:8080/v1",
        "http://LOCALHOST/v1",
        "http://127.0.0.1:3000/v1",
        "http://127.1/v1",
        "http://[::1]:3000/v1",
    ] {
        assert!(accepted(ok), "{ok}");
    }
    for refused in [
        "http://sagat.example.com/v1",
        "http://localhost.example.com/v1",
        "http://127.0.0.1.example.com/v1",
        "http://10.0.0.5/v1",
        "ftp://localhost/v1",
        "localhost:8080",
        "",
        "https://",
        // Where the request goes is what is after the @, not what is before it.
        "http://localhost:80@evil.example/v1",
        "http://[::1]:x@evil.example/v1",
        "http://127.0.0.1@evil.example/v1",
        "https://user:password@sagat.example.com/v1",
        "https://user@sagat.example.com/v1",
    ] {
        assert!(!accepted(refused), "{refused:?}");
    }
}

#[test]
fn a_token_that_cannot_travel_in_a_header_is_refused_before_anything_is_sent() {
    for token in ["", "has a space", "line\nbreak", "tab\tbed", "accént"] {
        assert!(
            Client::new("https://sagat.example.com/v1", token, "payments").is_err(),
            "{token:?}"
        );
    }
}
