use super::*;
use serde_json::json;

#[test]
fn redis_transport_requires_explicit_secret_and_network_peers() {
    let values = with(&json!({"eventTransport":"redis"}));
    assert!(
        helm(&values).is_err(),
        "missing endpoint secret and egress cannot render"
    );
    let values = with(&json!({"eventTransport":"redis", "redis":{"urlSecret":"redis-url"}}));
    assert!(helm(&values).is_err(), "no permissive network fallback");
    let docs = render(&with(
        &json!({"eventTransport":"redis", "redis":{"urlSecret":"redis-url", "timeoutSecs":7}, "networkPolicy":{"egress":{"redis":{"port":6379,"to":[{"podSelector":{"matchLabels":{"app":"redis"}}}]}}}}),
    ));
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    let env = env_of(engine);
    assert_eq!(env["SAGAT_EVENT_TRANSPORT"], "redis");
    assert_eq!(env["SAGAT_REDIS_TIMEOUT_SECS"], "7");
    assert!(
        !env.contains_key("SAGAT_NATS_URL"),
        "inactive credentials are not injected"
    );
    let actual = engine["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .expect("env");
    let url = actual
        .iter()
        .find(|e| e["name"] == "SAGAT_REDIS_URL")
        .expect("URL");
    assert_eq!(url["valueFrom"]["secretKeyRef"]["name"], "redis-url");
    let policy = named(
        &docs,
        "NetworkPolicy",
        "sagat-octopus-orchestrator-engine-egress",
    );
    assert!(has_rule_for_port(policy, "egress", 6379, "TCP"));
    assert!(!has_rule_for_port(policy, "egress", 4222, "TCP"));
    let cp = env_of(named(
        &docs,
        "Deployment",
        "sagat-octopus-orchestrator-control-plane",
    ));
    assert_eq!(cp["SAGAT_REDIS_CALLBACKS_ENABLED"], "false");
}
#[test]
fn invalid_transport_is_refused_and_none_does_not_inject_broker_credentials() {
    assert!(helm(&with(&json!({"eventTransport":"typo"}))).is_err());
    let docs = render(&with(&json!({"eventTransport":"none"})));
    let env = env_of(named(
        &docs,
        "Deployment",
        "sagat-octopus-orchestrator-engine",
    ));
    assert_eq!(env["SAGAT_EVENT_TRANSPORT"], "none");
    assert!(!env.contains_key("SAGAT_REDIS_URL"));
    assert!(!env.contains_key("SAGAT_NATS_URL"));
}
