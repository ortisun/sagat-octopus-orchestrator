//! `sagat diff` and `sagat publish`: make the namespace say what the manifest says.
//!
//! Git is the source of truth for the documents. The tool compares the digest of each document
//! (the canonical form, so whitespace and key order do not matter) with the digest of the active
//! version, shows what would change, and applies it through the public API: create the workflow or
//! replace its draft, then publish. A namespace that asks for approvals refuses a direct publish;
//! the tool then opens a publication request, or publishes the approved one, and exits `3` while
//! approvals are missing. Every request is retried safely: creation and publication carry a key
//! derived from the document, so a rerun after a lost answer is the same operation.

use std::fmt::Write as _;

use sagat_application::definitions::IrCodec;
use sagat_application::diff::{DiffFinding, DocumentDiff};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

use crate::args::Common;
use crate::client::CallError;
use crate::manifest::{Loaded, Manifest};
use crate::remote::{self, RemoteWorkflow};
use crate::{
    EXIT_BREAKING, EXIT_DIFFERENCES, EXIT_DONE, EXIT_FAILED, EXIT_PENDING, Env, Io, Session,
    session,
};

/// How a workflow of the manifest stands against the platform.
enum State {
    /// The platform has no workflow of this name.
    New,
    /// The active version is this document already.
    Unchanged { active: u32 },
    /// The document differs from the active version (or there is none).
    Changed {
        active: Option<u32>,
        /// A version published before that is exactly this document: publishing activates it again.
        republishes: Option<u32>,
    },
}

struct Item<'a> {
    loaded: &'a Loaded,
    digest: String,
    remote: Option<RemoteWorkflow>,
    state: State,
    diff: Option<DocumentDiff>,
}

fn short(digest: &str) -> &str {
    &digest[..digest.len().min(12)]
}

/// The manifest read, every document checked locally, and compared with the platform.
async fn plan<'a>(
    session: &Session,
    loaded: &'a [Loaded],
    docs: Option<&str>,
) -> Result<Vec<Item<'a>>, String> {
    let mut checked = Vec::new();
    let mut refused = Vec::new();
    for item in loaded {
        match JsonIrCodec.parse(&item.document) {
            Ok(parsed) => checked.push((item, parsed.digest.to_hex())),
            Err(_) => {
                refused.push(format!("{}:", item.entry.name));
                refused.extend(
                    crate::diagnostics::render(
                        &crate::diagnostics::of_document(&item.document),
                        docs,
                    )
                    .into_iter()
                    .map(|line| format!("    {line}")),
                );
            }
        }
    }
    if !refused.is_empty() {
        return Err(format!(
            "documents that cannot be published, nothing was sent:\n  {}",
            refused.join("\n  ")
        ));
    }
    let mut known = remote::workflows(&session.client).await?;
    let mut items = Vec::new();
    for (loaded, digest) in checked {
        let existing = known.remove(&loaded.entry.name);
        let Some(workflow) = existing else {
            items.push(Item {
                loaded,
                digest,
                remote: None,
                state: State::New,
                diff: None,
            });
            continue;
        };
        let (state, diff) = match &workflow.active {
            Some(active) if active.digest == digest => (
                State::Unchanged {
                    active: active.number,
                },
                None,
            ),
            active => {
                let diff = match active {
                    Some(active) => {
                        let document =
                            remote::version(&session.client, &workflow.id, active.number).await?;
                        let from = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
                        Some(
                            JsonIrCodec
                                .diff(&from, &loaded.document)
                                .map_err(|e| format!("{}: {e}", loaded.entry.name))?,
                        )
                    }
                    None => None,
                };
                let republishes = remote::versions(&session.client, &workflow.id)
                    .await?
                    .into_iter()
                    .find(|(_, d)| *d == digest)
                    .map(|(n, _)| n);
                (
                    State::Changed {
                        active: active.as_ref().map(|a| a.number),
                        republishes,
                    },
                    diff,
                )
            }
        };
        items.push(Item {
            loaded,
            digest,
            remote: Some(workflow),
            state,
            diff,
        });
    }
    Ok(items)
}

