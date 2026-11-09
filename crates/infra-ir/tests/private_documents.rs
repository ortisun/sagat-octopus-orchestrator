//! Private documents (ADR-0080, D035-150 slice S1b): a node that only the platform may write is parsed by a private twin of the codec's
//! parse entry points and by nothing public. The public schema never learns `icebergProbe`, so no author can publish one and no SDK or
//! reference page lists it; the private entry points validate against that same schema plus the private node types and nothing else.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::{
    definitions::{IrCodec, IrError, ParsedDefinition},
    execution::DefinitionCache,
    runs::lake_probe_document,
};
use sagat_domain::{
    database::{ConnectionRef, ManagedKind},
    definition::WorkflowVersionId,
    identity::Slug,
};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn probe_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "database-probe"}, "entrypoint": "probe",
        "nodes": [{
            "id": "probe", "type": "icebergProbe",
            "connectionRef": {"name": "lake-main", "revision": 3}, "timeoutMs": 25000
        }]
    })
}

fn bytes(document: &Value) -> Vec<u8> {
    document.to_string().into_bytes()
}

fn ordinary_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "ordinary"}, "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "line.work", "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

#[test]
fn no_public_entry_point_accepts_a_private_node() {
    let document = bytes(&probe_document());
    for (name, outcome) in [
        ("parse", JsonIrCodec.parse(&document).map(|_| ())),
        (
            "parse_stored",
            JsonIrCodec.parse_stored(&document).map(|_| ()),
        ),
        (
            "parse_structured",
            JsonIrCodec.parse_structured(&document).map(|_| ()),
        ),
        (
            "parse_declared_state",
            JsonIrCodec.parse_declared_state(&document).map(|_| ()),
        ),
        (
            "parse_continuation",
            JsonIrCodec.parse_continuation(&document).map(|_| ()),
        ),
    ] {
        let error = outcome.expect_err(name);
        assert!(
            matches!(&error, IrError::Schema(issues)
                if issues.iter().any(|issue| issue.message.contains("icebergProbe"))),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn the_private_entry_points_accept_it_and_give_the_domain_a_probe() {
    let document = bytes(&probe_document());
    for (name, parsed) in [
        ("parse_private", JsonIrCodec.parse_private(&document)),
        (
            "parse_stored_private",
            JsonIrCodec.parse_stored_private(&document),
        ),
    ] {
        let parsed = parsed.unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let node = parsed
            .definition
            .nodes()
            .next()
            .expect("the one node of the document");
        assert_eq!(node.managed_kind(), Some(ManagedKind::LakeProbe), "{name}");
        let target = node.lake_probe_target().expect("the connection tested");
        assert_eq!(target.connection().name.as_str(), "lake-main", "{name}");
        assert_eq!(target.connection().revision, Some(3), "{name}");
        assert!(!parsed.canonical.is_empty(), "{name}");
    }
    // The same document, parsed twice, is the same bytes: the start path compares it with what it was handed.
    let first = JsonIrCodec.parse_private(&document).expect("parsed");
    let second = JsonIrCodec.parse_private(&document).expect("parsed");
    assert_eq!(first.canonical, second.canonical);
    assert_eq!(first.digest, second.digest);
}

#[test]
fn a_private_document_still_obeys_every_other_rule_of_the_schema() {
    let mut cases: Vec<(&str, Value)> = Vec::new();
    let mut with = |name: &'static str, edit: &dyn Fn(&mut Value)| {
        let mut document = probe_document();
        edit(&mut document["nodes"][0]);
        cases.push((name, document));
    };
    with("a member the node does not have", &|node| {
        node["statement"] = json!("SELECT 1");
    });
    with("no connection", &|node| {
        node.as_object_mut().expect("node").remove("connectionRef");
    });
    with("a revision of zero", &|node| {
        node["connectionRef"]["revision"] = json!(0);
    });
    with("a connection name that is not a name", &|node| {
        node["connectionRef"]["name"] = json!("Not A Name");
    });
    with("a member the connection reference does not have", &|node| {
        node["connectionRef"]["host"] = json!("catalog.example");
    });
    with("a timeout of zero", &|node| {
        node["timeoutMs"] = json!(0);
    });
    for (name, document) in cases {
        let error = JsonIrCodec
            .parse_private(&bytes(&document))
            .expect_err(name);
        assert!(matches!(error, IrError::Schema(_)), "{name}: {error:?}");
    }
    // Only a root node: the private node types are not in the grammar of a body.
    let mut in_a_body = ordinary_document();
    in_a_body["bodies"] = json!({"line": {"entrypoint": "probe", "nodes": [
        {"id": "probe", "type": "icebergProbe", "connectionRef": {"name": "lake-main"}}
    ]}});
    assert!(JsonIrCodec.parse_private(&bytes(&in_a_body)).is_err());
}

#[test]
fn a_private_entry_point_parses_an_ordinary_document_exactly_as_the_public_one_does() {
    // The private twin widens the grammar by one node type and changes nothing else: same canonical bytes, same digest, same warnings.
    let document = bytes(&ordinary_document());
    let public = JsonIrCodec.parse(&document).expect("public");
    let private = JsonIrCodec.parse_private(&document).expect("private");
    assert_eq!(public.canonical, private.canonical);
    assert_eq!(public.digest, private.digest);
    assert_eq!(public.warnings, private.warnings);
    let stored = JsonIrCodec.parse_stored(&document).expect("stored public");
    let stored_private = JsonIrCodec
        .parse_stored_private(&document)
        .expect("stored private");
    assert_eq!(stored.canonical, stored_private.canonical);
    // and what is wrong with it is wrong in both
    let mut broken = ordinary_document();
    broken["nodes"][0]["type"] = json!("nonsense");
    let broken = bytes(&broken);
    assert!(JsonIrCodec.parse(&broken).is_err());
    assert!(JsonIrCodec.parse_private(&broken).is_err());
}

#[test]
fn an_author_is_never_told_of_a_private_node_type_by_an_error_about_an_unknown_one() {
    let mut document = ordinary_document();
    document["nodes"][0]["type"] = json!("nonsense");
    let error = JsonIrCodec
        .parse(&bytes(&document))
        .expect_err("a node of an unknown type");
    assert!(
        !format!("{error:?}").contains("icebergProbe"),
        "the list of the types an author may use is the public one: {error:?}"
    );
    // and the document that names the private type is refused by the public entry as any unknown type is
    let error = JsonIrCodec
        .parse(&bytes(&probe_document()))
        .expect_err("a private node");
    assert!(
        matches!(&error, IrError::Schema(issues)
            if issues.iter().any(|issue| issue.path == "/nodes/0/type")),
        "{error:?}"
    );
}

// ---- The cache and the platform's own document (slice S1c) --------------------------------------------------------------------

/// A codec that has no private grammar: it implements what the trait requires and nothing else, so the private entries are the
/// trait's defaults. Private documents fail closed.
struct PublicOnly;

impl IrCodec for PublicOnly {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        JsonIrCodec.check_draft(document)
    }
    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        JsonIrCodec.parse(document)
    }
    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        JsonIrCodec.diff(from, to)
    }
}

