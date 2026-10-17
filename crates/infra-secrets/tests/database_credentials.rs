#![allow(clippy::unwrap_used, clippy::panic)]
use sagat_application::database::DatabaseCredentialsCodec;
use sagat_domain::secrets::SecretValue;
use sagat_infra_secrets::database::JsonDatabaseCredentials;
#[test]
fn credentials_are_strict_and_sensitive_parts_are_scanned() {
    let value = |s: &str| SecretValue::new(s.as_bytes().to_vec()).unwrap();
    let codec = JsonDatabaseCredentials;
    let credential = value(r#"{"username":"orders_reader","password":"p\"ass\\word"}"#);
    let decoded = codec.decode(&credential).unwrap();
    assert_eq!(decoded.username.expose(), b"orders_reader");
    assert_eq!(decoded.password.expose(), b"p\"ass\\word");
    assert!(!format!("{decoded:?}").contains("orders_reader"));
    for output in [
        b"orders_reader".as_slice(),
        b"p\"ass\\word",
        br#"{"value":"p\"ass\\word"}"#,
    ] {
        assert!(codec.contains_credential(&credential, output).unwrap());
    }
    assert!(
        !codec
            .contains_credential(&credential, b"[{\"id\":42}]")
            .unwrap()
    );
    for invalid in [
        r#"{"username":"a","password":"x","password":"y"}"#,
        r#"{"username":"a","password":"x","host":"evil"}"#,
        r#"{"username":"","password":"x"}"#,
        r#"{"username":"a","password":""}"#,
        r#"{"username":"a","password":"\u0000"}"#,
    ] {
        assert!(codec.decode(&value(invalid)).is_err());
    }
}

#[test]
fn output_scan_covers_decoded_json_and_binary_even_with_duplicate_keys() {
    use base64::Engine as _;
    let codec = JsonDatabaseCredentials;
    let credential =
        SecretValue::new(br#"{"username":"fixture_user","password":"secret"}"#.to_vec()).unwrap();
    for output in [
        br#"{"value":"\u0073\u0065\u0063\u0072\u0065\u0074"}"#.to_vec(),
        br#"{"value":"\u0073\u0065\u0063\u0072\u0065\u0074","value":"safe"}"#.to_vec(),
        serde_json::to_vec(&serde_json::json!({"type":"binary","value":base64::engine::general_purpose::STANDARD.encode(b"prefix-secret-suffix")})).unwrap(),
        br#"{"\u0073\u0065\u0063\u0072\u0065\u0074":1}"#.to_vec(),
    ] { assert!(codec.contains_credential(&credential,&output).unwrap()); }
    assert!(
        codec
            .contains_credential(&credential, b"[malformed")
            .is_err()
    );
    assert!(
        codec
            .contains_credential(&credential, &vec![b' '; 4 * 1024 * 1024 + 1])
            .is_err()
    );
}
