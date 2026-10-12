use super::*;

fn native_tls_values() -> Value {
    with(&serde_json::json!({
        "controlPlane": {
            "allowPlaintextSecrets": false,
            "nativeTls": {
                "tlsSecret": "api-server-tls",
                "clientCaSecret": "webhook-client-ca",
                "healthcheckCaSecret": "api-healthcheck-ca",
                "healthcheckServerName": "api.sagat.svc"
            },
            "webhookOidcProfilesSecret": "webhook-oidc-profiles"
        }
    }))
}

fn volume<'a>(deployment: &'a Value, name: &str) -> &'a Value {
    deployment["spec"]["template"]["spec"]["volumes"]
        .as_array()
        .expect("volumes")
        .iter()
        .find(|volume| volume["name"] == name)
        .unwrap_or_else(|| panic!("missing volume {name}"))
}

#[test]
fn native_tls_mounts_each_trust_role_read_only_and_uses_https_probes() {
    let docs = render(&native_tls_values());
    let control = named(
        &docs,
        "Deployment",
        "sagat-octopus-orchestrator-control-plane",
    );
    let pod = &control["spec"]["template"]["spec"];
    let container = &pod["containers"][0];
    let env = env_of(control);
    assert_eq!(env["SAGAT_HTTP_TLS_CERT"], "/etc/sagat/http-tls/tls.crt");
    assert_eq!(env["SAGAT_HTTP_TLS_KEY"], "/etc/sagat/http-tls/tls.key");
    assert_eq!(
        env["SAGAT_HTTP_TLS_CLIENT_CA"],
        "/etc/sagat/http-client-ca/ca.crt"
    );
    assert_eq!(
        env["SAGAT_HEALTHCHECK_TLS_CA_FILE"],
        "/etc/sagat/healthcheck-ca/ca.crt"
    );
    assert_eq!(env["SAGAT_HEALTHCHECK_TLS_SERVER_NAME"], "api.sagat.svc");
    assert_eq!(
        env["SAGAT_WEBHOOK_OIDC_PROFILES_FILE"],
        "/etc/sagat/webhook-oidc/profiles.json"
    );
    for (volume_name, secret_name, keys) in [
        ("http-tls", "api-server-tls", vec!["tls.crt", "tls.key"]),
        ("http-client-ca", "webhook-client-ca", vec!["ca.crt"]),
        ("healthcheck-ca", "api-healthcheck-ca", vec!["ca.crt"]),
        (
            "webhook-oidc",
            "webhook-oidc-profiles",
            vec!["profiles.json"],
        ),
    ] {
        let mounted = volume(control, volume_name);
        assert_eq!(mounted["secret"]["secretName"], secret_name);
        assert_eq!(mounted["secret"]["defaultMode"], 288, "0440 in decimal");
        let actual: Vec<&str> = mounted["secret"]["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["key"].as_str().expect("key"))
            .collect();
        assert_eq!(actual, keys);
        let mount = container["volumeMounts"]
            .as_array()
            .expect("mounts")
            .iter()
            .find(|mount| mount["name"] == volume_name)
            .expect("volume mount");
        assert_eq!(mount["readOnly"], true);
    }
    assert_eq!(pod["securityContext"]["fsGroup"], 10001);
    assert_eq!(
        pod["securityContext"]["fsGroupChangePolicy"],
        "OnRootMismatch"
    );
    for probe in ["startupProbe", "livenessProbe", "readinessProbe"] {
        assert_eq!(container[probe]["httpGet"]["scheme"], "HTTPS", "{probe}");
    }
    let service = named(&docs, "Service", "sagat-octopus-orchestrator-control-plane");
    assert_eq!(service["spec"]["ports"][0]["appProtocol"], "https");

    // The same group ownership fixes the existing engine's 0440 server-key mount for uid 10001.
    let engine = named(&docs, "Deployment", "sagat-octopus-orchestrator-engine");
    assert_eq!(
        engine["spec"]["template"]["spec"]["securityContext"]["fsGroup"],
        10001
    );
}

#[test]
fn the_plain_chart_stays_plain_and_does_not_mount_tls_material() {
    let docs = render(&base());
    let control = named(
        &docs,
        "Deployment",
        "sagat-octopus-orchestrator-control-plane",
    );
    let env = env_of(control);
    for name in [
        "SAGAT_HTTP_TLS_CERT",
        "SAGAT_HTTP_TLS_KEY",
        "SAGAT_HTTP_TLS_CLIENT_CA",
        "SAGAT_HEALTHCHECK_TLS_CA_FILE",
        "SAGAT_WEBHOOK_OIDC_PROFILES_FILE",
    ] {
        assert!(!env.contains_key(name), "{name}");
    }
    let container = &control["spec"]["template"]["spec"]["containers"][0];
    for probe in ["startupProbe", "livenessProbe", "readinessProbe"] {
        assert!(
            container[probe]["httpGet"].get("scheme").is_none(),
            "{probe}"
        );
    }
    assert_eq!(
        named(&docs, "Service", "sagat-octopus-orchestrator-control-plane")["spec"]["ports"][0]["appProtocol"],
        "http"
    );
}

#[test]
fn partial_or_contradictory_native_tls_values_are_refused() {
    let cases = [
        (
            serde_json::json!({"controlPlane": {"allowPlaintextSecrets": false, "nativeTls": {"tlsSecret": "server"}}}),
            "required together",
        ),
        (
            serde_json::json!({"controlPlane": {"allowPlaintextSecrets": false, "nativeTls": {"clientCaSecret": "clients"}}}),
            "required together",
        ),
        (
            serde_json::json!({"controlPlane": {"allowPlaintextSecrets": true, "nativeTls": {"tlsSecret": "server", "clientCaSecret": "clients"}}}),
            "contradict",
        ),
        (
            serde_json::json!({"controlPlane": {"nativeTls": {"healthcheckCaSecret": "server-ca"}}}),
            "needs native TLS",
        ),
        (
            serde_json::json!({"controlPlane": {"allowPlaintextSecrets": false, "nativeTls": {"tlsSecret": "server", "clientCaSecret": "clients", "healthcheckServerName": ""}}}),
            "healthcheckServerName",
        ),
    ];
    for (changes, expected) in cases {
        let error = helm(&with(&changes)).expect_err("invalid TLS values");
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}
