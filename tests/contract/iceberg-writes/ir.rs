//! The canonical workflow schema and the IR of feature 035: the `icebergWrite` node and the `destinations` of a
//! root-level step (FR-035-008, FR-035-034).
//!
//! Valid samples are whole workflow documents under `fixtures/ir/valid`. A refusal is one of those samples
//! plus one edit, so the edit alone explains the refusal (the sample is accepted, the edited one is not).
//! The fragment (`workflow-lake.schema.json`) is checked on its own as well, which tells a fixture mistake from a
//! canonical schema that has not merged the fragment yet.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::{Value, json};

use super::{Edit, apply, fixtures, fragments, json_files, read_json, refusal, repository_root};

fn canonical() -> &'static Validator {
    static VALIDATOR: OnceLock<Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema = read_json(
            &repository_root().join("specs/001-platform-foundation/contracts/workflow.schema.json"),
        );
        jsonschema::options()
            .build(&schema)
            .expect("the canonical workflow schema compiles without external retrieval")
    })
}

/// One definition of the fragment, as a schema of its own.
fn fragment_definition(definition: &str) -> Validator {
    let mut schema = read_json(&fragments().join("workflow-lake.schema.json"));
    schema["$ref"] = json!(format!("#/$defs/{definition}"));
    jsonschema::options()
        .build(&schema)
        .unwrap_or_else(|error| panic!("the fragment definition {definition} compiles: {error}"))
}

fn valid_documents() -> Vec<(String, Value)> {
    json_files(&fixtures().join("ir/valid"))
}

fn valid(name: &str) -> Value {
    valid_documents()
        .into_iter()
        .find(|(candidate, _)| candidate == name)
        .unwrap_or_else(|| panic!("no valid sample named {name}"))
        .1
}

/// The nodes of every sample that match `kind`.
fn nodes_of<'a>(documents: &'a [(String, Value)], kind: &str) -> Vec<&'a Value> {
    documents
        .iter()
        .flat_map(|(_, document)| document["nodes"].as_array().expect("nodes").iter())
        .filter(|node| node["type"] == kind)
        .collect()
}

struct Refusal {
    name: &'static str,
    base: &'static str,
    edit: Edit,
    /// Where the fragment can judge the edited sample on its own: its definition and the pointer of the value.
    in_fragment: Option<(&'static str, &'static str)>,
}

fn case(name: &'static str, base: &'static str, edit: Edit) -> Refusal {
    Refusal {
        name,
        base,
        edit,
        in_fragment: None,
    }
}

impl Refusal {
    fn judged_as(mut self, definition: &'static str, pointer: &'static str) -> Self {
        self.in_fragment = Some((definition, pointer));
        self
    }
}

const WRITE: &str = "/nodes/1";
const DESTINATIONS: &str = "/nodes/0/destinations";

fn destination(name: &str) -> Value {
    json!({
        "name": name, "kind": "broker_stream", "subscriptionRef": "order-events",
        "target": {}, "records": {"output": true}
    })
}

fn body_with(node: Value) -> Value {
    json!({"worker": {"entrypoint": "work", "nodes": [
        node,
        {"id": "back", "type": "return", "result": "steps.work.output"}
    ]}})
}

fn plain_body_activity() -> Value {
    json!({"id": "work", "type": "activity", "activity": "orders.enrich", "next": "back"})
}