fn show(io: &mut Io<'_>, items: &[Item<'_>]) {
    for item in items {
        let name = &item.loaded.entry.name;
        let digest = short(&item.digest);
        match &item.state {
            State::New => io.say(format!(
                "+ {name}  new workflow, to be created and published ({digest})"
            )),
            State::Unchanged { active } => io.say(format!("= {name}  v{active} is this document")),
            State::Changed {
                active,
                republishes,
            } => {
                let from =
                    active.map_or_else(|| "no active version".to_owned(), |a| format!("v{a}"));
                let mut line = format!("~ {name}  {from} -> {digest}");
                if let Some(n) = republishes {
                    let _ = write!(
                        line,
                        "  (this is the document of v{n}, published before: publishing activates it again)"
                    );
                }
                if let Some(diff) = &item.diff {
                    let _ = write!(
                        line,
                        "  nodes +{} -{} ~{}, {} breaking, {} to prepare",
                        diff.nodes_added.len(),
                        diff.nodes_removed.len(),
                        diff.nodes_changed.len(),
                        diff.breaking.len(),
                        diff.attention.len()
                    );
                }
                io.say(line);
            }
        }
        if let Some(diff) = &item.diff {
            for finding in &diff.breaking {
                io.say(format!(
                    "    breaking  {}: {}",
                    finding.code, finding.message
                ));
            }
            for finding in &diff.attention {
                io.say(attention_line(finding));
            }
        }
    }
}

fn attention_line(finding: &DiffFinding) -> String {
    format!("    prepare   {}: {}", finding.code, finding.message)
}

