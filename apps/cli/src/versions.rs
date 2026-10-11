//! `sagat promote`, `sagat rollback` and `sagat history`: moving a version between namespaces and
//! moving the active pointer, through the same operations anyone with the permission can call.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::args::Common;
use crate::client::CallError;
use crate::{
    EXIT_BREAKING, EXIT_DONE, EXIT_FAILED, EXIT_PENDING, Env, Io, namespace_of, remote, session,
};

pub(crate) struct Promote<'a> {
    pub workflow: &'a str,
    pub to: &'a str,
    pub reason: &'a str,
    pub version: Option<u32>,
    pub accept_breaking: bool,
}

pub(crate) struct Rollback<'a> {
    pub workflow: &'a str,
    pub reason: &'a str,
    pub to: Option<u32>,
    pub accept_breaking: bool,
}

/// An idempotency key for one run of the tool: it makes a retry of this very call safe, and a
/// later call (after the state moved) a new one.
fn nonce(kind: &str, parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    for part in parts {
        hash.update([0]);
        hash.update(part.as_bytes());
    }
    hash.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
            .to_le_bytes(),
    );
    hash.update(std::process::id().to_le_bytes());
    let hex: String = hash
        .finalize()
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("cli-{kind}-{hex}")
}

/// What the platform said about a change that breaks callers.
fn breaking_refusal(io: &mut Io<'_>, what: &str, failure: &crate::client::Failure) {
    let codes: Vec<&str> = failure.details["codes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    io.warn(format!(
        "refused: {what} breaks what callers and emitters rely on ({} findings: {}). \
         If that is intended, run again with --accept-breaking.",
        failure.details["findings"].as_u64().unwrap_or(0),
        codes.join(", ")
    ));
}

pub(crate) async fn promote(
    common: &Common,
    env: &Env,
    ask: &Promote<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let workflow = remote::workflow(client, ask.workflow).await?;
    let number = ask
        .version
        .or(workflow.active.as_ref().map(|a| a.number))
        .ok_or_else(|| {
            format!(
                "{} has no active version in {namespace}: say which one with --version",
                ask.workflow
            )
        })?;
    let target = remote::namespaces(client)
        .await?
        .into_iter()
        .find(|n| n.slug == ask.to)
        .ok_or_else(|| format!("the tenant has no namespace '{}'", ask.to))?;
    let body = json!({
        "namespaceId": target.id,
        "reason": ask.reason,
        "acceptBreaking": ask.accept_breaking,
    });
    let key = nonce("promote", &[&workflow.id, &number.to_string(), &target.id]);
    let answer = client
        .call(
            "POST",
            &format!("/workflows/{}/versions/{number}/promote", workflow.id),
            Some(&body),
            Some(&key),
        )
        .await;
    let (_, promotion) = match answer {
        Ok(ok) => ok,
        Err(CallError::Refused(failure)) if failure.is("BREAKING_CHANGE") => {
            breaking_refusal(io, "this promotion", &failure);
            return Ok(EXIT_BREAKING);
        }
        // A request is open for this workflow in the target: publish it if it has its approvals,
        // say so if it has not, so that running the same command again converges.
        Err(CallError::Refused(failure)) if failure.is("REQUEST_OPEN") => {
            let id = failure.details["requestId"].as_str().unwrap_or_default();
            return finish_request(client, id, ask.to, io).await;
        }
        Err(e) => return Err(e.to_string()),
    };
    let from = format!("{}/{}@v{number}", namespace, ask.workflow);
    match promotion["outcome"].as_str() {
        Some("PUBLISHED") => {
            io.say(format!(
                "promoted {from} to {}: published as v{}, now active",
                ask.to,
                promotion["version"]["versionNumber"].as_u64().unwrap_or(0)
            ));
            Ok(EXIT_DONE)
        }
        Some("REQUESTED") => {
            io.say(format!(
                "promotion of {from} to {} waits for approvals: request {} has {} of {}",
                ask.to,
                promotion["request"]["id"].as_str().unwrap_or("?"),
                promotion["request"]["approvals"].as_u64().unwrap_or(0),
                promotion["request"]["requiredApprovals"]
                    .as_u64()
                    .unwrap_or(0)
            ));
            Ok(EXIT_PENDING)
        }
        Some("ALREADY_ACTIVE") => {
            io.say(format!(
                "{} already runs this document (v{}): nothing changed",
                ask.to,
                promotion["activeVersion"].as_u64().unwrap_or(0)
            ));
            Ok(EXIT_DONE)
        }
        _ => Err("the platform's answer is not a promotion".to_owned()),
    }
}

pub(crate) async fn rollback(
    common: &Common,
    env: &Env,
    ask: &Rollback<'_>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let workflow = remote::workflow(client, ask.workflow).await?;
    let active = workflow
        .active
        .as_ref()
        .map(|a| a.number)
        .ok_or_else(|| format!("{} has no active version in {namespace}", ask.workflow))?;
    let target = match ask.to {
        Some(number) => number,
        None => {
            let history =
                remote::first_page(client, &format!("/workflows/{}/activations", workflow.id))
                    .await?;
            history
                .first()
                .and_then(|row| row["previousVersion"].as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    "there is no earlier active version to go back to: say which with --to"
                        .to_owned()
                })?
        }
    };
    let body = json!({
        "version": target,
        "reason": ask.reason,
        "expectedVersion": active,
        "acceptBreaking": ask.accept_breaking,
    });
    let answer = client
        .call(
            "PUT",
            &format!("/workflows/{}/active-version", workflow.id),
            Some(&body),
            None,
        )
        .await;
    let (_, result) = match answer {
        Ok(ok) => ok,
        Err(CallError::Refused(failure)) if failure.is("BREAKING_CHANGE") => {
            breaking_refusal(io, "this change", &failure);
            return Ok(EXIT_BREAKING);
        }
        Err(e) => return Err(e.to_string()),
    };
    if result["changed"].as_bool() == Some(false) {
        io.say(format!(
            "{} v{target} is the active version already: nothing changed",
            ask.workflow
        ));
    } else {
        io.say(format!(
            "{} now runs v{target} (was v{active}); runs in flight keep the version they started with",
            ask.workflow
        ));
        io.say(
            "Git still holds the document you came from: change it there too, or the next publish activates it again.",
        );
    }
    Ok(EXIT_DONE)
}

