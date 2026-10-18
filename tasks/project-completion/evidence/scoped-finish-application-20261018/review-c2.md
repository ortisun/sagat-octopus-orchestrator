Fresh Sol/high bounded hash-oracle review: None. Removing fingerprint validation now fails the
derived-owner and invalid/exhausted assertions; hardcoded owner bytes with variable length now
fails tasks/items. Static fixtures/type review found no issue. No tests by reviewer.
Root mechanically split the reviewed module into production.rs and a public integration test
(public_tests.rs), replacing the private shared limit with independent i64::MAX in test code.