fn changes(items: &[Item<'_>]) -> usize {
    items
        .iter()
        .filter(|i| !matches!(i.state, State::Unchanged { .. }))
        .count()
}

fn breaking<'a>(items: &'a [Item<'_>]) -> Vec<&'a str> {
    items
        .iter()
        .filter(|i| i.diff.as_ref().is_some_and(|d| !d.breaking.is_empty()))
        .map(|i| i.loaded.entry.name.as_str())
        .collect()
}

/// The manifest and the namespace it names (the flag wins).
fn read(
    common: &Common,
    env: &Env,
    io: &mut Io<'_>,
) -> Result<Option<(Manifest, Vec<Loaded>)>, String> {
    let path = crate::manifest_path(common, env);
    match crate::manifest::load(&path) {
        Ok(found) => Ok(Some(found)),
        Err(problems) => {
            for problem in problems {
                io.warn(format!("  {problem}"));
            }
            io.warn(format!("{} is not valid", path.display()));
            Ok(None)
        }
    }
}

pub(crate) async fn diff(
    common: &Common,
    env: &Env,
    check: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let Some((manifest, loaded)) = read(common, env, io)? else {
        return Ok(EXIT_FAILED);
    };
    let namespace = common.namespace.clone().unwrap_or(manifest.namespace);
    let session = session(common, env, &namespace)?;
    let items = plan(&session, &loaded, io.docs()).await?;
    show(io, &items);
    let pending = changes(&items);
    io.say(format!(
        "{pending} of {} workflows would change in {namespace}",
        items.len()
    ));
    Ok(if check && pending > 0 {
        EXIT_DIFFERENCES
    } else {
        EXIT_DONE
    })
}

pub(crate) async fn publish(
    common: &Common,
    env: &Env,
    dry_run: bool,
    accept_breaking: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let Some((manifest, loaded)) = read(common, env, io)? else {
        return Ok(EXIT_FAILED);
    };
    let namespace = common.namespace.clone().unwrap_or(manifest.namespace);
    let session = session(common, env, &namespace)?;
    publish_documents(&session, &loaded, &namespace, dry_run, accept_breaking, io).await
}

/// Makes the namespace say what these documents say: the plan, the gate for what breaks callers,
/// then each workflow created or replaced and published.
pub(crate) async fn publish_documents(
    session: &Session,
    loaded: &[Loaded],
    namespace: &str,
    dry_run: bool,
    accept_breaking: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let items = plan(session, loaded, io.docs()).await?;
    show(io, &items);
    if dry_run {
        io.say(format!(
            "dry run: {} of {} workflows would change in {namespace}, nothing was sent",
            changes(&items),
            items.len()
        ));
        return Ok(EXIT_DONE);
    }
    let breaking = breaking(&items);
    if !breaking.is_empty() && !accept_breaking {
        io.warn(format!(
            "refused, nothing was sent: {} would break callers or emitters of the workflow. \
             Read the lines above; if that is intended, run again with --accept-breaking. \
             (This check is the tool's: the platform does not refuse a publication for it.)",
            breaking.join(", ")
        ));
        return Ok(EXIT_BREAKING);
    }
    let (mut published, mut pending) = (0usize, 0usize);
    for item in &items {
        if matches!(item.state, State::Unchanged { .. }) {
            continue;
        }
        match apply(session, item, io).await? {
            Applied::Published => published += 1,
            Applied::Pending => pending += 1,
        }
    }
    io.say(format!(
        "{published} published, {pending} waiting for approval, {} unchanged",
        items.len() - published - pending
    ));
    Ok(if pending > 0 { EXIT_PENDING } else { EXIT_DONE })
}

enum Applied {
    Published,
    Pending,
}

/// The deterministic part of a key: the same operation on the same document is the same key.
fn key(kind: &str, workflow: &str, revision: u64, digest: &str) -> String {
    format!(
        "cli-{kind}-{workflow}-r{revision}-{}",
        &digest[..digest.len().min(16)]
    )
}

async fn apply(session: &Session, item: &Item<'_>, io: &mut Io<'_>) -> Result<Applied, String> {
    let name = &item.loaded.entry.name;
    let client = &session.client;
    let (id, revision) = match &item.remote {
        None => {
            let body = json!({ "name": name, "ir": item.loaded.value });
            let (_, answer) = client
                .call(
                    "POST",
                    "/workflows",
                    Some(&body),
                    Some(&key("create", name, 0, &item.digest)),
                )
                .await
                .map_err(|e| format!("{name}: {e}"))?;
            io.say(format!("created   {name}"));
            identity(&answer, name)?
        }
        Some(remote) if remote.ir != item.loaded.value => {
            let body = json!({ "draftRevision": remote.revision, "ir": item.loaded.value });
            let (_, answer) = client
                .call(
                    "PUT",
                    &format!("/workflows/{}", remote.id),
                    Some(&body),
                    None,
                )
                .await
                .map_err(|e| format!("{name}: {e}"))?;
            io.say(format!("draft     {name} replaced by the manifest"));
            identity(&answer, name).map(|(_, revision)| (remote.id.clone(), revision))?
        }
        Some(remote) => (remote.id.clone(), remote.revision),
    };
    let mut body = json!({ "draftRevision": revision });
    if let Some(notes) = &item.loaded.entry.release_notes {
        body["releaseNotes"] = Value::String(notes.clone());
    }
    // A publication that creates a version is the same operation however often it is sent, and is
    // keyed by the document so that a run after a lost answer finds its own answer. One that
    // re-activates a version that was published before is a different thing each time it is
    // wanted (the pointer may have been moved since), and a key that has been used is answered as
    // it was, whatever the pointer says now: it gets a key of its own.
    let publish_key = match item.state {
        State::Changed {
            republishes: Some(_),
            ..
        } => crate::versions::nonce("publish", &[&id, &item.digest]),
        _ => key("publish", &id, revision, &item.digest),
    };
    match client
        .call(
            "POST",
            &format!("/workflows/{id}/publish"),
            Some(&body),
            Some(&publish_key),
        )
        .await
    {
        Ok((_, version)) => {
            announce(io, name, &version);
            Ok(Applied::Published)
        }
        Err(CallError::Refused(failure)) if failure.is("APPROVAL_REQUIRED") => {
            through_approval(session, item, &id, revision, io).await
        }
        Err(e) => Err(format!("{name}: {e}")),
    }
}

/// The id and the draft revision in the answer to a create or a replace.
fn identity(answer: &Value, name: &str) -> Result<(String, u64), String> {
    let id = answer.get("id").and_then(Value::as_str);
    let revision = answer.get("draftRevision").and_then(Value::as_u64);
    match (id, revision) {
        (Some(id), Some(revision)) => Ok((id.to_owned(), revision)),
        _ => Err(format!("{name}: the platform's answer is not a workflow")),
    }
}

fn announce(io: &mut Io<'_>, name: &str, version: &Value) {
    let number = version
        .get("versionNumber")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let created = version
        .get("created")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    io.say(if created {
        format!("published  {name} v{number}")
    } else {
        format!("activated  {name} v{number} (a version with this document existed)")
    });
    for warning in version
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(text) = warning.as_str() {
            io.say(format!("    warning: {text}"));
        }
    }
}

/// The namespace asks for approvals: publish the request that has them, wait for the one that does
/// not, or open one.
async fn through_approval(
    session: &Session,
    item: &Item<'_>,
    id: &str,
    revision: u64,
    io: &mut Io<'_>,
) -> Result<Applied, String> {
    let name = &item.loaded.entry.name;
    let client = &session.client;
    let requests =
        remote::first_page(client, &format!("/workflows/{id}/publication-requests")).await?;
    let matching = requests.iter().find(|r| {
        matches!(r["state"].as_str(), Some("PENDING" | "APPROVED"))
            && r["digest"].as_str() == Some(item.digest.as_str())
            && r["draftRevision"].as_u64() == Some(revision)
    });
    let request = match matching {
        Some(request) => request.clone(),
        None => {
            let mut body = json!({ "draftRevision": revision });
            if let Some(notes) = &item.loaded.entry.release_notes {
                body["releaseNotes"] = Value::String(notes.clone());
            }
            let (_, opened) = client
                .call(
                    "POST",
                    &format!("/workflows/{id}/publication-requests"),
                    Some(&body),
                    None,
                )
                .await
                .map_err(|e| format!("{name}: {e}"))?;
            opened
        }
    };
    let request_id = request["id"].as_str().unwrap_or_default();
    if request["state"].as_str() == Some("APPROVED") {
        let publish_key = format!("cli-request-{request_id}");
        match client
            .call(
                "POST",
                &format!("/publication-requests/{request_id}/publish"),
                None,
                Some(&publish_key),
            )
            .await
        {
            Ok((_, version)) => {
                announce(io, name, &version);
                return Ok(Applied::Published);
            }
            Err(CallError::Refused(failure)) if failure.is("NOT_ENOUGH_APPROVALS") => {
                io.say(format!(
                    "waiting    {name}: the namespace now asks for more approvals than request {request_id} has"
                ));
                return Ok(Applied::Pending);
            }
            Err(e) => return Err(format!("{name}: {e}")),
        }
    }
    io.say(format!(
        "waiting    {name}: request {request_id} has {} of {} approvals",
        request["approvals"].as_u64().unwrap_or(0),
        request["requiredApprovals"].as_u64().unwrap_or(0)
    ));
    Ok(Applied::Pending)
}

#[cfg(test)]
mod tests {
    use sagat_application::diff::DiffFinding;

    use super::attention_line;

    #[test]
    fn classification_changes_are_shown_as_prepare_findings() {
        for code in ["CLASSIFICATION_RELAXED", "CLASSIFICATION_TIGHTENED"] {
            let line = attention_line(&DiffFinding {
                code,
                message: "classification changed at /customer".to_owned(),
                node: None,
                path: Some("/customer".to_owned()),
            });
            assert_eq!(
                line,
                format!("    prepare   {code}: classification changed at /customer")
            );
        }
    }
}