pub(crate) async fn history(
    common: &Common,
    env: &Env,
    workflow: &str,
    limit: u32,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let found = remote::workflow(client, workflow).await?;
    let (_, page) = client
        .call(
            "GET",
            &crate::client::with_query(
                &format!("/workflows/{}/activations", found.id),
                &[("limit", &limit.to_string())],
            ),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    let rows = page["items"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        io.say(format!(
            "{workflow} has never been activated in {namespace}"
        ));
        return Ok(EXIT_FAILED);
    }
    for row in &rows {
        let text = |key: &str| row[key].as_str().unwrap_or("-");
        let from = row["previousVersion"]
            .as_u64()
            .map_or_else(|| "-".to_owned(), |p| format!("v{p}"));
        let mut line = format!(
            "{}  {:<8} {from} -> v{}  by {}",
            text("at"),
            text("kind"),
            row["version"].as_u64().unwrap_or(0),
            text("actor"),
        );
        if let Some(request) = row["requestId"].as_str() {
            line.push_str(&format!("  request {request}"));
        }
        if let Some(origin) = row["promotedFrom"].as_object() {
            line.push_str(&format!(
                "  from {}@v{}",
                origin
                    .get("workflowId")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                origin.get("version").and_then(Value::as_u64).unwrap_or(0)
            ));
        }
        io.say(line);
        if let Some(reason) = row["reason"].as_str() {
            io.say(format!("    {reason}"));
        }
    }
    if page["nextCursor"].as_str().is_some() {
        io.say(format!(
            "(older changes exist: raise --limit above {limit})"
        ));
    }
    Ok(EXIT_DONE)
}

/// An open request of the target: published when it has its approvals, waited for otherwise.
async fn finish_request(
    client: &crate::client::Client,
    id: &str,
    target: &str,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let (_, request) = client
        .call("GET", &format!("/publication-requests/{id}"), None, None)
        .await
        .map_err(|e| e.to_string())?;
    if request["state"].as_str() == Some("APPROVED") {
        let key = format!("cli-request-{id}");
        let (_, version) = client
            .call(
                "POST",
                &format!("/publication-requests/{id}/publish"),
                None,
                Some(&key),
            )
            .await
            .map_err(|e| e.to_string())?;
        io.say(format!(
            "request {id} had its approvals: published in {target} as v{}, now active",
            version["versionNumber"].as_u64().unwrap_or(0)
        ));
        return Ok(EXIT_DONE);
    }
    io.say(format!(
        "request {id} is {}: {} of {} approvals so far",
        request["state"].as_str().unwrap_or("?").to_lowercase(),
        request["approvals"].as_u64().unwrap_or(0),
        request["requiredApprovals"].as_u64().unwrap_or(0)
    ));
    Ok(EXIT_PENDING)
}

pub(crate) async fn requests(
    common: &Common,
    env: &Env,
    workflow: &str,
    state: Option<&str>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let found = remote::workflow(client, workflow).await?;
    let mut query = Vec::new();
    if let Some(state) = state {
        query.push(("state", state));
    }
    let path = crate::client::with_query(
        &format!("/workflows/{}/publication-requests", found.id),
        &query,
    );
    let rows = remote::first_page(client, &path).await?;
    if rows.is_empty() {
        io.say(format!("{workflow} has no requests in {namespace}"));
        return Ok(EXIT_DONE);
    }
    for row in &rows {
        let text = |key: &str| row[key].as_str().unwrap_or("-");
        io.say(format!(
            "{}  {:<10} {:<7} revision {} ({})  {} of {} approvals  by {} at {}",
            text("id"),
            text("state"),
            text("kind"),
            row["draftRevision"].as_u64().unwrap_or(0),
            text("digest").get(..12).unwrap_or("-"),
            row["approvals"].as_u64().unwrap_or(0),
            row["requiredApprovals"].as_u64().unwrap_or(0),
            text("requestedBy"),
            text("requestedAt"),
        ));
        for decision in row["decisions"].as_array().into_iter().flatten() {
            io.say(format!(
                "    {} {} at {}: {}",
                decision["verdict"].as_str().unwrap_or("?").to_lowercase(),
                decision["by"].as_str().unwrap_or("?"),
                decision["at"].as_str().unwrap_or("?"),
                decision["reason"].as_str().unwrap_or("")
            ));
        }
    }
    Ok(EXIT_DONE)
}

pub(crate) async fn decide(
    common: &Common,
    env: &Env,
    request: &str,
    reason: &str,
    approve: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let verb = if approve { "approve" } else { "reject" };
    let (_, answer) = session
        .client
        .call(
            "POST",
            &format!("/publication-requests/{request}/{verb}"),
            Some(&json!({ "reason": reason })),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    io.say(format!(
        "{} request {request}: now {} with {} of {} approvals",
        if approve { "approved" } else { "rejected" },
        answer["state"].as_str().unwrap_or("?").to_lowercase(),
        answer["approvals"].as_u64().unwrap_or(0),
        answer["requiredApprovals"].as_u64().unwrap_or(0)
    ));
    Ok(EXIT_DONE)
}
