//! Feature 022 (FR-022-035, FR-022-050): `concurrency` and `rate_limit` at the workflow level and on activity and
//! script steps. The builders assemble the policy with the IR's names; the server checks the bounds at publication.
#![allow(clippy::expect_used)]
use sagat_octopus_orchestrator_sdk::{
    Value, json,
    workflow::{
        Builder, ConcurrencyPolicy, RateLimitPolicy, RatePeriod, ScriptOptions, StepOptions,
        canonical_json,
    },
};

const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../tests/contract/flow-control-step-policies/fixtures/{name}.workflow.json"
    ));
    serde_json::from_slice(&std::fs::read(path).expect("fixture")).expect("fixture JSON")
}

fn provider_sync() -> Builder {
    let workflow = Builder::new("provider-sync");
    workflow
        .concurrency(ConcurrencyPolicy {
            key: Some("input.accountId".into()),
            ..ConcurrencyPolicy::new(1)
        })
        .rate_limit(RateLimitPolicy {
            group: Some("provider-starts".into()),
            key: Some("input.providerId".into()),
            queue_timeout_ms: Some(900_000),
            ..RateLimitPolicy::new(100, RatePeriod::Minute)
        })
        .step(
            "fetch",
            "provider.fetch",
            StepOptions {
                rate_limit: Some(RateLimitPolicy {
                    key: Some("input.providerId".into()),
                    ..RateLimitPolicy::new(10, RatePeriod::Second)
                }),
                ..StepOptions::default()
            },
        )
        .end("done", None);
    workflow
}

fn account_serial() -> Builder {
    let workflow = Builder::new("account-serial");
    workflow
        .concurrency(ConcurrencyPolicy {
            group: Some("orders-by-account".into()),
            key: Some("input.accountId".into()),
            queue_timeout_ms: Some(600_000),
            ..ConcurrencyPolicy::new(1)
        })
        .step("price", "catalog.price", StepOptions::default())
        .step(
            "notify",
            "crm.notify",
            StepOptions {
                concurrency: Some(ConcurrencyPolicy {
                    key: Some("steps.price.output.region".into()),
                    ..ConcurrencyPolicy::new(2)
                }),
                ..StepOptions::default()
            },
        )
        .end("done", None);
    workflow
}

#[test]
fn the_builders_write_the_shared_fixtures_byte_for_byte() {
    for (built, name) in [
        (provider_sync(), "provider-sync"),
        (account_serial(), "account-serial"),
    ] {
        let expected = fixture(name);
        assert_eq!(built.build().expect("valid workflow"), expected, "{name}");
        assert_eq!(
            built.json().expect("canonical text"),
            canonical_json(&expected).expect("canonical fixture"),
            "{name}"
        );
    }
}

#[test]
fn no_policy_writes_no_field_so_earlier_documents_are_unchanged() {
    let workflow = Builder::new("plain-flow");
    workflow
        .step("a", "act.a", StepOptions::default())
        .script("s", DIGEST, ScriptOptions::default())
        .end("done", None);
    let document = workflow.build().expect("valid workflow");
    for key in ["concurrency", "rateLimit"] {
        assert!(document.get(key).is_none(), "{key}");
        for node in document["nodes"].as_array().expect("nodes") {
            assert!(node.get(key).is_none(), "{key}");
        }
    }
}

#[test]
fn a_script_carries_both_policies_with_every_member() {
    let workflow = Builder::new("gated-script");
    workflow
        .script(
            "s",
            DIGEST,
            ScriptOptions {
                concurrency: Some(ConcurrencyPolicy {
                    key: Some("input.tenant".into()),
                    group: Some("scripts.a-b".into()),
                    queue_timeout_ms: Some(86_400_000),
                    ..ConcurrencyPolicy::new(10_000)
                }),
                rate_limit: Some(RateLimitPolicy {
                    key: Some("input.tenant".into()),
                    group: Some("scripts.a-b".into()),
                    queue_timeout_ms: Some(1),
                    ..RateLimitPolicy::new(3, RatePeriod::Hour)
                }),
                ..ScriptOptions::default()
            },
        )
        .end("done", None);
    let node = workflow.build().expect("valid workflow")["nodes"][0].clone();
    assert_eq!(
        node["concurrency"],
        json!({"key": "input.tenant", "limit": 10000, "group": "scripts.a-b", "queueTimeoutMs": 86400000})
    );
    assert_eq!(
        node["rateLimit"],
        json!({"key": "input.tenant", "limit": 3, "per": "hour", "group": "scripts.a-b", "queueTimeoutMs": 1})
    );
}

#[test]
fn the_last_workflow_level_call_wins() {
    let workflow = Builder::new("replaced-flow");
    workflow
        .concurrency(ConcurrencyPolicy::new(9))
        .concurrency(ConcurrencyPolicy::new(2))
        .rate_limit(RateLimitPolicy::new(9, RatePeriod::Hour))
        .rate_limit(RateLimitPolicy::new(4, RatePeriod::Second))
        .step("a", "act.a", StepOptions::default())
        .end("done", None);
    let document = workflow.build().expect("valid workflow");
    assert_eq!(document["concurrency"], json!({"limit": 2}));
    assert_eq!(document["rateLimit"], json!({"limit": 4, "per": "second"}));
}

#[test]
fn the_builders_only_assemble_the_bounds_are_the_servers() {
    let workflow = Builder::new("unchecked-flow");
    workflow
        .concurrency(ConcurrencyPolicy {
            group: Some("Not A Group".into()),
            ..ConcurrencyPolicy::new(0)
        })
        .rate_limit(RateLimitPolicy {
            queue_timeout_ms: Some(86_400_001),
            ..RateLimitPolicy::new(101, RatePeriod::Second)
        })
        .step(
            "a",
            "act.a",
            StepOptions {
                concurrency: Some(ConcurrencyPolicy {
                    key: Some(String::new()),
                    ..ConcurrencyPolicy::new(10_001)
                }),
                ..StepOptions::default()
            },
        )
        .end("done", None);
    let document = workflow.build().expect("valid workflow");
    assert_eq!(
        document["concurrency"],
        json!({"limit": 0, "group": "Not A Group"})
    );
    assert_eq!(
        document["rateLimit"],
        json!({"limit": 101, "per": "second", "queueTimeoutMs": 86400001})
    );
    assert_eq!(
        document["nodes"][0]["concurrency"],
        json!({"limit": 10001, "key": ""})
    );
}

#[test]
fn a_policy_decoded_from_json_refuses_a_member_the_ir_does_not_name() {
    let refused =
        serde_json::from_value::<RateLimitPolicy>(json!({"limit": 1, "per": "second", "burst": 5}));
    assert!(refused.is_err());
    let refused = serde_json::from_value::<RateLimitPolicy>(json!({"limit": 1, "per": "day"}));
    assert!(refused.is_err());
    let decoded =
        serde_json::from_value::<ConcurrencyPolicy>(json!({"limit": 3, "key": "input.k"}))
            .expect("a policy in the IR's names");
    assert_eq!(
        decoded,
        ConcurrencyPolicy {
            key: Some("input.k".into()),
            ..ConcurrencyPolicy::new(3)
        }
    );
}
