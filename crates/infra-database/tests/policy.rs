#![allow(clippy::unwrap_used, clippy::panic)]
use sagat_domain::database::OperationKind;
use sagat_infra_database::{DestinationScope, ExecutionError, NetworkPolicy, validate_statement};

#[test]
fn query_syntax_cannot_smuggle_a_write_or_session_change() {
    for sql in [
        "SELECT 1; DELETE FROM orders",
        "SELECT 1 INTO stolen",
        "WITH x AS (DELETE FROM orders RETURNING *) SELECT * FROM x",
        "SET ROLE postgres",
        "COPY orders TO PROGRAM 'cat'",
        "BEGIN",
        "SELECT * FROM orders FOR UPDATE",
        "EXPLAIN ANALYZE DELETE FROM orders",
    ] {
        assert_eq!(
            validate_statement(sql, OperationKind::Query),
            Err(ExecutionError::StatementDenied)
        );
    }
    for sql in [
        "SELECT $1::text",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "SELECT ';DROP TABLE orders'",
        "SELECT 1 UNION SELECT 2",
    ] {
        assert!(validate_statement(sql, OperationKind::Query).is_ok());
    }
    for sql in [
        "DELETE FROM orders WHERE id = $1",
        "INSERT INTO orders(id) VALUES ($1) RETURNING id",
        "UPDATE orders SET id = $1",
    ] {
        assert!(validate_statement(sql, OperationKind::Execute).is_ok());
        assert!(validate_statement(sql, OperationKind::Query).is_err());
    }
    for sql in [
        "CREATE TABLE t(id int)",
        "TRUNCATE orders",
        "DO $$ BEGIN NULL; END $$",
        "SELECT 1",
    ] {
        assert!(validate_statement(sql, OperationKind::Execute).is_err());
    }
}

#[test]
fn all_dns_answers_must_pass_the_intersected_policy() {
    let policy = NetworkPolicy::new(
        DestinationScope::new(vec!["10.20.0.0/16".parse().unwrap()], vec![5432, 5433]).unwrap(),
        vec!["10.20.1.0/24".parse().unwrap()],
        false,
    )
    .unwrap();
    let ns = DestinationScope::new(vec!["10.20.0.0/24".parse().unwrap()], vec![5432]).unwrap();
    assert!(
        policy
            .validate_destination(&["10.20.0.4".parse().unwrap()], 5432, &ns)
            .is_ok()
    );
    for answers in [
        vec!["10.20.0.4", "127.0.0.1"],
        vec!["10.20.1.4"],
        vec!["10.20.2.4"],
        vec!["169.254.169.254"],
        vec!["::ffff:127.0.0.1"],
        vec![],
    ] {
        let ips = answers
            .into_iter()
            .map(|s| s.parse().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            policy.validate_destination(&ips, 5432, &ns),
            Err(ExecutionError::DestinationDenied)
        );
    }
    let broad = NetworkPolicy::new(
        DestinationScope::new(
            vec!["0.0.0.0/0".parse().unwrap(), "::/0".parse().unwrap()],
            vec![5432],
        )
        .unwrap(),
        vec![],
        false,
    )
    .unwrap();
    for ip in [
        "0.0.0.0",
        "127.0.0.2",
        "169.254.1.1",
        "224.0.0.1",
        "255.255.255.255",
        "100.100.100.200",
        "168.63.129.16",
        "::1",
        "fe80::1",
        "ff00::1",
        "2002:7f00:1::1",
        "2001::1",
    ] {
        assert!(
            broad
                .validate_destination(&[ip.parse().unwrap()], 5432, broad.allowed())
                .is_err(),
            "{ip}"
        );
    }
}

#[test]
fn namespace_and_operator_must_both_allow_the_destination_port() {
    let scope =
        |ports| DestinationScope::new(vec!["10.20.0.0/16".parse().unwrap()], ports).unwrap();
    let policy = NetworkPolicy::new(scope(vec![5432, 6432]), vec![], false).unwrap();
    let namespace = scope(vec![5432, 9999]);
    let addresses = ["10.20.0.4".parse().unwrap()];
    assert!(
        policy
            .validate_destination(&addresses, 5432, &namespace)
            .is_ok()
    );
    for port in [0, 22, 80, 443, 6432, 9999] {
        assert_eq!(
            policy.validate_destination(&addresses, port, &namespace),
            Err(ExecutionError::DestinationDenied)
        );
    }
    assert!(
        policy
            .validate_destination(&addresses, 5432, &scope(vec![]))
            .is_err()
    );
}
