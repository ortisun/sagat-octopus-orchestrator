#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::Env;

fn run_document(document: &str) -> (u8, String, String) {
    let path = std::env::temp_dir().join(format!(
        "sagat-cli-classification-{}-{}.json",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    std::fs::write(&path, document).expect("write document");
    let args = vec![
        "workflow".to_owned(),
        "validate".to_owned(),
        path.to_string_lossy().into_owned(),
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(sagat_cli::run(&args, &Env::default(), &mut out, &mut err));
    let _ = std::fs::remove_file(path);
    (
        code,
        String::from_utf8(out).expect("stdout"),
        String::from_utf8(err).expect("stderr"),
    )
}

#[test]
fn offline_validate_prints_effective_levels_and_the_inherited_open_warning() {
    let (code, out, err) = run_document(
        r#"{
          "apiVersion":"sagat-octopus-orchestrator.io/v1",
          "kind":"Workflow",
          "metadata":{"name":"classified-flow"},
          "entrypoint":"done",
          "inputSchema":{
            "type":"object",
            "x-sagat-classification":"public",
            "additionalProperties":false,
            "properties":{
              "name":{"type":"string"},
              "email":{"type":"string","x-sagat-classification":"pii"}
            }
          },
          "nodes":[{"id":"done","type":"activity","activity":"demo.done"}]
        }"#,
    );
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(
        out.contains("warning[CLASSIFICATION_INHERITED_OPEN]"),
        "{out}"
    );
    assert_eq!(
        out.matches("warning[CLASSIFICATION_INHERITED_OPEN]")
            .count(),
        1,
        "{out}"
    );
    assert!(out.contains("classification for classified-flow:"), "{out}");
    assert!(out.contains("inputSchema / public declared"), "{out}");
    assert!(out.contains("inputSchema /name public inherited"), "{out}");
    assert!(out.contains("inputSchema /email pii declared"), "{out}");
}

#[test]
fn report_labels_body_node_carriers_without_confusing_root_node_ids() {
    use sagat_application::classification::{
        CarrierPointer, ClassificationPointer, ClassificationReportEntry,
    };
    use sagat_domain::classification::Level;
    use sagat_domain::workflow::NodeId;

    let entries = [
        ClassificationReportEntry {
            carrier: CarrierPointer::NodeOutput(NodeId::parse("shared").expect("node")),
            path: ClassificationPointer::parse("").expect("pointer"),
            level: Level::Public,
            declared: true,
        },
        ClassificationReportEntry {
            carrier: CarrierPointer::BodyNodeOutput {
                body: NodeId::parse("batch").expect("body"),
                node: NodeId::parse("shared").expect("node"),
            },
            path: ClassificationPointer::parse("").expect("pointer"),
            level: Level::Secret,
            declared: true,
        },
    ];
    let lines = sagat_cli::diagnostics::classification_report("flow", &entries, false);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("nodeOutputSchema shared / public declared")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| { line.contains("body batch nodeOutputSchema shared / secret declared") }),
        "{lines:?}"
    );
}