fn version() -> WorkflowVersionId {
    WorkflowVersionId::generate()
}

#[test]
fn a_codec_with_no_private_grammar_refuses_a_private_document_at_both_entries() {
    let document = bytes(&probe_document());
    assert!(matches!(
        PublicOnly.parse_private(&document),
        Err(IrError::Schema(_))
    ));
    assert!(matches!(
        PublicOnly.parse_stored_private(&document),
        Err(IrError::Schema(_))
    ));
    // and an ordinary document is not parsed by it as private either: the private entry is not a second way in
    assert!(
        PublicOnly
            .parse_private(&bytes(&ordinary_document()))
            .is_err()
    );
    assert!(
        PublicOnly
            .parse_stored_private(&bytes(&ordinary_document()))
            .is_err(),
        "the stored private entry of such a codec is not the ordinary stored one under another name"
    );
}

#[test]
fn the_cache_restores_a_private_version_through_the_private_entry_and_a_public_one_never_does() {
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 8);
    let document = bytes(&probe_document());
    let private = version();
    let restored = cache
        .get_or_parse_private(private, &document)
        .expect("a private version restores");
    assert_eq!(
        restored.nodes().next().and_then(|node| node.managed_kind()),
        Some(ManagedKind::LakeProbe)
    );
    // the version is cached: the next read, by either entry, is the same definition
    assert!(Arc::ptr_eq(&restored, &cache.get(private).expect("cached")));
    // a version that is not private never takes the private entry, whatever its document says
    let public = version();
    assert!(
        cache.get_or_parse(public, &document).is_err(),
        "the public restore refuses a private node"
    );
    assert!(cache.get(public).is_none(), "and keeps nothing of it");
}

