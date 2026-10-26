//! FR-017-001,010,011,021: configuration boundaries before any external connection.
#![allow(clippy::expect_used)]
use sagat_domain::database::{
    ConnectionConfig, ConnectionRef, DatabaseError, DatabaseLimits, DatabaseOperation,
    OperationKind, Parameter, TlsPolicy, TypedValue,
};
use sagat_domain::{identity::Slug, secrets::SecretName};

fn config(host: &str) -> Result<ConnectionConfig, DatabaseError> {
    ConnectionConfig::postgres(
        host,
        5432,
        "orders",
        TlsPolicy::VerifyFull { ca_pem: None },
        SecretName::parse("orders-access").expect("name"),
        DatabaseLimits::default(),
    )
}
fn operation(
    statement: &str,
    parameters: Vec<Parameter>,
    limits: DatabaseLimits,
) -> Result<DatabaseOperation, DatabaseError> {
    DatabaseOperation::new(
        ConnectionRef::new(Slug::parse("orders-db").expect("name"), None).expect("reference"),
        OperationKind::Query,
        statement,
        parameters,
        limits,
    )
}

#[test]
fn target_is_a_bare_host_and_never_a_credential_url_or_driver_options() {
    for host in ["db.example.com", "db", "10.23.4.8", "2001:db8::5"] {
        assert!(config(host).is_ok());
    }
    for host in [
        "",
        "postgres://user:secret@db",
        "db:5432",
        "db/path",
        "db?sslmode=disable",
        "db\n",
        "-db.example",
        "db..example",
        "localhost%00",
        "[::1]",
    ] {
        assert_eq!(config(host), Err(DatabaseError::InvalidHost));
    }
    assert_eq!(
        ConnectionRef::new(Slug::parse("orders-db").expect("name"), Some(0)),
        Err(DatabaseError::InvalidRevision)
    );
}

#[test]
fn finite_limits_are_intersected_and_never_interpreted_as_unlimited() {
    let request = DatabaseLimits {
        max_rows: 4,
        ..DatabaseLimits::default()
    };
    let policy = DatabaseLimits {
        timeout_ms: 900,
        ..DatabaseLimits::default()
    };
    let effective = request.intersect(policy);
    assert_eq!((effective.max_rows, effective.timeout_ms), (4, 900));
    let mut invalid = DatabaseLimits {
        max_rows: 0,
        ..DatabaseLimits::default()
    };
    assert_eq!(invalid.validate(), Err(DatabaseError::InvalidLimits));
    invalid.max_rows = 10_001;
    assert!(invalid.validate().is_err());
    invalid = DatabaseLimits::default();
    invalid.max_result_bytes = 4 * 1024 * 1024 + 1;
    assert!(invalid.validate().is_err());
    invalid = DatabaseLimits::default();
    invalid.pool_size = 17;
    assert!(invalid.validate().is_err());
}

#[test]
fn operation_content_is_bounded_and_never_in_debug_or_validation_errors() {
    let canary = "private-content-do-not-log";
    let valid = operation(
        &format!("SELECT '{canary}'"),
        vec![Parameter::Literal(TypedValue::Text(canary.into()))],
        DatabaseLimits::default(),
    )
    .expect("operation");
    assert!(!format!("{valid:?}").contains(canary));
    assert!(operation("", vec![], DatabaseLimits::default()).is_err());
    assert!(
        operation(
            &"x".repeat(16 * 1024 + 1),
            vec![],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(
        operation(
            "SELECT $1",
            vec![Parameter::Literal(TypedValue::Text("x".repeat(65 * 1024)))],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(
        operation(
            "SELECT 1",
            vec![Parameter::Literal(TypedValue::Null); 65],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(operation("SELECT\0 1", vec![], DatabaseLimits::default()).is_err());
    assert!(TypedValue::float(f64::INFINITY).is_err());
    assert!(
        TypedValue::Integer("1; DROP TABLE orders".into())
            .validate()
            .is_err()
    );
    assert!(TypedValue::Decimal("NaN".into()).validate().is_err());
    assert!(
        TypedValue::Integer("9223372036854775807".into())
            .validate()
            .is_ok()
    );
    assert!(
        TypedValue::Decimal("12345678901234567890.123456789".into())
            .validate()
            .is_ok()
    );
}

#[test]
fn managed_node_policies_and_cache_weight_cannot_be_bypassed() {
    use sagat_domain::{
        saga::EffectPolicy,
        workflow::{Node, NodeId, WorkflowDefinition},
    };
    let build = |node| {
        WorkflowDefinition::new(
            Slug::parse("database-flow").expect("name"),
            NodeId::parse("query").expect("id"),
            vec![node],
        )
    };
    let make = |size| {
        Node::database(
            NodeId::parse("query").expect("id"),
            operation(&"x".repeat(size), vec![], DatabaseLimits::default()).expect("operation"),
        )
    };
    let small = build(make(10)).expect("small");
    let large = build(make(10_010)).expect("large");
    assert_eq!(large.footprint() - small.footprint(), 10_000);
    let mut write = operation(
        "UPDATE orders SET done = true",
        vec![],
        DatabaseLimits::default(),
    )
    .expect("operation");
    write.kind = OperationKind::Execute;
    let node = Node::database(NodeId::parse("query").expect("id"), write);
    assert!(build(node.clone()).is_ok());
    assert!(build(node.with_effect(EffectPolicy::NONE)).is_err());
    let mut operation =
        operation("SELECT 1", vec![], DatabaseLimits::default()).expect("operation");
    operation.limits.max_rows = 0;
    assert!(
        build(Node::database(
            NodeId::parse("query").expect("id"),
            operation
        ))
        .is_err()
    );
}
