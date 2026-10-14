//! The reference pages are what a regeneration gives. `SAGAT_DOCS_BLESS=1` rewrites them.

use std::path::{Path, PathBuf};

use crate::generate::{api_reference, cli_reference, workflow_reference};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository")
        .to_path_buf()
}

fn check(page: &str, generated: &str) {
    let path = repo().join("docs/reference").join(page);
    if std::env::var("SAGAT_DOCS_BLESS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        std::fs::write(&path, generated).expect("write");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/reference/{page} is not what the contracts give: run \
         `SAGAT_DOCS_BLESS=1 cargo test -p sagat-tests --test contract_developer_experience` and commit the result"
    );
}

#[test]
fn the_api_reference_is_what_the_openapi_document_gives() {
    let source = "specs/001-platform-foundation/contracts/openapi.yaml";
    let text = std::fs::read_to_string(repo().join(source)).expect("openapi");
    let openapi: serde_json::Value = serde_saphyr::from_str(&text).expect("yaml");
    let page = api_reference(&openapi, source);
    // The page is the map of the contract: every operation in it, none made up.
    let operations: usize = openapi["paths"]
        .as_object()
        .expect("paths")
        .values()
        .map(|item| {
            ["get", "post", "put", "patch", "delete"]
                .iter()
                .filter(|m| item.get(**m).is_some())
                .count()
        })
        .sum();
    assert!(operations > 100, "{operations}");
    assert_eq!(page.matches("\n| `").count(), operations);
    check("api.md", &page);
}

#[test]
fn the_workflow_reference_is_what_the_schema_gives() {
    let source = "specs/001-platform-foundation/contracts/workflow.schema.json";
    let text = std::fs::read_to_string(repo().join(source)).expect("schema");
    let schema: serde_json::Value = serde_json::from_str(&text).expect("json");
    let page = workflow_reference(&schema, source);
    // Every kind of node the schema allows has its section.
    for kind in schema["$defs"]["node"]["oneOf"].as_array().expect("kinds") {
        let name = kind["$ref"]
            .as_str()
            .expect("ref")
            .rsplit('/')
            .next()
            .expect("name");
        assert!(
            page.contains(&format!("\n### {name}\n")),
            "no section for {name}"
        );
    }
    check("workflow-document.md", &page);
}

#[test]
fn the_command_reference_is_what_the_tool_prints() {
    check("cli.md", &cli_reference(sagat_cli::args::USAGE));
}