#[test]
fn a_cache_over_a_codec_without_private_documents_restores_none() {
    let cache = DefinitionCache::new(Arc::new(PublicOnly), 8);
    assert!(
        cache
            .get_or_parse_private(version(), &bytes(&probe_document()))
            .is_err()
    );
    // an ordinary version still restores through the ordinary entry
    assert!(
        cache
            .get_or_parse(version(), &bytes(&ordinary_document()))
            .is_ok()
    );
}

fn lake_connection(name: &str, revision: Option<u32>) -> ConnectionRef {
    ConnectionRef::new(Slug::parse(name).expect("slug"), revision).expect("reference")
}

#[test]
fn the_fixed_probe_document_of_the_platform_is_a_private_document_of_one_probe_node() {
    for (name, revision) in [
        ("lake-main", None),
        ("lake-main", Some(3)),
        ("abc", Some(1)),
    ] {
        let connection = lake_connection(name, revision);
        let document = lake_probe_document(&connection);
        let parsed = JsonIrCodec
            .parse_private(&document)
            .unwrap_or_else(|error| panic!("{name} {revision:?}: {error:?}"));
        let nodes: Vec<_> = parsed.definition.nodes().collect();
        assert_eq!(nodes.len(), 1, "a probe is one node");
        let target = nodes[0].lake_probe_target().expect("a probe node");
        assert_eq!(target.connection(), &connection);
        assert_eq!(
            nodes[0].timeout(),
            Some(std::time::Duration::from_secs(25)),
            "the attempt's budget is inside the run's 30 s, so a typed answer arrives before the deadline"
        );
        // the public entries refuse it: an author cannot pass it off as a published workflow
        assert!(JsonIrCodec.parse(&document).is_err());
        assert!(JsonIrCodec.parse_stored(&document).is_err());
        // it is a fixed document: the same connection gives the same bytes and the same digest
        let again = JsonIrCodec
            .parse_private(&lake_probe_document(&connection))
            .expect("parsed");
        assert_eq!(
            (parsed.canonical, parsed.digest),
            (again.canonical, again.digest)
        );
    }
    // another connection, or another revision, is another document
    let one = JsonIrCodec
        .parse_private(&lake_probe_document(&lake_connection("lake-main", None)))
        .expect("parsed");
    let other = JsonIrCodec
        .parse_private(&lake_probe_document(&lake_connection("lake-main", Some(2))))
        .expect("parsed");
    assert_ne!(one.digest, other.digest);
}

#[test]
fn the_declared_state_of_a_private_document_is_read_through_the_private_entry_only() {
    // Run creation reads the stored document of the version for its declared state, and a private version is read by the private entry
    // there as everywhere else. A probe declares none: the private entry says so, and the public one cannot read the document at all.
    let probe = bytes(&probe_document());
    assert!(matches!(
        JsonIrCodec.declared_state_publication_private(&probe),
        Ok(None)
    ));
    assert!(JsonIrCodec.declared_state_publication(&probe).is_err());
    // a codec with no private grammar refuses it, as it does at every private entry
    assert!(
        PublicOnly
            .declared_state_publication_private(&probe)
            .is_err()
    );
    // an ordinary document is read the same through both
    let ordinary = bytes(&ordinary_document());
    assert!(matches!(
        JsonIrCodec.declared_state_publication(&ordinary),
        Ok(None)
    ));
    assert!(matches!(
        JsonIrCodec.declared_state_publication_private(&ordinary),
        Ok(None)
    ));
    assert!(
        PublicOnly
            .declared_state_publication_private(&ordinary)
            .is_err(),
        "and not by a codec with no private grammar, for which the private entry is not the public one under another name"
    );
    // the cache offers the same two entries
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 8);
    assert!(matches!(
        cache.declared_state_publication_private(&probe),
        Ok(None)
    ));
    assert!(cache.declared_state_publication(&probe).is_err());
}