fn refusals() -> Vec<Refusal> {
    let compare = json!({"compare": {"op": "eq", "column": "region", "value": "steps.extract.output.region"}});
    vec![
        // ---- icebergWrite ---------------------------------------------------------------------------------
        case("extra property on the write step", "append-inline", Edit::Set("/nodes/1/extra", json!(1)))
            .judged_as("icebergWrite", WRITE),
        case("a mode outside append, overwrite and upsert", "append-inline", Edit::Set("/nodes/1/mode", json!("merge")))
            .judged_as("icebergWrite", WRITE),
        case("overwrite mode without the overwrite block", "overwrite-batch-partitions", Edit::Remove("/nodes/1/overwrite"))
            .judged_as("icebergWrite", WRITE),
        case("filter scope without a filter", "overwrite-filter", Edit::Remove("/nodes/1/overwrite/filter"))
            .judged_as("icebergWrite", WRITE),
        case("two record sources at once", "append-inline", Edit::Set("/nodes/1/records/blobs", json!("steps.extract.output.files")))
            .judged_as("icebergWrite", WRITE),
        case("a write step reading the source node's output as one record", "append-inline", Edit::Set("/nodes/1/records", json!({"output": true})))
            .judged_as("icebergWrite", WRITE),
        case("a write step reading an output path", "append-inline", Edit::Set("/nodes/1/records", json!({"outputPath": "/rows"})))
            .judged_as("icebergWrite", WRITE),
        case("an empty namespace", "append-inline", Edit::Set("/nodes/1/table/namespace", json!([])))
            .judged_as("icebergWrite", WRITE),
        case("a namespace nine levels deep", "append-inline", Edit::Set("/nodes/1/table/namespace", json!(["a", "b", "c", "d", "e", "f", "g", "h", "i"])))
            .judged_as("icebergWrite", WRITE),
        case("a connection name that is not a slug", "append-inline", Edit::Set("/nodes/1/connectionRef/name", json!("Lake_Main")))
            .judged_as("icebergWrite", WRITE),
        case("a record ceiling of zero", "append-blobs-mapped", Edit::Set("/nodes/1/limits/maxRecords", json!(0)))
            .judged_as("icebergWrite", WRITE),
        case("a timeout under a second", "append-blobs-mapped", Edit::Set("/nodes/1/limits/timeoutMs", json!(999)))
            .judged_as("icebergWrite", WRITE),
        case("seventeen key columns", "upsert-database-result", Edit::Set("/nodes/1/upsert/keyColumns", json!((0..17).map(|n| format!("key{n}")).collect::<Vec<_>>())))
            .judged_as("icebergWrite", WRITE),
        case("a mapped field that is not a JSON pointer", "append-blobs-mapped", Edit::Set("/nodes/1/mapping/0/field", json!("id")))
            .judged_as("icebergWrite", WRITE),
        case("a compensation other than revert", "append-blobs-mapped", Edit::Set("/nodes/1/compensation", json!("undo")))
            .judged_as("icebergWrite", WRITE),
        // ---- the filter language ---------------------------------------------------------------------------
        case("an and with one operand", "overwrite-filter", Edit::Set("/nodes/1/overwrite/filter", json!({"and": [compare.clone()]})))
            .judged_as("icebergWrite", WRITE),
        case("a filter with two operators", "overwrite-filter", Edit::Set("/nodes/1/overwrite/filter", json!({"and": [compare.clone(), compare.clone()], "or": [compare.clone(), compare.clone()]})))
            .judged_as("icebergWrite", WRITE),
        case("a comparison operator outside the closed set", "overwrite-filter", Edit::Set("/nodes/1/overwrite/filter", json!({"compare": {"op": "like", "column": "region", "value": "steps.extract.output.region"}})))
            .judged_as("icebergWrite", WRITE),
        // ---- placement: the root graph only -----------------------------------------------------------------
        case(
            "a write step inside a structured body",
            "destinations-every-kind",
            Edit::Set(
                "/bodies",
                body_with(json!({
                    "id": "work", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
                    "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
                    "records": {"inline": "steps.work.input"}, "next": "back"
                })),
            ),
        ),
        case(
            "destinations on an activity inside a structured body",
            "destinations-every-kind",
            Edit::Set(
                "/bodies",
                body_with(json!({
                    "id": "work", "type": "activity", "activity": "orders.enrich", "next": "back",
                    "destinations": [destination("notify")]
                })),
            ),
        ),
        // ---- destinations -----------------------------------------------------------------------------------
        case("no destination at all", "destinations-every-kind", Edit::Set(DESTINATIONS, json!([])))
            .judged_as("destinations", DESTINATIONS),
        case("nine destinations", "destinations-every-kind", Edit::Set(DESTINATIONS, json!((0..9).map(|n| destination(&format!("dest-{n}"))).collect::<Vec<_>>())))
            .judged_as("destinations", DESTINATIONS),
        case("a destination kind outside the five", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/kind", json!("kafka")))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("an unexpected property on a destination", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/extra", json!(1)))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("a destination name that is not a slug", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/name", json!("Bad Name")))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("a broker stream with a connection", "destinations-every-kind", Edit::Set("/nodes/0/destinations/4/connectionRef", json!({"name": "lake-main"})))
            .judged_as("destination", "/nodes/0/destinations/4"),
        case("a warehouse destination without a connection", "destinations-every-kind", Edit::Remove("/nodes/0/destinations/0/connectionRef"))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("an Iceberg destination in insert mode", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/mode", json!("insert")))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("a PostgreSQL destination in append mode", "destinations-every-kind", Edit::Set("/nodes/0/destinations/1/mode", json!("append")))
            .judged_as("destination", "/nodes/0/destinations/1"),
        case("an object-storage destination with a mode", "destinations-every-kind", Edit::Set("/nodes/0/destinations/2/mode", json!("append")))
            .judged_as("destination", "/nodes/0/destinations/2"),
        case("a destination reading a database result", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/records", json!({"databaseResult": "read"})))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("a broker stream reading record blobs", "destinations-every-kind", Edit::Set("/nodes/0/destinations/4/records", json!({"blobs": "steps.extract.output.files"})))
            .judged_as("destination", "/nodes/0/destinations/4"),
        case("a failure policy outside hold, fail and continue", "destinations-every-kind", Edit::Set("/nodes/0/destinations/0/onFailure", json!("retry")))
            .judged_as("destination", "/nodes/0/destinations/0"),
        case("destinations of the wrong type on an activity", "destinations-every-kind", Edit::Set(DESTINATIONS, json!("archive")))
            .judged_as("destinations", DESTINATIONS),
        case("destinations of the wrong shape on a database node", "destinations-on-database", Edit::Set(DESTINATIONS, json!([{"name": "copy"}])))
            .judged_as("destinations", DESTINATIONS),
    ]
}

#[test]
fn every_valid_sample_is_accepted_by_the_canonical_schema() {
    let documents = valid_documents();
    assert!(
        documents.len() >= 8,
        "the samples cover every mode, scope, record source and destination kind"
    );
    let refused: Vec<String> = documents
        .iter()
        .filter_map(|(name, document)| {
            refusal(canonical(), document).map(|reason| format!("{name}: {reason}"))
        })
        .collect();
    assert!(
        refused.is_empty(),
        "the canonical schema refuses valid samples:\n{}",
        refused.join("\n")
    );
}

#[test]
fn every_refusal_is_a_sample_that_is_accepted_plus_one_edit() {
    let cases = refusals();
    assert!(
        cases.len() >= 14,
        "at least the 14 cases checked at planning"
    );
    let names: BTreeSet<&str> = cases.iter().map(|c| c.name).collect();
    assert_eq!(names.len(), cases.len(), "every case has its own name");

    let mut problems = Vec::new();
    for case in &cases {
        let base = valid(case.base);
        if let Some(reason) = refusal(canonical(), &base) {
            problems.push(format!(
                "{}: its base sample {} is refused: {reason}",
                case.name, case.base
            ));
            continue;
        }
        let mut edited = base;
        apply(&mut edited, &case.edit);
        if refusal(canonical(), &edited).is_none() {
            problems.push(format!("{}: accepted, but must be refused", case.name));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} cases do not hold:\n{}",
        problems.len(),
        cases.len(),
        problems.join("\n")
    );
}

/// The control of the two body cases: the same body shape with a plain activity is accepted, so the refusals above
/// are about the node and not about the body document built for them.
#[test]
fn a_body_with_a_plain_activity_is_accepted() {
    let mut document = valid("destinations-every-kind");
    apply(
        &mut document,
        &Edit::Set("/bodies", body_with(plain_body_activity())),
    );
    assert_eq!(refusal(canonical(), &document), None);
}

/// The fragment on its own: it accepts the nodes of the samples and refuses the edited ones. A refusal that only
/// the canonical schema fails to give is therefore a merge that has not happened (T035-008), not a mistake of the
/// sample.
#[test]
fn the_fragment_alone_accepts_the_sample_nodes_and_refuses_the_edited_ones() {
    let documents = valid_documents();
    let write = fragment_definition("icebergWrite");
    for node in nodes_of(&documents, "icebergWrite") {
        assert_eq!(refusal(&write, node), None, "icebergWrite node {node}");
    }
    let destinations = fragment_definition("destinations");
    for (name, document) in &documents {
        for node in document["nodes"].as_array().expect("nodes") {
            if let Some(declared) = node.get("destinations") {
                assert_eq!(
                    refusal(&destinations, declared),
                    None,
                    "destinations of {name}"
                );
            }
        }
    }

    let mut not_refused = Vec::new();
    for case in refusals() {
        let Some((definition, pointer)) = case.in_fragment else {
            continue;
        };
        let mut edited = valid(case.base);
        apply(&mut edited, &case.edit);
        let value = edited
            .pointer(pointer)
            .unwrap_or_else(|| panic!("{}: {pointer} is not in the edited sample", case.name));
        if refusal(&fragment_definition(definition), value).is_none() {
            not_refused.push(case.name);
        }
    }
    assert!(
        not_refused.is_empty(),
        "the fragment itself accepts: {not_refused:?}"
    );
}

/// The samples keep covering what the fragment declares: a mode, a scope, a record source or a destination kind
/// added to the fragment without a sample fails here.
#[test]
fn the_samples_cover_every_mode_scope_record_source_and_destination_kind() {
    let fragment = read_json(&fragments().join("workflow-lake.schema.json"));
    let defs = &fragment["$defs"];
    let names = |value: &Value| -> BTreeSet<String> {
        value
            .as_array()
            .expect("an enum")
            .iter()
            .map(|item| item.as_str().expect("a string").to_string())
            .collect()
    };

    let documents = valid_documents();
    let writes = nodes_of(&documents, "icebergWrite");
    let modes: BTreeSet<String> = writes
        .iter()
        .map(|node| node["mode"].as_str().expect("a mode").to_string())
        .collect();
    assert_eq!(
        modes,
        names(&defs["icebergWrite"]["properties"]["mode"]["enum"])
    );

    let scopes: BTreeSet<String> = writes
        .iter()
        .filter_map(|node| node["overwrite"]["scope"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        scopes,
        names(&defs["icebergWrite"]["properties"]["overwrite"]["properties"]["scope"]["enum"])
    );

    let sources: BTreeSet<String> = writes
        .iter()
        .flat_map(|node| {
            node["records"]
                .as_object()
                .expect("records")
                .keys()
                .cloned()
        })
        .collect();
    assert_eq!(
        sources,
        ["inline", "blobs", "databaseResult"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "the record sources a write step may read"
    );

    let kinds: BTreeSet<String> = documents
        .iter()
        .flat_map(|(_, document)| document["nodes"].as_array().expect("nodes").iter())
        .filter_map(|node| node["destinations"].as_array())
        .flatten()
        .map(|destination| destination["kind"].as_str().expect("a kind").to_string())
        .collect();
    assert_eq!(
        kinds,
        names(&defs["destination"]["properties"]["kind"]["enum"])
    );
}

/// The schema is the editor-time gate: it describes the nodes, and publication is the codec's decision. The codec builds
/// the append `icebergWrite` (T035-053) and refuses everything it cannot build as an unsupported capability, instead of
/// accepting the document and dropping the step. This is the pending state, written down per document (AGENTS.md rule
/// 11): the destinations (T035-093 and before), overwrite and upsert (T035-080, T035-085) and a record schema
/// (T035-054, which teaches the classification check to read it) each replace their line here with an acceptance test. That the codec accepts a document does not publish
/// it: the store refuses a definition that holds an Iceberg node until its admission exists (D035-085), which the
/// real-store scenario of `integration_database_connections` and the application test of the default show.
#[test]
fn the_codec_builds_the_append_step_and_refuses_the_rest_instead_of_dropping_it() {
    use sagat_application::definitions::{IrCodec, IrError};
    use sagat_infra_ir::JsonIrCodec;

    let found: std::collections::BTreeMap<String, String> = valid_documents()
        .into_iter()
        .map(|(name, document)| {
            let bytes = serde_json::to_vec(&document).expect("JSON");
            let outcome = match JsonIrCodec.parse(&bytes) {
                Ok(_) => "built".to_owned(),
                Err(IrError::Unsupported { kind, .. }) => kind,
                // The classification check refuses an `x-sagat-classification` annotation where it does not know to
                // read one, which is a record schema, before the step is built.
                Err(IrError::Classification(_)) => "classification".to_owned(),
                Err(other) => format!("unexpected: {other:?}"),
            };
            (name, outcome)
        })
        .collect();
    let expected: std::collections::BTreeMap<String, String> = [
        ("append-inline", "built"),
        ("append-blobs-mapped", "classification"),
        ("destinations-every-kind", "destinations"),
        ("destinations-on-database", "destinations"),
        ("overwrite-batch-partitions", "icebergWrite.overwrite"),
        ("overwrite-filter", "icebergWrite.overwrite"),
        ("overwrite-table-allow-empty", "icebergWrite.overwrite"),
        ("upsert-database-result", "icebergWrite.upsert"),
    ]
    .into_iter()
    .map(|(name, outcome)| (name.to_owned(), outcome.to_owned()))
    .collect();
    assert_eq!(
        found, expected,
        "what the codec builds and what it refuses, per valid sample"
    );
}
