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
